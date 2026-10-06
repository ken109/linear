//! The six functions, called as the JavaScript side calls them: JSON text in,
//! JSON text out. Run natively; `tests/node` runs the same through the wasm.

use linear_wasm::api;
use linear_wasm::ops::OPERATIONS;
use serde_json::{json, Value};

const NOW_MS: f64 = 1_792_497_600_000.0; // 2026-10-20T12:00:00Z

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../core/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn call(out: String) -> Value {
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("not JSON ({e}): {out}"))
}

fn data(out: String) -> Value {
    let v = call(out);
    assert_eq!(v["ok"], true, "{v:#}");
    v["data"].clone()
}

fn error(out: String) -> Value {
    let v = call(out);
    assert_eq!(v["ok"], false, "{v:#}");
    v["error"].clone()
}

fn ok_meta() -> String {
    json!({ "status": 200 }).to_string()
}

fn request_vars(operation: &str, params: &str) -> Value {
    let built = data(api::build_request(operation, params));
    assert_eq!(built["url"], "https://api.linear.app/graphql");
    let body: Value = serde_json::from_str(built["body"].as_str().unwrap()).unwrap();
    assert!(body["query"].as_str().unwrap().contains("query"), "{body}");
    body.get("variables").cloned().unwrap_or(Value::Null)
}

// ------------------------------------------------------------ build_request

#[test]
fn every_operation_builds_a_request() {
    for op in OPERATIONS {
        let params = if op.name.contains("view")
            || matches!(
                op.name,
                "issue" | "issue_comments" | "milestones_of_project"
            ) {
            r#"{"id":"KK-1"}"#
        } else {
            "{}"
        };
        let built = data(api::build_request(op.name, params));
        assert!(
            built["body"].as_str().unwrap().contains("query"),
            "{}",
            op.name
        );
    }
}

#[test]
fn operations_without_parameters_take_empty_null_or_an_empty_object() {
    for params in ["", "  ", "null", "{}"] {
        let body: Value = serde_json::from_str(
            data(api::build_request("whoami", params))["body"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(body["query"].as_str().unwrap().contains("viewer"));
    }
}

#[test]
fn an_id_goes_into_the_variables() {
    let vars = request_vars("issue", r#"{"id":"KK-12"}"#);
    assert_eq!(vars, json!({ "id": "KK-12" }));
}

#[test]
fn page_defaults_stay_under_the_complexity_limit_per_listing() {
    // 10 for projects: Linear rejected 20 (complexity 12720 > 10000).
    assert_eq!(request_vars("projects", "")["first"], 10);
    assert_eq!(request_vars("assigned_started_issues", "{}")["first"], 50);
    assert_eq!(request_vars("labels", "{}")["first"], 100);
    assert_eq!(
        request_vars("projects", r#"{"first":3,"after":"abc"}"#),
        json!({ "first": 3, "after": "abc" })
    );
}

#[test]
fn users_can_include_disabled_ones() {
    let vars = request_vars("users", r#"{"includeDisabled":true}"#);
    assert_eq!(vars["includeDisabled"], true);
    // Null unless asked for, so Linear's own default applies.
    assert_eq!(request_vars("users", "{}")["includeDisabled"], Value::Null);
}

#[test]
fn bad_requests_are_usage_errors_not_panics() {
    for (op, params, needle) in [
        ("nope", "{}", "unknown operation"),
        ("issue", "{}", "missing field `id`"),
        ("issue", "not json", "invalid parameters"),
        ("whoami", r#"{"x":1}"#, "unknown field"),
        ("projects", r#"{"first":"ten"}"#, "invalid parameters"),
    ] {
        let e = error(api::build_request(op, params));
        assert_eq!(e["code"], "usage", "{op} {params}");
        assert!(e["message"].as_str().unwrap().contains(needle), "{e}");
    }
}

// ----------------------------------------------------------- parse_response

#[test]
fn whoami_is_parsed_in_the_shape_of_the_response() {
    let body = fixture("whoami");
    let d = data(api::parse_response("whoami", &ok_meta(), &body, NOW_MS));
    assert_eq!(d["viewer"]["isMe"], true);
    assert_eq!(d["organization"]["urlKey"], "example");
}

#[test]
fn every_fixture_parses_through_its_operation() {
    for (op, name) in [
        ("whoami", "whoami"),
        ("issue", "issue"),
        ("assigned_started_issues", "assigned_issues"),
        ("projects", "projects"),
        ("issue_comments", "issue_comments"),
        ("templates", "templates"),
        ("initiatives", "initiatives"),
        ("issue_view", "issue_view"),
        ("project_view", "project_view"),
        ("milestones_of_project", "milestones"),
        ("milestone_view", "milestone_view"),
        ("initiative_view", "initiative_view"),
        ("labels", "labels"),
        ("teams", "teams"),
        ("users", "users"),
    ] {
        let d = data(api::parse_response(op, &ok_meta(), &fixture(name), NOW_MS));
        assert!(d.is_object(), "{op}");
    }
}

#[test]
fn a_rejected_key_is_an_auth_error() {
    let e = error(api::parse_response(
        "whoami",
        r#"{"status":401}"#,
        &fixture("error_unauthenticated"),
        NOW_MS,
    ));
    assert_eq!(e["code"], "auth");
}

#[test]
fn linears_own_error_code_is_passed_on() {
    let e = error(api::parse_response(
        "projects",
        r#"{"status":400}"#,
        &fixture("error_too_complex"),
        NOW_MS,
    ));
    assert_eq!(e["code"], "error");
    assert_eq!(e["apiCode"], "INPUT_ERROR");
}

#[test]
fn a_rate_limit_says_how_long_to_wait() {
    let e = error(api::parse_response(
        "whoami",
        r#"{"status":429,"rateLimitResetMs":1792497630000}"#,
        "{}",
        NOW_MS,
    ));
    assert_eq!(e["code"], "error");
    assert_eq!(e["retryAfterSecs"], 30);
    let e = error(api::parse_response(
        "whoami",
        r#"{"status":429,"retryAfterSecs":7}"#,
        "{}",
        NOW_MS,
    ));
    assert_eq!(e["retryAfterSecs"], 7);
}

#[test]
fn bad_inputs_to_parse_response_are_usage_errors() {
    for (op, meta, now, needle) in [
        ("nope", r#"{"status":200}"#, NOW_MS, "unknown operation"),
        ("whoami", "", NOW_MS, "invalid meta"),
        (
            "whoami",
            r#"{"status":200,"extra":1}"#,
            NOW_MS,
            "unknown field",
        ),
        ("whoami", r#"{"status":200}"#, f64::NAN, "not a number"),
        ("whoami", r#"{"status":200}"#, 1e300, "out of range"),
    ] {
        let e = error(api::parse_response(op, meta, "{}", now));
        assert_eq!(e["code"], "usage", "{op} {meta}");
        assert!(e["message"].as_str().unwrap().contains(needle), "{e}");
    }
}

#[test]
fn a_body_that_is_not_json_is_a_decode_error() {
    let e = error(api::parse_response("whoami", &ok_meta(), "<html>", NOW_MS));
    assert_eq!(e["code"], "error");
    assert!(e["message"].as_str().unwrap().contains("could not decode"));
}

// -------------------------------------------------------------------- audit

fn snapshot() -> String {
    let issues: Value = serde_json::from_str(&fixture("assigned_issues")).unwrap();
    let projects: Value = serde_json::from_str(&fixture("projects")).unwrap();
    json!({
        "workspace": "example",
        "issues": issues["data"]["viewer"]["assignedIssues"]["nodes"],
        "projects": projects["data"]["projects"]["nodes"],
    })
    .to_string()
}

#[test]
fn audit_matches_the_native_core() {
    let snapshot_json = snapshot();
    let got = data(api::audit(&snapshot_json, "", NOW_MS, None));

    let snap: linear_core::audit::Snapshot = serde_json::from_str(&snapshot_json).unwrap();
    let now = chrono::DateTime::from_timestamp_millis(NOW_MS as i64).unwrap();
    let native = linear_core::audit::audit(&snap, &Default::default(), now);
    assert_eq!(got, serde_json::to_value(native).unwrap());
    assert!(got["findings"].is_array());
}

#[test]
fn audit_takes_a_config_and_options() {
    let snapshot_json = snapshot();
    let config = json!({ "stale_days": 1, "status_update_days": 1, "validators": [] }).to_string();
    let all = data(api::audit(&snapshot_json, &config, NOW_MS, None));
    assert!(!all["findings"].as_array().unwrap().is_empty());

    let narrowed = data(api::audit(
        &snapshot_json,
        &config,
        NOW_MS,
        Some(r#"{"issues":["NO-SUCH-1"]}"#),
    ));
    assert_eq!(narrowed["unresolved_issues"], json!(["NO-SUCH-1"]));
}

#[test]
fn bad_inputs_to_audit_are_usage_errors() {
    let s = snapshot();
    for (snapshot_json, config, now, options, needle) in [
        ("{}", "", NOW_MS, None, "invalid snapshot"),
        (
            s.as_str(),
            "{\"stale_days\":-1}",
            NOW_MS,
            None,
            "invalid config",
        ),
        (s.as_str(), "{\"nope\":1}", NOW_MS, None, "unknown field"),
        (s.as_str(), "", f64::INFINITY, None, "not a number"),
        (s.as_str(), "", NOW_MS, Some("[1]"), "invalid options"),
        // `since` without `issues` has nothing to be about.
        (
            s.as_str(),
            "",
            NOW_MS,
            Some(r#"{"since":"2026-10-01T00:00:00Z"}"#),
            "--since needs --issues",
        ),
    ] {
        let e = error(api::audit(snapshot_json, config, now, options));
        assert_eq!(e["code"], "usage", "{needle}: {e}");
        assert!(e["message"].as_str().unwrap().contains(needle), "{e}");
    }
}

// --------------------------------------------------------------------- diff

#[test]
fn diff_reports_only_new_findings() {
    let s = snapshot();
    let strict = json!({ "stale_days": 0, "status_update_days": 0, "validators": [] }).to_string();
    let report = data(api::audit(&s, &strict, NOW_MS, None));
    let findings = report["findings"].as_array().unwrap();
    assert!(!findings.is_empty());

    let all = data(api::diff("", &report.to_string()));
    assert_eq!(
        all.as_array().unwrap().len(),
        findings.len(),
        "no previous: all new"
    );
    let none = data(api::diff(&report.to_string(), &report.to_string()));
    assert_eq!(none, json!([]));

    let earlier = json!({ "findings": findings[..findings.len() - 1], "unresolved_issues": [] });
    let new = data(api::diff(&earlier.to_string(), &report.to_string()));
    assert_eq!(new, json!([findings[findings.len() - 1]]));
}

#[test]
fn bad_inputs_to_diff_are_usage_errors() {
    let e = error(api::diff("{", "{}"));
    assert_eq!(e["code"], "usage");
    let e = error(api::diff("null", ""));
    assert!(e["message"]
        .as_str()
        .unwrap()
        .contains("invalid current report"));
}

// ----------------------------------------------------------- decide_refresh

#[test]
fn decide_refresh_answers_from_meta_event_and_now() {
    let meta = json!({ "schemaVersion": 1, "fetchedAt": "2026-10-20T11:59:00Z" });
    let d = data(api::decide_refresh(
        &meta.to_string(),
        r#"{"kind":"read"}"#,
        NOW_MS,
    ));
    assert_eq!(
        d,
        json!({
            "refresh": false,
            "reason": "fresh",
            "freshness": { "state": "fresh", "age_secs": 60 },
        })
    );
    let d = data(api::decide_refresh("", r#"{"kind":"read"}"#, NOW_MS));
    assert_eq!(d["reason"], "never-fetched");
    let d = data(api::decide_refresh(
        &meta.to_string(),
        r#"{"kind":"webhook","resourceType":"Issue","action":"update"}"#,
        NOW_MS,
    ));
    assert_eq!(d["refresh"], true);
}

#[test]
fn bad_inputs_to_decide_refresh_are_usage_errors() {
    for (meta, event) in [
        ("{", "{}"),
        ("", ""),
        ("", r#"{"kind":"nope"}"#),
        ("", r#"{"kind":"webhook"}"#),
    ] {
        let e = error(api::decide_refresh(meta, event, NOW_MS));
        assert_eq!(e["code"], "usage", "{meta} {event}");
    }
}

// ----------------------------------------------------------- verify_webhook

const SECRET: &str = "lin_wh_testsecret";
const BODY: &str = r#"{"action":"update","type":"Issue","organizationId":"org-1","webhookTimestamp":1760000000000}"#;
const SIGNATURE: &str = "19882a77e4dae983f01a21fbe0a53dd8142d0d699c37633bc9258a6237b13830";

#[test]
fn verify_webhook_accepts_a_known_signature() {
    let d = data(api::verify_webhook(
        BODY,
        SIGNATURE,
        SECRET,
        1_760_000_030_000.0,
    ));
    assert_eq!(d["status"], "valid");
    assert_eq!(d["event"]["type"], "Issue");
}

#[test]
fn verify_webhook_rejects_without_erroring() {
    let d = data(api::verify_webhook(
        BODY,
        SIGNATURE,
        SECRET,
        1_760_000_061_000.0,
    ));
    assert_eq!(
        d,
        json!({ "status": "invalid", "reason": "stale-timestamp" })
    );
    let d = data(api::verify_webhook(
        BODY,
        SIGNATURE,
        "other-secret",
        1_760_000_000_000.0,
    ));
    assert_eq!(
        d,
        json!({ "status": "invalid", "reason": "signature-mismatch" })
    );
}

#[test]
fn verify_webhook_with_an_empty_secret_is_an_error() {
    let e = error(api::verify_webhook(
        BODY,
        SIGNATURE,
        "",
        1_760_000_000_000.0,
    ));
    assert_eq!(e["code"], "usage");
}

// ------------------------------------------------------------ schema_version

#[test]
fn the_schema_version_is_the_cores() {
    assert_eq!(api::schema_version(), linear_core::SCHEMA_VERSION);
}
