//! Verifying Linear's webhook signatures.
//!
//! Linear signs the raw request body with HMAC-SHA256 under the webhook's
//! signing secret and sends the hex digest in the `Linear-Signature` header.
//! The body carries `webhookTimestamp` (epoch milliseconds); a receiver should
//! reject a delivery whose timestamp is far from its own clock, so that a
//! captured delivery cannot be replayed later. Everything here is pure: `now`
//! is an argument.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// How far `webhookTimestamp` may be from `now`, in either direction.
/// This is the minute Linear's documentation recommends.
pub const TOLERANCE_MS: i64 = 60_000;

type HmacSha256 = Hmac<Sha256>;

/// What a verified delivery says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookEvent {
    /// `create`, `update` or `remove`.
    pub action: String,
    /// The kind of resource: `Issue`, `Project`, `ProjectUpdate`, ...
    #[serde(rename = "type")]
    pub resource_type: String,
    pub organization_id: Option<String>,
    /// When Linear sent it, in epoch milliseconds.
    pub webhook_timestamp: i64,
}

/// Why a delivery was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Rejection {
    /// The signature is not 64 hex digits.
    MalformedSignature,
    /// The signature is well formed but is not the one for this body and secret.
    SignatureMismatch,
    /// The signature is right but the body is not a webhook payload.
    MalformedBody,
    /// The signature is right but the body has no `webhookTimestamp`.
    MissingTimestamp,
    /// The signature is right but the delivery is too old (or from the future).
    StaleTimestamp,
}

/// The outcome of checking a delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Verification {
    Valid { event: WebhookEvent },
    Invalid { reason: Rejection },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    action: String,
    #[serde(rename = "type")]
    resource_type: String,
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default)]
    webhook_timestamp: Option<i64>,
}

/// The hex HMAC-SHA256 of `body` under `secret`: what Linear puts in
/// `Linear-Signature`. Useful for tests and for sending a delivery to yourself.
pub fn sign(secret: &str, body: &str) -> String {
    hex_encode(&mac(secret, body).finalize().into_bytes())
}

/// Check a delivery: signature first (in constant time), then the timestamp.
///
/// `body` must be the exact text received, not a re-serialization. An empty
/// secret is a configuration error, not a rejected delivery.
pub fn verify_webhook(
    body: &str,
    signature: &str,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<Verification> {
    if secret.is_empty() {
        return Err(Error::Usage("the webhook secret is empty".into()));
    }
    let invalid = |reason| Ok(Verification::Invalid { reason });

    let Some(expected) = hex_decode(signature.trim()).filter(|b| b.len() == 32) else {
        return invalid(Rejection::MalformedSignature);
    };
    if mac(secret, body).verify_slice(&expected).is_err() {
        return invalid(Rejection::SignatureMismatch);
    }

    let Ok(payload) = serde_json::from_str::<Payload>(body) else {
        return invalid(Rejection::MalformedBody);
    };
    let Some(sent) = payload.webhook_timestamp else {
        return invalid(Rejection::MissingTimestamp);
    };
    if (now.timestamp_millis() - sent).abs() > TOLERANCE_MS {
        return invalid(Rejection::StaleTimestamp);
    }

    Ok(Verification::Valid {
        event: WebhookEvent {
            action: payload.action,
            resource_type: payload.resource_type,
            organization_id: payload.organization_id,
            webhook_timestamp: sent,
        },
    })
}

fn mac(secret: &str, body: &str) -> HmacSha256 {
    // HMAC accepts a key of any length, so this cannot fail.
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
    mac.update(body.as_bytes());
    mac
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    s.as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}
