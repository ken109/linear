//! WebAssembly boundary for `linear-core`.
//!
//! Everything crosses the boundary as JSON text: strings in, strings out. That
//! keeps the exported surface to a handful of `fn(&str, ...) -> String`
//! functions, independent of the Rust types behind them, and avoids
//! `serde-wasm-bindgen`.
//!
//! The caller (TypeScript) performs the HTTP request itself. It asks
//! [`build_request`] what to send, sends it, then hands the status, headers and
//! body to [`parse_response`].

use chrono::{DateTime, Utc};
use linear_core::queries;
use linear_core::types::PageVars;
use linear_core::wire::{self, ResponseMeta};
use linear_core::{Error, API_URL};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
struct PageVarsIn {
    #[serde(default = "default_first")]
    first: i32,
    #[serde(default)]
    after: Option<String>,
}

impl Default for PageVarsIn {
    fn default() -> Self {
        Self {
            first: default_first(),
            after: None,
        }
    }
}

fn default_first() -> i32 {
    50
}

impl From<PageVarsIn> for PageVars {
    fn from(v: PageVarsIn) -> Self {
        PageVars {
            first: v.first,
            after: v.after,
        }
    }
}

#[derive(Deserialize)]
struct IdIn {
    id: String,
}

/// What `build_request` returns: where and what to POST.
#[derive(Serialize)]
struct RequestOut {
    url: &'static str,
    /// The JSON text to send as the request body.
    body: String,
}

/// The HTTP facts `parse_response` needs besides the body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaIn {
    status: u16,
    #[serde(default)]
    retry_after_secs: Option<u64>,
    #[serde(default)]
    rate_limit_reset_ms: Option<i64>,
}

fn vars<T: serde::de::DeserializeOwned + Default>(json: &str) -> Result<T, String> {
    if json.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(json).map_err(|e| format!("invalid variables: {e}"))
}

fn error_json(code: &str, message: impl Into<String>) -> String {
    json!({ "ok": false, "error": { "code": code, "message": message.into() } }).to_string()
}

fn core_error_json(e: &Error) -> String {
    let mut err = json!({ "code": e.code().as_str(), "message": e.to_string() });
    if let Error::RateLimited { retry_after_secs } = e {
        err["retryAfterSecs"] = json!(retry_after_secs);
    }
    json!({ "ok": false, "error": err }).to_string()
}

/// Build the GraphQL request for a named operation.
///
/// `operation` is one of `whoami`, `issue` (`{"id": "KK-1"}`),
/// `assigned_started_issues` and `projects` (`{"first": 10, "after": null}`).
/// `variables_json` may be empty for operations without variables.
///
/// Returns `{"ok":true,"request":{"url","body"}}` or `{"ok":false,"error":{...}}`.
#[wasm_bindgen]
pub fn build_request(operation: &str, variables_json: &str) -> String {
    match build_request_inner(operation, variables_json) {
        Ok(req) => json!({ "ok": true, "request": req }).to_string(),
        Err(msg) => error_json("usage", msg),
    }
}

fn build_request_inner(operation: &str, variables_json: &str) -> Result<RequestOut, String> {
    let request = match operation {
        "whoami" => wire::build_request(&queries::whoami()),
        "issue" => {
            let v: IdIn = serde_json::from_str(variables_json)
                .map_err(|e| format!("invalid variables: {e}"))?;
            wire::build_request(&queries::issue(v.id))
        }
        "assigned_started_issues" => {
            let v: PageVarsIn = vars(variables_json)?;
            wire::build_request(&queries::assigned_started_issues(v.into()))
        }
        "projects" => {
            let v: PageVarsIn = vars(variables_json)?;
            wire::build_request(&queries::projects(v.into()))
        }
        other => return Err(format!("unknown operation: {other}")),
    };
    Ok(RequestOut {
        url: API_URL,
        body: request.to_json(),
    })
}

/// Interpret a Linear response for a named operation.
///
/// `meta_json` is `{"status": 200, "retryAfterSecs": null, "rateLimitResetMs": null}`
/// (only `status` is required). `now_ms` is the current time in epoch
/// milliseconds; core never reads the clock itself.
///
/// Returns `{"ok":true,"data":...}` or `{"ok":false,"error":{"code","message"}}`.
#[wasm_bindgen]
pub fn parse_response(operation: &str, meta_json: &str, body: &str, now_ms: f64) -> String {
    let meta: MetaIn = match serde_json::from_str(meta_json) {
        Ok(m) => m,
        Err(e) => return error_json("usage", format!("invalid meta: {e}")),
    };
    let meta = ResponseMeta {
        status: meta.status,
        retry_after_secs: meta.retry_after_secs,
        rate_limit_reset_ms: meta.rate_limit_reset_ms,
    };
    let Some(now) = DateTime::<Utc>::from_timestamp_millis(now_ms as i64) else {
        return error_json("usage", "now_ms is out of range");
    };

    let result: Result<Value, Error> = match operation {
        "whoami" => wire::parse_response::<queries::Whoami>(&meta, body, now)
            .map(|d| json!({ "viewer": d.viewer, "organization": d.organization })),
        "issue" => wire::parse_response::<queries::IssueById>(&meta, body, now)
            .map(|d| json!({ "issue": d.issue })),
        "assigned_started_issues" => {
            wire::parse_response::<queries::AssignedStartedIssues>(&meta, body, now)
                .map(|d| json!({ "viewer": { "assignedIssues": d.viewer.assigned_issues } }))
        }
        "projects" => wire::parse_response::<queries::Projects>(&meta, body, now)
            .map(|d| json!({ "projects": d.projects })),
        other => return error_json("usage", format!("unknown operation: {other}")),
    };

    match result {
        Ok(data) => json!({ "ok": true, "data": data }).to_string(),
        Err(e) => core_error_json(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_whoami() {
        let out: Value = serde_json::from_str(&build_request("whoami", "")).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["request"]["url"], API_URL);
        assert!(out["request"]["body"].as_str().unwrap().contains("viewer"));
    }

    #[test]
    fn unknown_operation_is_an_error_object() {
        let out: Value = serde_json::from_str(&build_request("nope", "")).unwrap();
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"]["code"], "usage");
    }

    #[test]
    fn parse_auth_failure() {
        let out: Value =
            serde_json::from_str(&parse_response("whoami", r#"{"status":401}"#, "{}", 0.0))
                .unwrap();
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"]["code"], "auth");
    }

    #[test]
    fn parse_whoami() {
        let body = r#"{"data":{"viewer":{"id":"u1","name":"A","displayName":"a","email":"a@x","active":true,"isMe":true},"organization":{"id":"o1","name":"Org","urlKey":"org"}}}"#;
        let out: Value =
            serde_json::from_str(&parse_response("whoami", r#"{"status":200}"#, body, 0.0))
                .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"]["organization"]["urlKey"], "org");
    }
}
