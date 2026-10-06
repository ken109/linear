//! When a failed request is tried again.

use linear_core::retry::{Failure, RetryPolicy};
use linear_core::Error;
use std::time::Duration;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn http(status: u16) -> Error {
    Error::Http {
        status,
        body: String::new(),
    }
}

fn rate_limited(after: Option<u64>) -> Error {
    Error::RateLimited {
        retry_after_secs: after,
    }
}

#[test]
fn a_transport_failure_is_retried_with_a_doubling_wait_until_the_retries_run_out() {
    let p = RetryPolicy::default();
    assert_eq!(p.max_retries, 2);
    assert_eq!(p.wait(Failure::Transport, 1), Some(secs(1)));
    assert_eq!(p.wait(Failure::Transport, 2), Some(secs(2)));
    assert_eq!(p.wait(Failure::Transport, 3), None);
}

#[test]
fn only_server_errors_are_retried_among_http_statuses() {
    let p = RetryPolicy::default();
    for status in [500, 502, 503, 504] {
        assert_eq!(
            p.wait(Failure::Response(&http(status)), 1),
            Some(secs(1)),
            "{status}"
        );
    }
    for status in [400, 404, 422] {
        assert_eq!(
            p.wait(Failure::Response(&http(status)), 1),
            None,
            "{status}"
        );
    }
}

#[test]
fn errors_that_a_second_try_cannot_fix_are_never_retried() {
    let p = RetryPolicy::default();
    for e in [
        Error::Auth("bad key".into()),
        Error::Api {
            message: "no such issue".into(),
            code: None,
        },
        Error::Decode("x".into()),
        Error::Empty,
    ] {
        assert_eq!(p.wait(Failure::Response(&e), 1), None, "{e}");
    }
}

#[test]
fn a_short_rate_limit_is_waited_out_but_a_long_one_stops_the_command() {
    let p = RetryPolicy::default();
    assert_eq!(
        p.wait(Failure::Response(&rate_limited(Some(0))), 1),
        Some(secs(0))
    );
    assert_eq!(
        p.wait(Failure::Response(&rate_limited(Some(3))), 1),
        Some(secs(3))
    );
    // The hourly request limit resets in up to an hour: never sat out.
    assert_eq!(
        p.wait(Failure::Response(&rate_limited(Some(2400))), 1),
        None
    );
    // No hint, no telling how long it lasts.
    assert_eq!(p.wait(Failure::Response(&rate_limited(None)), 1), None);
}

#[test]
fn never_retries_nothing_and_zero_retries_turns_it_off() {
    assert_eq!(RetryPolicy::never().wait(Failure::Transport, 1), None);
    let p = RetryPolicy {
        max_retries: 0,
        ..RetryPolicy::default()
    };
    assert_eq!(p.wait(Failure::Response(&http(503)), 1), None);
}

#[test]
fn the_wait_never_overflows() {
    let p = RetryPolicy {
        max_retries: u32::MAX,
        ..RetryPolicy::default()
    };
    assert!(p.wait(Failure::Transport, u32::MAX).is_some());
}
