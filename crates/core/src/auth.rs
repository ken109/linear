//! Credentials: pure types and the rules for building an `Authorization` header.
//!
//! Reading and writing credential files is the CLI's job. A [`Secret`] never
//! prints its value through `Debug`, and there is no `Display`.

use chrono::{DateTime, Utc};
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

/// A credential as stored in `credentials/<workspace>.json`.
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
}

impl Credential {
    pub fn method(&self) -> AuthMethod {
        match self {
            Self::ApiKey { .. } => AuthMethod::ApiKey,
            Self::Oauth { .. } => AuthMethod::Oauth,
        }
    }

    /// The value for the `Authorization` header. Personal API keys are sent
    /// bare; OAuth tokens as a bearer token. The result contains the secret.
    pub fn authorization(&self) -> String {
        match self {
            Self::ApiKey { api_key } => api_key.expose().to_owned(),
            Self::Oauth { access_token, .. } => format!("Bearer {}", access_token.expose()),
        }
    }

    /// Whether an OAuth access token has expired (API keys never do).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        matches!(self, Self::Oauth { expires_at: Some(t), .. } if *t <= now)
    }
}

/// The environment variable that overrides the stored API key for a workspace:
/// `LINEAR_API_KEY_<NAME>`, upper-cased, with non-alphanumerics as `_`.
pub fn api_key_env_var(workspace: &str) -> String {
    let suffix: String = workspace
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("LINEAR_API_KEY_{suffix}")
}
