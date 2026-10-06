//! Errors and their stable machine-readable codes.

use std::fmt;

/// The category of an error. Each maps to a process exit code and to the
/// `code` field of the `--json` error object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Anything not covered below (network, API, I/O, internal). Exit 1.
    General,
    /// The command line was used incorrectly. Exit 2.
    Usage,
    /// Missing, invalid or insufficient credentials. Exit 3.
    Auth,
    /// A write was refused by the ownership rules. Exit 4.
    WriteDenied,
    /// A write was refused by a validator. Exit 5.
    Validation,
    /// `audit --fail-on` found actionable findings. Exit 6.
    AuditFindings,
}

impl ErrorCode {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::General => 1,
            Self::Usage => 2,
            Self::Auth => 3,
            Self::WriteDenied => 4,
            Self::Validation => 5,
            Self::AuditFindings => 6,
        }
    }

    /// The stable string used in `--json` error output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "error",
            Self::Usage => "usage",
            Self::Auth => "auth",
            Self::WriteDenied => "write_denied",
            Self::Validation => "validation",
            Self::AuditFindings => "audit_findings",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// HTTP 401, or a GraphQL authentication/permission error.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// Linear throttled the request.
    #[error("rate limited by Linear{}", retry_hint(*.retry_after_secs))]
    RateLimited { retry_after_secs: Option<u64> },

    /// The GraphQL response carried `errors`.
    #[error("Linear API error: {message}")]
    Api {
        message: String,
        /// Linear's `extensions.code`, e.g. `INPUT_ERROR`.
        code: Option<String>,
    },

    /// A non-success HTTP status without a GraphQL error body.
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },

    /// The body was not a valid GraphQL response, or did not match the query.
    #[error("could not decode the response: {0}")]
    Decode(String),

    /// The response had neither data nor errors.
    #[error("the response was empty")]
    Empty,

    /// A pagination invariant was violated (missing or repeated cursor, too many pages).
    #[error("pagination error: {0}")]
    Pagination(String),

    /// Invalid configuration.
    #[error("configuration error: {0}")]
    Config(String),

    /// The command line was used incorrectly.
    #[error("{0}")]
    Usage(String),
}

fn retry_hint(secs: Option<u64>) -> String {
    match secs {
        Some(s) => format!(" (retry in {s}s)"),
        None => String::new(),
    }
}

impl Error {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Auth(_) => ErrorCode::Auth,
            Self::Usage(_) => ErrorCode::Usage,
            _ => ErrorCode::General,
        }
    }
}
