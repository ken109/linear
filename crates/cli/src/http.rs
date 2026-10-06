//! The HTTP side: send a core `Request`, hand the response back to core.

use crate::error::{CliError, Result};
use chrono::Utc;
use linear_core::auth::Credential;
use linear_core::wire::{build_request, parse_response, ResponseMeta};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

/// Overrides the API endpoint (used by tests and proxies).
pub const API_URL_ENV: &str = "LINEAR_API_URL";

pub struct Client {
    agent: ureq::Agent,
    url: String,
    credential: Credential,
}

impl Client {
    pub fn new(credential: Credential) -> Self {
        let url = std::env::var(API_URL_ENV)
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| linear_core::API_URL.to_owned());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // Status handling belongs to core, which understands Linear's error bodies.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("linear-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            url,
            credential,
        }
    }

    /// Run an operation and decode its data.
    pub fn execute<Q, V, T>(&self, op: &cynic::Operation<Q, V>) -> Result<T>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        let request = build_request(op);
        let mut response = self
            .agent
            .post(&self.url)
            .header("content-type", "application/json")
            .header("authorization", self.credential.authorization())
            .send(request.to_json())
            // Do not include the underlying error's text: it can echo request details.
            .map_err(|e| CliError::general(format!("request to Linear failed: {}", short(&e))))?;

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
        let body = response.body_mut().read_to_string().map_err(|e| {
            CliError::general(format!("could not read Linear's response: {}", short(&e)))
        })?;

        Ok(parse_response(&meta, &body, Utc::now())?)
    }
}

fn short(e: &ureq::Error) -> String {
    e.to_string().chars().take(200).collect()
}
