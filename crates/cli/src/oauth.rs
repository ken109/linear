//! The OAuth login (authorization code flow with PKCE): a browser, a one-shot server on
//! `localhost` for the callback, and the token requests. The pure half is `linear_core::oauth`.
//!
//! The access token lives for hours, so every run first checks it (`Ctx::credential`) and
//! trades the refresh token for a new one when it is about to run out (see [`refresh`]).

use crate::error::{CliError, Result};
use crate::http;
use linear_core::auth::{Credential, Secret};
use linear_core::oauth::{self, Callback};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The port of the callback server, ahead of the workspace's `oauth_port`.
pub const PORT_ENV: &str = "LINEAR_OAUTH_PORT";
/// How many seconds a login waits for the browser (default 300).
pub const TIMEOUT_ENV: &str = "LINEAR_OAUTH_TIMEOUT";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// The callback port: `--port`, else `LINEAR_OAUTH_PORT`, else the workspace's `oauth_port`,
/// else [`oauth::DEFAULT_PORT`].
pub fn port(flag: Option<u16>, configured: Option<u16>) -> Result<u16> {
    let from_env = match std::env::var(PORT_ENV) {
        Ok(v) if !v.trim().is_empty() => Some(
            v.trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| {
                    CliError::usage(format!(
                        "{PORT_ENV} must be a port number (1-65535), got {:?}",
                        v.trim()
                    ))
                })?,
        ),
        _ => None,
    };
    if flag == Some(0) {
        return Err(CliError::usage("--port must be a port number (1-65535)"));
    }
    Ok(flag
        .or(from_env)
        .or(configured)
        .unwrap_or(oauth::DEFAULT_PORT))
}

fn login_timeout() -> Result<Duration> {
    match std::env::var(TIMEOUT_ENV) {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .ok_or_else(|| {
                CliError::usage(format!(
                    "{TIMEOUT_ENV} must be a whole number of seconds, got {:?}",
                    v.trim()
                ))
            }),
        _ => Ok(DEFAULT_TIMEOUT),
    }
}

/// `n` random bytes as unpadded base64url.
fn random_token(n: usize) -> Result<String> {
    let mut bytes = vec![0u8; n];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| CliError::general(format!("cannot get random bytes: {e}")))?;
    Ok(oauth::base64url(&bytes))
}

/// Log in: print the URL (and open it in a browser unless `open_browser` is false),
/// wait for Linear to call back, and exchange the code for tokens.
pub fn login(client_id: &str, port: u16, open_browser: bool) -> Result<Credential> {
    let timeout = login_timeout()?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        CliError::general(format!(
            "cannot listen on 127.0.0.1:{port} for the login callback: {e}; choose another port with \
             --port (it must be registered as the redirect URI {} of the OAuth app)",
            oauth::redirect_uri(port)
        ))
    })?;
    listener.set_nonblocking(true)?;

    let state = random_token(16)?;
    let verifier = Secret::new(random_token(32)?);
    let redirect = oauth::redirect_uri(port);
    let url = oauth::authorization_url(
        client_id,
        &redirect,
        oauth::SCOPE,
        &state,
        &oauth::code_challenge(verifier.expose()),
    );

    // The user needs this whatever --quiet and --json say.
    eprintln!("Open this URL in a browser to log in to Linear:\n\n  {url}\n");
    if open_browser {
        open_in_browser(&url);
    }
    eprintln!("Waiting for the login to finish (redirect: {redirect}) ...");

    let code = wait_for_code(&listener, &state, timeout)?;
    let form = oauth::authorization_code_form(client_id, &code, &redirect, &verifier);
    let tokens = http::oauth_token(&form, &[&code, &verifier])?;
    Ok(tokens.into_credential(None))
}

/// Trade the refresh token of an OAuth credential for a new access token.
///
/// A refused request is an auth error: the refresh token is expired or revoked and the
/// login has to be done again. Anything else (no connection, a 5xx) is passed on as it is.
pub fn refresh(client_id: &str, credential: &Credential) -> Result<Credential> {
    let Credential::Oauth {
        refresh_token: Some(refresh_token),
        ..
    } = credential
    else {
        return Err(CliError::auth("the OAuth login has no refresh token"));
    };
    let form = oauth::refresh_form(client_id, refresh_token);
    let tokens = http::oauth_token(&form, &[refresh_token])?;
    Ok(tokens.into_credential(Some(refresh_token)))
}

fn open_in_browser(url: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return;
    };
    // Failing to open one (no desktop, SSH) is fine: the URL has been printed.
    let _ = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Serve the callback until it brings a code. Requests that are not Linear's answer to this
/// login (a favicon, another tab, a wrong state) are answered and skipped.
fn wait_for_code(listener: &TcpListener, state: &str, timeout: Duration) -> Result<Secret> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = serve(stream, state) {
                    return outcome;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(CliError::general(format!(
                        "timed out after {} s waiting for the login; run the command again \
                         ({TIMEOUT_ENV} changes the wait)",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(CliError::from(e)),
        }
    }
}

/// Answer one request. `Some` when it ended the login.
fn serve(mut stream: TcpStream, state: &str) -> Option<Result<Secret>> {
    // A connection that says nothing (a browser's speculative one) must not hold the login up.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let target = read_request_target(&mut stream)?;
    match oauth::parse_callback(&target, state) {
        Callback::Ignore => {
            respond(&mut stream, "404 Not Found", "Not found.");
            None
        }
        Callback::Rejected(why) => {
            respond(&mut stream, "400 Bad Request", &format!("Rejected: {why}."));
            None
        }
        Callback::Denied(why) => {
            respond(
                &mut stream,
                "200 OK",
                "The login was not completed. You can close this tab.",
            );
            Some(Err(CliError::auth(format!(
                "Linear did not log you in ({why})"
            ))))
        }
        Callback::Code(code) => {
            respond(
                &mut stream,
                "200 OK",
                "Logged in to Linear. You can close this tab and go back to the terminal.",
            );
            Some(Ok(code))
        }
    }
}

/// The target of a `GET` request (`/callback?code=...`), read from the request line.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    // The request line is enough; stop at its end, or after 8 KiB.
    while buf.len() < 8192 && !buf.contains(&b'\n') {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>linear</title><p>{}</p>",
        message.replace('&', "&amp;").replace('<', "&lt;")
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// The error when a login cannot be renewed and has to be done again.
pub fn expired(workspace: &str, why: &str) -> CliError {
    CliError::auth(format!(
        "the OAuth login for workspace {workspace:?} has expired ({why}); \
         run `linear workspace login {workspace}` again"
    ))
}
