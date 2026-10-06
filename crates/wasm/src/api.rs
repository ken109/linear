//! The six functions, as plain Rust: JSON text in, JSON text out.
//!
//! `lib.rs` exports them to JavaScript unchanged. They live here so that they
//! can be tested (and compared with the native core) without a JS runtime.
//!
//! Every function returns one of
//!
//! ```text
//! {"ok": true,  "data": ...}
//! {"ok": false, "error": {"code", "message", ...}}
//! ```
//!
//! and never panics on bad input: malformed JSON, unknown operations and
//! out-of-range numbers are `usage` errors.

use crate::ops;
use chrono::{DateTime, Utc};
use linear_core::audit::{self, AuditConfig, AuditOptions, AuditReport, Finding, Snapshot};
use linear_core::refresh::{self, RefreshEvent, RefreshMeta};
use linear_core::wire::ResponseMeta;
use linear_core::{webhook, Error, API_URL, SCHEMA_VERSION};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The `code` of an error. These are `linear_core::ErrorCode::as_str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub enum ErrorCodeName {
    #[serde(rename = "error")]
    General,
    #[serde(rename = "usage")]
    Usage,
    #[serde(rename = "auth")]
    Auth,
    #[serde(rename = "write_denied")]
    WriteDenied,
    #[serde(rename = "validation")]
    Validation,
    #[serde(rename = "audit_findings")]
    AuditFindings,
}

/// What a failed call reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub code: ErrorCodeName,
    pub message: String,
    /// For a rate-limit error: how long to wait, if Linear said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    /// For a GraphQL error: Linear's own `extensions.code` (`INPUT_ERROR`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_code: Option<String>,
}

impl ErrorBody {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCodeName::Usage,
            message: message.into(),
            retry_after_secs: None,
            api_code: None,
        }
    }

    pub fn from_core(e: &Error) -> Self {
        use linear_core::ErrorCode as C;
        let code = match e.code() {
            C::General => ErrorCodeName::General,
            C::Usage => ErrorCodeName::Usage,
            C::Auth => ErrorCodeName::Auth,
            C::WriteDenied => ErrorCodeName::WriteDenied,
            C::Validation => ErrorCodeName::Validation,
            C::AuditFindings => ErrorCodeName::AuditFindings,
        };
        Self {
            code,
            message: e.to_string(),
            retry_after_secs: match e {
                Error::RateLimited { retry_after_secs } => *retry_after_secs,
                _ => None,
            },
            api_code: match e {
                Error::Api { code, .. } => code.clone(),
                _ => None,
            },
        }
    }
}

/// What `build_request` returns: where and what to POST.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BuiltRequest {
    pub url: String,
    /// The JSON text to send as the request body.
    pub body: String,
}

/// The HTTP facts `parse_response` needs besides the body.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpMeta {
    pub status: u16,
    /// `Retry-After`, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    /// `X-RateLimit-Requests-Reset`: epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_reset_ms: Option<i64>,
}

#[derive(Serialize)]
struct Success<'a, T> {
    ok: bool,
    data: &'a T,
}

#[derive(Serialize)]
struct Failed<'a> {
    ok: bool,
    error: &'a ErrorBody,
}

fn success<T: Serialize>(data: &T) -> String {
    serde_json::to_string(&Success { ok: true, data }).expect("results always serialize")
}

fn failure(error: &ErrorBody) -> String {
    serde_json::to_string(&Failed { ok: false, error }).expect("errors always serialize")
}

fn usage(message: impl Into<String>) -> String {
    failure(&ErrorBody::usage(message))
}

fn core_failure(e: &Error) -> String {
    failure(&ErrorBody::from_core(e))
}

/// Parse an input. `what` names it in the error. `default` is used when the
/// text is empty or `null` (an input that may be left out).
fn input<T: DeserializeOwned>(what: &str, json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| format!("invalid {what}: {e}"))
}

fn input_or<T: DeserializeOwned>(
    what: &str,
    json: &str,
    default: impl FnOnce() -> T,
) -> Result<T, String> {
    match json.trim() {
        "" | "null" => Ok(default()),
        text => input(what, text),
    }
}

fn instant(what: &str, ms: f64) -> Result<DateTime<Utc>, String> {
    if !ms.is_finite() {
        return Err(format!("{what} is not a number"));
    }
    DateTime::<Utc>::from_timestamp_millis(ms as i64)
        .ok_or_else(|| format!("{what} is out of range"))
}

/// The version of every JSON shape that crosses this boundary
/// (`linear_core::SCHEMA_VERSION`). The TypeScript package checks it at start-up.
pub fn schema_version() -> u32 {
    SCHEMA_VERSION
}

/// Build the GraphQL request for a named operation.
///
/// `params_json` may be empty for operations without parameters.
/// Result data: [`BuiltRequest`].
pub fn build_request(operation: &str, params_json: &str) -> String {
    let Some(op) = ops::find(operation) else {
        return usage(format!("unknown operation: {operation}"));
    };
    match op.build(params_json) {
        Ok(request) => success(&BuiltRequest {
            url: API_URL.to_owned(),
            body: request.to_json(),
        }),
        Err(message) => usage(message),
    }
}

/// Interpret Linear's response to the request `build_request` made.
///
/// `meta_json` is an [`HttpMeta`]. `now_ms` is the current time in epoch
/// milliseconds: core never reads the clock. Result data: the operation's
/// result type, in the shape of Linear's own response.
pub fn parse_response(operation: &str, meta_json: &str, body: &str, now_ms: f64) -> String {
    let Some(op) = ops::find(operation) else {
        return usage(format!("unknown operation: {operation}"));
    };
    let meta: HttpMeta = match input("meta", meta_json) {
        Ok(m) => m,
        Err(e) => return usage(e),
    };
    let now = match instant("now", now_ms) {
        Ok(t) => t,
        Err(e) => return usage(e),
    };
    let meta = ResponseMeta {
        status: meta.status,
        retry_after_secs: meta.retry_after_secs,
        rate_limit_reset_ms: meta.rate_limit_reset_ms,
    };
    match op.parse(&meta, body, now) {
        Ok(data) => success(&data),
        Err(e) => core_failure(&e),
    }
}

/// Run every `audit` rule over a snapshot. Result data: [`AuditReport`].
///
/// `config_json` is an `AuditConfig` (empty for the defaults). `options_json`
/// narrows the audit to some issues (`AuditOptions`); leave it out for all.
pub fn audit(
    snapshot_json: &str,
    config_json: &str,
    now_ms: f64,
    options_json: Option<&str>,
) -> String {
    let snapshot: Snapshot = match input("snapshot", snapshot_json) {
        Ok(s) => s,
        Err(e) => return usage(e),
    };
    let config: AuditConfig = match input_or("config", config_json, AuditConfig::default) {
        Ok(c) => c,
        Err(e) => return usage(e),
    };
    let now = match instant("now", now_ms) {
        Ok(t) => t,
        Err(e) => return usage(e),
    };
    let options: Option<AuditOptions> = match options_json {
        None => None,
        Some(text) if matches!(text.trim(), "" | "null") => None,
        Some(text) => match input("options", text) {
            Ok(o) => Some(o),
            Err(e) => return usage(e),
        },
    };
    let report = match options {
        None => Ok(audit::audit(&snapshot, &config, now)),
        Some(options) => audit::audit_scoped(&snapshot, &config, &options, now),
    };
    match report {
        Ok(report) => success(&report),
        Err(e) => core_failure(&e),
    }
}

/// The findings in `current` that were not in `previous`. Both are an
/// `AuditReport`; `previous` may be empty or `null` (no earlier audit: everything is new).
/// Result data: an array of `Finding`.
pub fn diff(previous_json: &str, current_json: &str) -> String {
    let previous: AuditReport =
        match input_or("previous report", previous_json, AuditReport::default) {
            Ok(r) => r,
            Err(e) => return usage(e),
        };
    let current: AuditReport = match input("current report", current_json) {
        Ok(r) => r,
        Err(e) => return usage(e),
    };
    let new: Vec<Finding> = audit::diff(&previous.findings, &current.findings);
    success(&new)
}

/// Whether a cached snapshot should be refreshed now. Result data: a `RefreshDecision`.
///
/// `meta_json` is a `RefreshMeta` (empty or `null` for a cache that does not
/// exist yet), `event_json` a `RefreshEvent`.
pub fn decide_refresh(meta_json: &str, event_json: &str, now_ms: f64) -> String {
    let meta: RefreshMeta = match input_or("meta", meta_json, RefreshMeta::empty) {
        Ok(m) => m,
        Err(e) => return usage(e),
    };
    let event: RefreshEvent = match input("event", event_json) {
        Ok(e) => e,
        Err(e) => return usage(e),
    };
    let now = match instant("now", now_ms) {
        Ok(t) => t,
        Err(e) => return usage(e),
    };
    success(&refresh::decide_refresh(&meta, &event, now))
}

/// Check a webhook delivery: `body` is the exact text received, `signature`
/// the `Linear-Signature` header. Result data: a `Verification`. A delivery
/// that is rejected is a successful call with `status: "invalid"`; an error is
/// reserved for a call that cannot be answered (an empty secret, a bad `now`).
pub fn verify_webhook(body: &str, signature: &str, secret: &str, now_ms: f64) -> String {
    let now = match instant("now", now_ms) {
        Ok(t) => t,
        Err(e) => return usage(e),
    };
    match webhook::verify_webhook(body, signature, secret, now) {
        Ok(verification) => success(&verification),
        Err(e) => core_failure(&e),
    }
}
