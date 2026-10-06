//! When a request to Linear is tried again.
//!
//! Only reads are ever retried. A write may have reached Linear even when its
//! response was lost (a timeout, a 502 from a proxy), so sending it again could
//! create a second issue; a failed write is reported and the caller decides.
//!
//! The decision is pure: the caller sleeps and sends.

use crate::error::Error;
use std::time::Duration;

/// How a read that failed is retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// How many times a failed read is sent again (so `2` means up to 3 requests).
    pub max_retries: u32,
    /// The wait before the first retry; each further retry waits twice as long.
    pub base: Duration,
    /// The longest rate-limit wait that is sat out. A rate limit that asks for
    /// more (the hourly request limit resets in up to an hour) is not retried:
    /// the command stops, as it always did.
    pub max_rate_limit_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base: Duration::from_secs(1),
            max_rate_limit_wait: Duration::from_secs(10),
        }
    }
}

/// What went wrong with a request, as far as retrying is concerned.
#[derive(Debug, Clone, Copy)]
pub enum Failure<'a> {
    /// No response: a timeout, or the connection failed or broke off.
    Transport,
    /// Linear (or something in front of it) answered with this error.
    Response(&'a Error),
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn never() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    /// How long to wait before retry number `retry` (1 for the first), or
    /// `None` when the request is not retried: the failure is not transient
    /// (a bad query, an authentication error), the retries are used up, or
    /// Linear asked for a longer rate-limit wait than is worth sitting out.
    pub fn wait(&self, failure: Failure<'_>, retry: u32) -> Option<Duration> {
        if retry == 0 || retry > self.max_retries {
            return None;
        }
        let backoff = self.base.saturating_mul(1 << (retry - 1).min(16));
        match failure {
            Failure::Transport => Some(backoff),
            Failure::Response(Error::Http { status, .. }) if *status >= 500 => Some(backoff),
            Failure::Response(Error::RateLimited { retry_after_secs }) => {
                // Without a hint there is no telling how long the limit lasts.
                let asked = Duration::from_secs((*retry_after_secs)?);
                (asked <= self.max_rate_limit_wait).then_some(asked)
            }
            Failure::Response(_) => None,
        }
    }
}
