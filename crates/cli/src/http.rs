//! The HTTP side: send a core `Request`, hand the response back to core.
//!
//! A read that fails for a transient reason (a timeout, a broken connection, a
//! 5xx, a short rate limit) is sent again a bounded number of times with a
//! growing wait. A write never is: it may have reached Linear even though its
//! response was lost, and sending it twice could create two issues. A rate limit
//! that asks for a long wait (the hourly request limit) is not waited out; the
//! command stops with the error, as before.

use crate::error::{CliError, Result};
use chrono::Utc;
use linear_core::auth::{
    client_credentials_form, parse_token_response, AccessToken, Credential, Secret, APP_SCOPE,
};
use linear_core::document::{operation_kinds, OperationKind};
use linear_core::retry::{Failure, RetryPolicy};
use linear_core::wire::{build_request, parse_response, Request, ResponseMeta};
use linear_core::Error;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

mod files;

/// Overrides the API endpoint (used by tests and proxies).
pub const API_URL_ENV: &str = "LINEAR_API_URL";
/// Overrides Linear's token endpoint (used by tests and proxies).
pub const TOKEN_URL_ENV: &str = "LINEAR_OAUTH_TOKEN_URL";
/// The time limit of one request, in seconds. `--timeout` overrides it.
pub const TIMEOUT_ENV: &str = "LINEAR_TIMEOUT";
/// How many times a failed read is sent again (default 2; 0 turns retrying off).
pub const RETRIES_ENV: &str = "LINEAR_RETRIES";
/// The wait before the first retry, in milliseconds (default 1000). Mostly for tests.
pub const RETRY_BASE_MS_ENV: &str = "LINEAR_RETRY_BASE_MS";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RETRIES: u64 = 10;

/// How requests are sent, for the whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub timeout: Duration,
    pub retry: RetryPolicy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            retry: RetryPolicy::default(),
        }
    }
}

impl Settings {
    /// `--timeout` (seconds), else the environment, else the defaults. A value
    /// that is not a number in range is a usage error.
    pub fn resolve(timeout_flag: Option<u64>) -> Result<Self> {
        let mut s = Self::default();
        if let Some(secs) = timeout_flag.or(env_u64(TIMEOUT_ENV)?) {
            if secs == 0 {
                return Err(CliError::usage(format!(
                    "the timeout must be at least 1 second (--timeout, or {TIMEOUT_ENV})"
                )));
            }
            s.timeout = Duration::from_secs(secs);
        }
        if let Some(n) = env_u64(RETRIES_ENV)? {
            if n > MAX_RETRIES {
                return Err(CliError::usage(format!(
                    "{RETRIES_ENV} must be at most {MAX_RETRIES}"
                )));
            }
            s.retry.max_retries = n as u32;
        }
        if let Some(ms) = env_u64(RETRY_BASE_MS_ENV)? {
            s.retry.base = Duration::from_millis(ms);
        }
        Ok(s)
    }
}

fn env_u64(name: &str) -> Result<Option<u64>> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v.trim().parse().map(Some).map_err(|_| {
            CliError::usage(format!("{name} must be a whole number, got {:?}", v.trim()))
        }),
        _ => Ok(None),
    }
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// Fix the settings for this run (the first call wins). Clients made before
/// this call, or without it, use the defaults.
pub fn configure(settings: Settings) {
    let _ = SETTINGS.set(settings);
}

pub struct Client {
    agent: ureq::Agent,
    url: String,
    token_url: String,
    credential: Credential,
    retry: RetryPolicy,
    /// The app's access token, for client credentials: fetched when first
    /// needed, kept for the run, and replaced when it is about to expire or
    /// Linear refuses it. Never written anywhere.
    token: Mutex<Option<AccessToken>>,
}

/// One try at a request.
enum Attempt {
    /// No response: a timeout, or the connection failed or broke off.
    Transport(CliError),
    /// Linear answered with an error.
    Linear(linear_core::Error),
    /// Failed in a way no retry helps.
    Failed(CliError),
}

impl Client {
    pub fn new(credential: Credential) -> Self {
        let settings = SETTINGS.get().copied().unwrap_or_default();
        let url = std::env::var(API_URL_ENV)
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| linear_core::API_URL.to_owned());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // Status handling belongs to core, which understands Linear's error bodies.
            .http_status_as_error(false)
            .timeout_global(Some(settings.timeout))
            .user_agent(concat!("linear-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let token_url = std::env::var(TOKEN_URL_ENV)
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| linear_core::auth::TOKEN_URL.to_owned());
        Self {
            agent,
            url,
            token_url,
            credential,
            retry: settings.retry,
            token: Mutex::new(None),
        }
    }

    fn is_app(&self) -> bool {
        matches!(self.credential, Credential::ClientCredentials { .. })
    }

    /// The `Authorization` header value. For client credentials this is the
    /// app's token, fetched first if there is none or it is about to expire.
    fn authorization(&self) -> Result<String> {
        let Credential::ClientCredentials {
            client_id,
            client_secret,
        } = &self.credential
        else {
            return Ok(self.credential.authorization());
        };
        let mut slot = self.token.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(token) = slot.as_ref().filter(|t| !t.needs_replacing(Utc::now())) {
            return Ok(token.bearer());
        }
        let token = self.fetch_token(client_id, client_secret)?;
        let bearer = token.bearer();
        *slot = Some(token);
        Ok(bearer)
    }

    /// Drop the token, so the next request fetches a new one.
    fn forget_token(&self) {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Exchange the client id and secret for a token. Asking for a token changes
    /// nothing, so it is retried like a read.
    fn fetch_token(&self, client_id: &str, client_secret: &Secret) -> Result<AccessToken> {
        let form = client_credentials_form(client_id, client_secret, APP_SCOPE);
        let mut retries = 0;
        loop {
            let (error, wait) = match self.post_token(&form, client_secret) {
                Ok(token) => return Ok(token),
                Err(Attempt::Failed(e)) => return Err(e),
                Err(Attempt::Transport(e)) => {
                    retries += 1;
                    (e, self.retry.wait(Failure::Transport, retries))
                }
                Err(Attempt::Linear(e)) => {
                    retries += 1;
                    let wait = self.retry.wait(Failure::Response(&e), retries);
                    (CliError::from(e), wait)
                }
            };
            match wait {
                Some(wait) => std::thread::sleep(wait),
                None => return Err(error),
            }
        }
    }

    fn post_token(
        &self,
        form: &Secret,
        client_secret: &Secret,
    ) -> std::result::Result<AccessToken, Attempt> {
        let mut response = self
            .agent
            .post(&self.token_url)
            .header("content-type", "application/x-www-form-urlencoded")
            .send(form.expose())
            .map_err(transport)?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().map_err(|e| {
            Attempt::Transport(CliError::general(format!(
                "could not read Linear's answer to the token request: {}",
                short(&e)
            )))
        })?;
        parse_token_response(status, &body, Utc::now(), client_secret).map_err(Attempt::Linear)
    }

    /// Run an operation and decode its data.
    pub fn execute<Q, V, T>(&self, op: &cynic::Operation<Q, V>) -> Result<T>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        self.execute_request(&build_request(op))
    }

    /// Send an already-built request and decode its data. A read that fails
    /// transiently is retried (see the module docs); anything else is not.
    pub fn execute_request<T: DeserializeOwned>(&self, request: &Request) -> Result<T> {
        let policy = if is_read(request) {
            self.retry
        } else {
            RetryPolicy::never()
        };
        // The number of the retry that would follow the attempt now being made.
        let mut next_retry = 0;
        let mut replaced_token = false;
        loop {
            let (error, wait) = match self.send(request) {
                Ok(data) => return Ok(data),
                Err(Attempt::Failed(e)) => return Err(e),
                // The app's token was refused (revoked, or expired early): get a
                // new one and send the request again, once. A refusal comes before
                // anything runs, so this is safe for a write as well.
                Err(Attempt::Linear(Error::Auth(_))) if self.is_app() && !replaced_token => {
                    replaced_token = true;
                    self.forget_token();
                    continue;
                }
                Err(Attempt::Transport(e)) => {
                    next_retry += 1;
                    (e, policy.wait(Failure::Transport, next_retry))
                }
                Err(Attempt::Linear(e)) => {
                    next_retry += 1;
                    let wait = policy.wait(Failure::Response(&e), next_retry);
                    (CliError::from(e), wait)
                }
            };
            match wait {
                Some(wait) => std::thread::sleep(wait),
                None => return Err(error),
            }
        }
    }

    fn send<T: DeserializeOwned>(&self, request: &Request) -> std::result::Result<T, Attempt> {
        let authorization = self.authorization().map_err(Attempt::Failed)?;
        let mut response = self
            .agent
            .post(&self.url)
            .header("content-type", "application/json")
            .header("authorization", authorization)
            .send(request.to_json())
            .map_err(transport)?;

        let header_u64 = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        let meta = ResponseMeta {
            status: response.status().as_u16(),
            retry_after_secs: header_u64("retry-after"),
            rate_limit_reset_ms: header_u64("x-ratelimit-requests-reset").map(|v| v as i64),
        };
        let body = response.body_mut().read_to_string().map_err(|e| match e {
            ureq::Error::Timeout(_) | ureq::Error::Io(_) => Attempt::Transport(CliError::general(
                format!("could not read Linear's response: {}", short(&e)),
            )),
            e => Attempt::Failed(CliError::general(format!(
                "could not read Linear's response: {}",
                short(&e)
            ))),
        })?;

        parse_response(&meta, &body, Utc::now()).map_err(Attempt::Linear)
    }
}

/// A request that cannot change anything: every operation in it is a query.
/// A document that cannot be read counts as a write, so it is never retried.
fn is_read(request: &Request) -> bool {
    operation_kinds(&request.query)
        .is_ok_and(|kinds| kinds.iter().all(|k| *k == OperationKind::Query))
}

fn transport(e: ureq::Error) -> Attempt {
    // Do not include the underlying error's text beyond a short summary: it can echo request details.
    let hint = if matches!(e, ureq::Error::Timeout(_)) {
        format!(
            " (raise the limit with --timeout <secs> or {TIMEOUT_ENV}; the default is {} s)",
            DEFAULT_TIMEOUT.as_secs()
        )
    } else {
        String::new()
    };
    let error = CliError::general(format!("request to Linear failed: {}{hint}", short(&e)));
    match e {
        ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            Attempt::Transport(error)
        }
        _ => Attempt::Failed(error),
    }
}

fn short(e: &ureq::Error) -> String {
    e.to_string().chars().take(200).collect()
}
