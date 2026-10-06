//! Integration tests for `linear api` against a mock Linear server.

mod common;
use common::*;
use serde_json::{json, Value};

const KEY: &str = "lin_api_TESTKEY_do_not_leak_0123456789";
const KEY_OTHER: &str = "lin_api_OTHERKEY_do_not_leak_9876543210";

fn sandbox_with_workspaces() -> Sandbox {
    let sb = Sandbox::new();
    for (name, url_key) in [("example", "example"), ("other", "other-co")] {
        let o = sb.run(&["workspace", "add", name, "--url-key", url_key], None, &[]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
    sb
}

const ENV: &[(&str, &str)] = &[
    ("LINEAR_API_KEY_EXAMPLE", KEY),
    ("LINEAR_API_KEY_OTHER", KEY_OTHER),
];

fn sent(mock: &Mock) -> Value {
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1, "expected exactly one request");
    serde_json::from_str(&reqs[0].body).unwrap()
}

#[test]
fn a_query_is_sent_with_the_workspace_credentials_and_data_is_printed() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"viewer":{"name":"Alice Example"}}}"#)]);

    let o = sb.run(&["api", "{ viewer { name } }", "--json"], Some(&mock), ENV);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v, json!({"viewer": {"name": "Alice Example"}}));

    let body = sent(&mock);
    assert_eq!(body["query"], "{ viewer { name } }");
    // No variables were given, so the field is absent rather than null or {}.
    assert!(body.get("variables").is_none());
    assert_eq!(mock.requests()[0].authorization.as_deref(), Some(KEY));
    assert!(!stdout(&o).contains(KEY) && !stderr(&o).contains(KEY));
}

#[test]
fn quiet_prints_compact_json_and_the_default_is_pretty() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#), ok(r#"{"data":{"a":1}}"#)]);
    let o = sb.run(&["api", "{ a }", "--quiet"], Some(&mock), ENV);
    assert_eq!(stdout(&o), "{\"a\":1}\n");
    let o = sb.run(&["api", "{ a }"], Some(&mock), ENV);
    assert_eq!(stdout(&o), "{\n  \"a\": 1\n}\n");
}

#[test]
fn workspace_flag_selects_the_credentials() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#)]);
    let o = sb.run(&["api", "{ a }", "-w", "other"], Some(&mock), ENV);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.requests()[0].authorization.as_deref(), Some(KEY_OTHER));
}

#[test]
fn variables_come_from_flags_and_a_file_with_flags_winning() {
    let sb = sandbox_with_workspaces();
    let file = sb.cwd().join("vars.json");
    std::fs::write(&file, r#"{"id":"from-file","first":1,"keep":true}"#).unwrap();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#)]);

    let o = sb.run(
        &[
            "api",
            "query Q($id: String!, $first: Int) { a }",
            "--variables-file",
            file.to_str().unwrap(),
            "--var",
            "id=KK-1",
            "--var",
            "text=a=b",
            "--var-json",
            "first=5",
            "--var-json",
            "tags=[\"x\",null]",
            "--operation-name",
            "Q",
        ],
        Some(&mock),
        ENV,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let body = sent(&mock);
    assert_eq!(
        body["variables"],
        json!({"id": "KK-1", "text": "a=b", "first": 5, "keep": true, "tags": ["x", null]})
    );
    assert_eq!(body["operationName"], "Q");
}

#[test]
fn a_var_stays_a_string_even_when_it_looks_like_a_number() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#)]);
    let o = sb.run(&["api", "{ a }", "--var", "n=123"], Some(&mock), ENV);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(sent(&mock)["variables"], json!({"n": "123"}));
}

#[test]
fn the_document_can_come_from_stdin_or_a_file() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#), ok(r#"{"data":{"a":1}}"#)]);
    let o = sb.run_stdin(&["api", "-"], Some(&mock), ENV, Some("{ fromStdin }"));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let file = sb.cwd().join("q.graphql");
    std::fs::write(&file, "{ fromFile }").unwrap();
    let o = sb.run(
        &["api", "--query-file", file.to_str().unwrap()],
        Some(&mock),
        ENV,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let reqs = mock.requests();
    assert!(reqs[0].body.contains("fromStdin"));
    assert!(reqs[1].body.contains("fromFile"));
}

#[test]
fn mutations_are_refused_with_exit_4_before_anything_is_sent() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"x":1}}"#)]);

    for doc in [
        "mutation { issueDelete(id: \"x\") { success } }",
        "query Q { viewer { id } }\nmutation M { issueDelete(id: \"x\") { success } }",
    ] {
        let o = sb.run(&["api", doc, "--json"], Some(&mock), ENV);
        assert_eq!(code(&o), 4, "{doc}");
        assert_eq!(stdout(&o), "");
        let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
        assert_eq!(e["error"]["code"], "write_denied");
        assert!(e["error"]["message"].as_str().unwrap().contains("mutation"));
    }

    // `--mutation` is reserved: it does not unlock anything yet.
    for doc in [
        "mutation { issueDelete(id: \"x\") { success } }",
        "{ viewer { id } }",
    ] {
        let o = sb.run(&["api", doc, "--mutation"], Some(&mock), ENV);
        assert_eq!(code(&o), 4, "{doc}");
        assert!(stderr(&o).contains("allow_raw_mutation"));
    }
    assert!(mock.requests().is_empty(), "nothing may reach Linear");
}

#[test]
fn a_mutation_cannot_hide_in_a_comment_or_string_trick() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#)]);
    let hidden =
        "# query only\nquery Q { a(s: \"}\") }\nmutation { issueDelete(id: \"x\") { success } }";
    let o = sb.run(&["api", hidden], Some(&mock), ENV);
    assert_eq!(code(&o), 4);
    assert!(mock.requests().is_empty());

    // The reverse: a keyword inside a string is just a query.
    let o = sb.run(&["api", "{ a(s: \"mutation { x }\") }"], Some(&mock), ENV);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn bad_input_is_a_usage_error() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![]);
    let file = sb.cwd().join("array.json");
    std::fs::write(&file, "[1]").unwrap();

    let cases: Vec<Vec<&str>> = vec![
        vec!["api", "subscription { a }"],
        vec!["api", "not graphql"],
        vec!["api", "{ a", "--json"],
        vec!["api", "{ a }", "--var", "novalue"],
        vec!["api", "{ a }", "--var", "=x"],
        vec!["api", "{ a }", "--var-json", "n={"],
        vec!["api", "{ a }", "--variables-file", file.to_str().unwrap()],
        vec!["api", "{ a }", "--variables-file", "/nonexistent/vars.json"],
        vec!["api"],
        vec!["api", "{ a }", "--query-file", "x.graphql"],
    ];
    for args in cases {
        let o = sb.run(&args, Some(&mock), ENV);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    let o = sb.run_stdin(
        &["api", "-", "--variables-file", "-"],
        Some(&mock),
        ENV,
        Some("{ a }"),
    );
    assert_eq!(code(&o), 2);
    assert!(mock.requests().is_empty());
}

#[test]
fn api_errors_exit_1_and_auth_problems_exit_3() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![
        ok(r#"{"errors":[{"message":"Cannot query field \"nope\" on type \"Query\"."}]}"#),
        Reply {
            status: 401,
            body: r#"{"errors":[{"message":"bad key","extensions":{"type":"authentication error","code":"AUTHENTICATION_ERROR"}}]}"#.into(),
        },
    ]);

    let o = sb.run(&["api", "{ nope }", "--json"], Some(&mock), ENV);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");
    let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert!(e["error"]["message"].as_str().unwrap().contains("nope"));

    let o = sb.run(&["api", "{ viewer { id } }"], Some(&mock), ENV);
    assert_eq!(code(&o), 3);
}

#[test]
fn missing_credentials_exit_3_without_sending() {
    let sb = sandbox_with_workspaces();
    let mock = Mock::start(vec![]);
    let o = sb.run(&["api", "{ a }"], Some(&mock), &[]);
    assert_eq!(code(&o), 3);
    assert!(mock.requests().is_empty());
}
