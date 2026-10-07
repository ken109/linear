//! OAuth 2.0 authorization code flow with PKCE (RFC 7636), the pure half.
//!
//! The CLI opens the authorization URL in a browser, waits for Linear to redirect
//! to `http://localhost:<port>/callback`, and exchanges the code for tokens. What
//! this module does is everything that needs no I/O: the PKCE challenge, the
//! authorization URL, the token requests (as form bodies), the reading of the
//! callback and of the token answer. Randomness, the listener, the browser and the
//! HTTP calls are the CLI's job.
//!
//! A PKCE client has no client secret: the code verifier takes its place, so the
//! tokens it gets can be refreshed with the client id alone.

use crate::auth::{form_encode, redact, Credential, Secret};
use crate::error::Error;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Where the user approves the login.
pub const AUTHORIZE_URL: &str = "https://linear.app/oauth/authorize";

/// What a login asks for: everything a person can do, which `read,write` covers.
pub const SCOPE: &str = "read,write";

/// The callback port when nothing else is chosen. It has to match a redirect URI
/// registered for the OAuth app (see [`redirect_uri`]).
pub const DEFAULT_PORT: u16 = 4601;

/// The path of the redirect URI.
pub const CALLBACK_PATH: &str = "/callback";

/// The redirect URI to register for the OAuth app: `http://localhost:<port>/callback`.
pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

/// Unpadded URL-safe base64 (RFC 4648 section 5), the encoding PKCE uses.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        let chars = chunk.len() + 1; // 1 byte -> 2 chars, 2 -> 3, 3 -> 4
        for i in 0..chars {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 0x3f) as usize] as char);
        }
    }
    out
}

/// The `S256` code challenge of a verifier: `BASE64URL(SHA256(verifier))`.
pub fn code_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// The URL the user opens to approve the login.
pub fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    challenge: &str,
) -> String {
    let pairs = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", scope),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    let query: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", form_encode(v)))
        .collect();
    format!("{AUTHORIZE_URL}?{}", query.join("&"))
}

fn form(pairs: &[(&str, &str)]) -> Secret {
    Secret::new(
        pairs
            .iter()
            .map(|(k, v)| format!("{k}={}", form_encode(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

/// The token request that exchanges the callback's code for tokens
/// (`application/x-www-form-urlencoded`). It contains the code and the verifier.
pub fn authorization_code_form(
    client_id: &str,
    code: &Secret,
    redirect_uri: &str,
    verifier: &Secret,
) -> Secret {
    form(&[
        ("grant_type", "authorization_code"),
        ("code", code.expose()),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier.expose()),
    ])
}

/// The token request that trades a refresh token for a new access token.
pub fn refresh_form(client_id: &str, refresh_token: &Secret) -> Secret {
    form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose()),
        ("client_id", client_id),
    ])
}

/// What the token endpoint answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthTokens {
    pub access_token: Secret,
    /// Absent when Linear did not rotate it (or issued none).
    pub refresh_token: Option<Secret>,
    /// Absent when the answer gave no lifetime: the token is not known to expire.
    pub expires_at: Option<DateTime<Utc>>,
}

impl OauthTokens {
    /// The credential to store. A refresh that does not name a new refresh token
    /// keeps `previous`.
    pub fn into_credential(self, previous: Option<&Secret>) -> Credential {
        Credential::Oauth {
            access_token: self.access_token,
            refresh_token: self.refresh_token.or_else(|| previous.cloned()),
            expires_at: self.expires_at,
        }
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Read the answer of the token endpoint to an authorization code or refresh request.
///
/// What goes into an error is the OAuth `error` and its description, never the raw
/// body, and never any of `secrets` (a server that echoes what it was sent has it
/// masked): an error message is printed, a secret must not be. A refused request is
/// [`Error::Auth`], which for a refresh means the login has to be done again.
pub fn parse_token_response(
    status: u16,
    body: &str,
    now: DateTime<Utc>,
    secrets: &[&Secret],
) -> Result<OauthTokens, Error> {
    let answer: Option<TokenAnswer> = serde_json::from_str(body).ok();

    if (200..300).contains(&status) {
        let a = answer.as_ref();
        let access = a
            .and_then(|a| a.access_token.as_deref())
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                Error::Auth("Linear answered the token request without an access token".into())
            })?;
        return Ok(OauthTokens {
            access_token: Secret::new(access),
            refresh_token: a
                .and_then(|a| a.refresh_token.as_deref())
                .filter(|t| !t.trim().is_empty())
                .map(Secret::new),
            expires_at: a
                .and_then(|a| a.expires_in)
                .filter(|s| *s > 0)
                .map(|s| now + Duration::seconds(s)),
        });
    }

    let detail = answer
        .and_then(|a| match (a.error, a.error_description) {
            (Some(e), Some(d)) if !d.is_empty() => Some(format!("{e}: {d}")),
            (Some(e), _) => Some(e),
            (None, Some(d)) => Some(d),
            (None, None) => None,
        })
        .map(|d| secrets.iter().fold(d, |text, s| redact(&text, s)));

    match status {
        400 | 401 | 403 => Err(Error::Auth(format!(
            "Linear refused the OAuth request{}",
            detail.map(|d| format!(" ({d})")).unwrap_or_default()
        ))),
        429 => Err(Error::RateLimited {
            retry_after_secs: None,
        }),
        _ => Err(Error::Http {
            status,
            body: detail.unwrap_or_default(),
        }),
    }
}

/// What a request to the local callback server means.
#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    /// Not the callback (a favicon, a probe): answer 404 and keep waiting.
    Ignore,
    /// The callback, but not one that Linear sent for this login (wrong or missing
    /// `state`, no `code`): answer 400 and keep waiting.
    Rejected(String),
    /// The user said no, or Linear could not log them in: the login is over.
    Denied(String),
    /// The authorization code.
    Code(Secret),
}

/// Read the request target of a `GET` to the callback server (`/callback?code=...&state=...`).
/// `state` is the value this login sent: a callback that does not carry it is not trusted.
pub fn parse_callback(target: &str, state: &str) -> Callback {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Callback::Ignore;
    }
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| form_decode(v))
    };
    if param("state").as_deref() != Some(state) {
        return Callback::Rejected("the state does not match this login".into());
    }
    if let Some(error) = param("error") {
        let detail = param("error_description")
            .filter(|d| !d.is_empty())
            .map(|d| format!(": {d}"))
            .unwrap_or_default();
        let detail: String = format!("{error}{detail}").chars().take(200).collect();
        return Callback::Denied(detail);
    }
    match param("code").filter(|c| !c.is_empty()) {
        Some(code) => Callback::Code(Secret::new(code)),
        None => Callback::Rejected("there is no code".into()),
    }
}

/// Undo percent-encoding in a query value (`+` is a space). Bytes that do not form
/// UTF-8 become U+FFFD, and a stray `%` stays as it is.
fn form_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}
