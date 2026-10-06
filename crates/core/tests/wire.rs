//! Request building, response interpretation and pagination (no I/O involved).

use chrono::{DateTime, TimeZone, Utc};
use linear_core::queries::{self, Whoami};
use linear_core::types::{PageInfo, PageVars};
use linear_core::wire::{build_request, parse_response, Page, Pager, ResponseMeta};
use linear_core::{Error, ErrorCode};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A fixed "now": core never reads the clock.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
}

fn meta(status: u16) -> ResponseMeta {
    ResponseMeta {
        status,
        ..Default::default()
    }
}

// ------------------------------------------------------------------ requests

#[test]
fn request_without_variables_omits_them() {
    let req = build_request(&queries::whoami());
    let v: serde_json::Value = serde_json::from_str(&req.to_json()).unwrap();
    assert!(v["query"].as_str().unwrap().contains("viewer"));
    assert!(v.get("variables").is_none());
    assert_eq!(v["operationName"], "Whoami");
}

#[test]
fn request_carries_variables_in_camel_case() {
    let req = build_request(&queries::assigned_started_issues(PageVars {
        first: 50,
        after: Some("cursor-1".into()),
    }));
    let v: serde_json::Value = serde_json::from_str(&req.to_json()).unwrap();
    assert_eq!(v["variables"]["first"], 50);
    assert_eq!(v["variables"]["after"], "cursor-1");
    assert_eq!(v["operationName"], "AssignedStartedIssues");
}

// ------------------------------------------------------------------ responses

#[test]
fn a_successful_response_is_decoded() {
    let who: Whoami = parse_response(&meta(200), &fixture("whoami"), now()).unwrap();
    assert_eq!(who.organization.url_key, "example");
}

#[test]
fn authentication_errors_are_auth_errors() {
    let err =
        parse_response::<Whoami>(&meta(401), &fixture("error_unauthenticated"), now()).unwrap_err();
    assert!(matches!(err, Error::Auth(_)), "{err:?}");
    assert_eq!(err.code(), ErrorCode::Auth);
    assert_eq!(err.code().exit_code(), 3);
    // The user-facing text is included.
    assert!(err.to_string().contains("authenticate"));
}

#[test]
fn a_bare_401_without_a_body_is_still_an_auth_error() {
    let err = parse_response::<Whoami>(&meta(401), "", now()).unwrap_err();
    assert!(matches!(err, Error::Auth(_)));
}

#[test]
fn query_too_complex_is_an_api_error_with_its_code() {
    let err =
        parse_response::<Whoami>(&meta(400), &fixture("error_too_complex"), now()).unwrap_err();
    match err {
        Error::Api { message, code } => {
            assert_eq!(code.as_deref(), Some("INPUT_ERROR"));
            assert!(message.contains("too complex"), "{message}");
            assert!(
                message.contains("Maximum allowed complexity: 10000"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn rate_limit_wait_is_computed_from_the_reset_time_and_the_given_now() {
    let body = r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED","type":"ratelimited"}}]}"#;
    let reset = now().timestamp_millis() + 12_345;
    let err = parse_response::<Whoami>(
        &ResponseMeta {
            status: 400,
            rate_limit_reset_ms: Some(reset),
            ..Default::default()
        },
        body,
        now(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            Error::RateLimited {
                retry_after_secs: Some(13)
            }
        ),
        "{err:?}"
    );

    // A reset that already passed means "retry now", not a negative wait.
    let err = parse_response::<Whoami>(
        &ResponseMeta {
            status: 400,
            rate_limit_reset_ms: Some(now().timestamp_millis() - 5_000),
            ..Default::default()
        },
        body,
        now(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        Error::RateLimited {
            retry_after_secs: Some(0)
        }
    ));

    // Retry-After wins when present; a 429 without a body is also a rate limit.
    let err = parse_response::<Whoami>(
        &ResponseMeta {
            status: 429,
            retry_after_secs: Some(7),
            ..Default::default()
        },
        "",
        now(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        Error::RateLimited {
            retry_after_secs: Some(7)
        }
    ));
}

#[test]
fn partial_data_with_errors_is_a_failure_not_a_partial_success() {
    let body = format!(
        r#"{{"data":{},"errors":[{{"message":"Something broke"}}]}}"#,
        serde_json::from_str::<serde_json::Value>(&fixture("whoami")).unwrap()["data"]
    );
    let err = parse_response::<Whoami>(&meta(200), &body, now()).unwrap_err();
    assert!(matches!(err, Error::Api { .. }), "{err:?}");
}

#[test]
fn empty_and_malformed_bodies_are_reported_as_such() {
    let err = parse_response::<Whoami>(&meta(200), r#"{"data":null}"#, now()).unwrap_err();
    assert!(matches!(err, Error::Empty));

    let err = parse_response::<Whoami>(&meta(200), "<html>", now()).unwrap_err();
    assert!(matches!(err, Error::Decode(_)));

    // Data of the wrong shape names the problem instead of yielding defaults.
    let err = parse_response::<Whoami>(&meta(200), r#"{"data":{"viewer":{}}}"#, now()).unwrap_err();
    assert!(matches!(err, Error::Decode(_)));

    let err = parse_response::<Whoami>(&meta(502), "<html>Bad gateway</html>", now()).unwrap_err();
    assert!(matches!(err, Error::Http { status: 502, .. }));
    assert_eq!(err.code(), ErrorCode::General);
}

// ------------------------------------------------------------------ pagination

struct TestPage {
    items: Vec<u32>,
    info: PageInfo,
}

impl Page for TestPage {
    type Item = u32;
    fn page_info(&self) -> &PageInfo {
        &self.info
    }
    fn into_items(self) -> Vec<u32> {
        self.items
    }
}

fn page(items: &[u32], next: Option<&str>) -> TestPage {
    TestPage {
        items: items.to_vec(),
        info: PageInfo {
            has_next_page: next.is_some(),
            end_cursor: next.map(str::to_owned),
        },
    }
}

#[test]
fn pager_walks_every_page_in_order() {
    let mut pages = vec![
        page(&[1, 2], Some("a")),
        page(&[3, 4], Some("b")),
        page(&[5], None),
    ]
    .into_iter();
    let mut seen_vars = Vec::new();
    let items = Pager::new(2)
        .collect_all(|vars| {
            seen_vars.push((vars.first, vars.after.clone()));
            Ok(pages.next().expect("no extra fetch"))
        })
        .unwrap();
    assert_eq!(items, vec![1, 2, 3, 4, 5]);
    assert_eq!(
        seen_vars,
        vec![(2, None), (2, Some("a".into())), (2, Some("b".into()))]
    );
}

#[test]
fn pager_stops_after_a_single_page() {
    let mut calls = 0;
    let items = Pager::new(50)
        .collect_all(|_| {
            calls += 1;
            Ok(page(&[], None))
        })
        .unwrap();
    assert!(items.is_empty());
    assert_eq!(calls, 1);
}

#[test]
fn pager_rejects_a_next_page_without_a_cursor() {
    let mut p = Pager::new(10);
    let bad = TestPage {
        items: vec![1],
        info: PageInfo {
            has_next_page: true,
            end_cursor: None,
        },
    };
    assert!(matches!(p.accept(bad), Err(Error::Pagination(_))));
}

#[test]
fn pager_rejects_a_cursor_that_does_not_advance() {
    let mut p = Pager::new(10);
    p.accept(page(&[1], Some("same"))).unwrap();
    let err = p.accept(page(&[2], Some("same"))).unwrap_err();
    assert!(matches!(err, Error::Pagination(_)), "{err:?}");
}

#[test]
fn pager_enforces_a_page_limit() {
    let mut n = 0;
    let err = Pager::new(1)
        .with_max_pages(3)
        .collect_all(|_| {
            n += 1;
            Ok(page(&[n], Some(&format!("c{n}"))))
        })
        .unwrap_err();
    assert!(matches!(err, Error::Pagination(_)));
    assert_eq!(n, 3);
}

#[test]
fn pager_propagates_fetch_errors() {
    let err = Pager::new(5)
        .collect_all::<TestPage, _>(|_| Err(Error::Auth("nope".into())))
        .unwrap_err();
    assert!(matches!(err, Error::Auth(_)));
}

#[test]
fn a_real_connection_type_paginates() {
    use linear_core::queries::AssignedStartedIssues;
    let d: AssignedStartedIssues =
        parse_response(&meta(200), &fixture("assigned_issues"), now()).unwrap();
    let mut pager = Pager::new(50);
    let items = pager.accept(d.viewer.assigned_issues).unwrap();
    assert!(!items.is_empty());
    assert!(pager.next_vars().is_none());
}

// ------------------------------------------------------------------ error codes

#[test]
fn error_codes_map_to_the_documented_exit_codes() {
    let table = [
        (ErrorCode::General, 1, "error"),
        (ErrorCode::Usage, 2, "usage"),
        (ErrorCode::Auth, 3, "auth"),
        (ErrorCode::WriteDenied, 4, "write_denied"),
        (ErrorCode::Validation, 5, "validation"),
        (ErrorCode::AuditFindings, 6, "audit_findings"),
    ];
    for (code, exit, name) in table {
        assert_eq!(code.exit_code(), exit);
        assert_eq!(code.as_str(), name);
    }
    assert_eq!(Error::Usage("x".into()).code(), ErrorCode::Usage);
    assert_eq!(Error::Auth("x".into()).code(), ErrorCode::Auth);
    assert_eq!(Error::Empty.code(), ErrorCode::General);
}
