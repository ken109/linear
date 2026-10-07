//! Credentials: pure types and the rules for building an `Authorization` header.
//!
//! Reading and writing credential files is the CLI's job. A [`Secret`] never
//! prints its value through `Debug`, and there is no `Display`.
//!
//! Besides a personal API key and an OAuth token, a workspace can authenticate
//! as an **app** with the client credentials grant: the app's client id and
//! secret are exchanged for an access token. Building the token request and
//! reading its answer is pure and lives here ([`client_credentials_form`],
//! [`parse_token_response`]); sending it, and keeping the token, is the CLI's job.

use crate::error::Error;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A token. `Debug` is redacted; call [`Secret::expose`] to use the value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Only for placing into a request header or a credential file.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

pub use crate::config::AuthMethod;

/// A credential: stored in `credentials/<workspace>.json` (an API key or an
/// OAuth token), or read from the environment (the app's client credentials).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Credential {
    ApiKey {
        api_key: Secret,
    },
    Oauth {
        access_token: Secret,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<DateTime<Utc>>,
    },
    /// An app's client id and secret. Never written to a file: it is read from
    /// the environment on every run, so it cannot be (de)serialized.
    #[serde(skip)]
    ClientCredentials {
        client_id: String,
        client_secret: Secret,
    },
}

impl Credential {
    pub fn method(&self) -> AuthMethod {
        match self {
            Self::ApiKey { .. } => AuthMethod::ApiKey,
            Self::Oauth { .. } => AuthMethod::Oauth,
            Self::ClientCredentials { .. } => AuthMethod::ClientCredentials,
        }
    }

    /// The value for the `Authorization` header. Personal API keys are sent
    /// bare; OAuth tokens as a bearer token. The result contains the secret.
    ///
    /// Client credentials have no header of their own: they are exchanged for
    /// an [`AccessToken`] first, which sends as [`AccessToken::bearer`]. This
    /// returns an empty string for them, so the secret can never be sent by mistake.
    pub fn authorization(&self) -> String {
        match self {
            Self::ApiKey { api_key } => api_key.expose().to_owned(),
            Self::Oauth { access_token, .. } => format!("Bearer {}", access_token.expose()),
            Self::ClientCredentials { .. } => String::new(),
        }
    }

    /// Whether an OAuth access token has expired (API keys never do).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        matches!(self, Self::Oauth { expires_at: Some(t), .. } if *t <= now)
    }

    /// Whether an OAuth access token has expired or will within a minute, so it
    /// should be refreshed before it is used. A token without an expiry never needs it.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        matches!(self, Self::Oauth { expires_at: Some(t), .. } if *t <= now + EXPIRY_MARGIN)
    }
}

/// `NAME` for the environment: upper-cased, with non-alphanumerics as `_`.
fn env_suffix(workspace: &str) -> String {
    workspace
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// The environment variable that overrides the stored API key for a workspace:
/// `LINEAR_API_KEY_<NAME>`, upper-cased, with non-alphanumerics as `_`.
pub fn api_key_env_var(workspace: &str) -> String {
    format!("LINEAR_API_KEY_{}", env_suffix(workspace))
}

/// The client secret of every workspace that uses client credentials.
pub const CLIENT_SECRET_ENV: &str = "LINEAR_CLIENT_SECRET";
/// The client id, for every workspace (it can also be `client_id` in the workspace's config).
pub const CLIENT_ID_ENV: &str = "LINEAR_CLIENT_ID";

/// `LINEAR_CLIENT_SECRET_<NAME>`: the secret for one workspace, ahead of [`CLIENT_SECRET_ENV`].
pub fn client_secret_env_var(workspace: &str) -> String {
    format!("{CLIENT_SECRET_ENV}_{}", env_suffix(workspace))
}

/// `LINEAR_CLIENT_ID_<NAME>`: the client id for one workspace, ahead of [`CLIENT_ID_ENV`].
pub fn client_id_env_var(workspace: &str) -> String {
    format!("{CLIENT_ID_ENV}_{}", env_suffix(workspace))
}

// ------------------------------------------------------------ client credentials

/// Linear's token endpoint.
pub const TOKEN_URL: &str = "https://api.linear.app/oauth/token";

/// The scopes an app token is requested with.
///
/// `initiative:write` is needed by an app actor to link projects to
/// initiatives. Linear invalidates the app's existing tokens when a token is
/// requested with *different* scopes, so every user of the same app (a CI
/// script, a Worker) has to ask for the same ones.
pub const APP_SCOPE: &str = "read,write,initiative:write";

/// The body of the token request for the client credentials grant
/// (`application/x-www-form-urlencoded`). It contains the secret.
pub fn client_credentials_form(client_id: &str, client_secret: &Secret, scope: &str) -> Secret {
    let pairs = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret.expose()),
        ("scope", scope),
    ];
    Secret::new(
        pairs
            .iter()
            .map(|(k, v)| format!("{k}={}", form_encode(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

/// Percent-encode a form value (RFC 3986 unreserved characters stay as they are).
pub(crate) fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// An app's access token and when it stops working.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken {
    token: Secret,
    expires_at: DateTime<Utc>,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &self.token)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// How long before its expiry a token is replaced, so a request is never sent
/// with one that runs out on the way.
const EXPIRY_MARGIN: Duration = Duration::seconds(60);

/// The lifetime assumed when the answer does not say (Linear's is 30 days).
const DEFAULT_LIFETIME_SECS: i64 = 3600;

impl AccessToken {
    pub fn new(token: Secret, expires_at: DateTime<Utc>) -> Self {
        Self { token, expires_at }
    }

    /// The `Authorization` header value. Contains the token.
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.token.expose())
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Whether it is expired, or will be within a minute.
    pub fn needs_replacing(&self, now: DateTime<Utc>) -> bool {
        now + EXPIRY_MARGIN >= self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Read the answer of the token endpoint.
///
/// What goes into an error is the OAuth `error` and its description, never the
/// raw body, and never `client_secret` (a server that echoes what it was sent
/// has it masked): an error message is printed, a secret must not be.
pub fn parse_token_response(
    status: u16,
    body: &str,
    now: DateTime<Utc>,
    client_secret: &Secret,
) -> Result<AccessToken, Error> {
    let answer: Option<TokenAnswer> = serde_json::from_str(body).ok();

    if (200..300).contains(&status) {
        let token = answer
            .as_ref()
            .and_then(|a| a.access_token.as_deref())
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                Error::Auth("Linear answered the token request without an access token".into())
            })?;
        let lifetime = answer
            .as_ref()
            .and_then(|a| a.expires_in)
            .filter(|s| *s > 0)
            .unwrap_or(DEFAULT_LIFETIME_SECS);
        return Ok(AccessToken::new(
            Secret::new(token),
            now + Duration::seconds(lifetime),
        ));
    }

    let detail = answer
        .and_then(|a| match (a.error, a.error_description) {
            (Some(e), Some(d)) if !d.is_empty() => Some(format!("{e}: {d}")),
            (Some(e), _) => Some(e),
            (None, Some(d)) => Some(d),
            (None, None) => None,
        })
        .map(|d| redact(&d, client_secret));

    match status {
        400 | 401 | 403 => Err(Error::Auth(format!(
            "Linear refused the client credentials{}",
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

/// Mask the secret in `text` and keep it short.
pub(crate) fn redact(text: &str, secret: &Secret) -> String {
    let raw = secret.expose();
    let masked = if raw.is_empty() {
        text.to_owned()
    } else {
        text.replace(raw, "[redacted]")
            .replace(&form_encode(raw), "[redacted]")
    };
    masked.chars().take(200).collect()
}
