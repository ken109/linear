//! `linear issue list|view` against a mock Linear server.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

/// The `issue_list` fixture with its page info replaced.
fn list_page(has_next: bool, cursor: &str) -> String {
    let mut v: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    v["data"]["issues"]["pageInfo"] = json!({"hasNextPage": has_next, "endCursor": cursor});
    v.to_string()
}

#[test]
fn list_prints_a_table_and_sends_the_filter() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "list",
            "--assignee",
            "me",
            "--state-type",
            "started",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("ID"), "{out}");
    assert!(
        out.contains("EX-23") && out.contains("In Progress"),
        "{out}"
    );
    assert!(
        out.contains("Alice Example") && out.contains("Fixture Project"),
        "{out}"
    );
    assert_no_leak(&o);

    let req = request(&mock, 0);
    assert_eq!(req["operationName"], "IssueList");
    assert_eq!(
        req["variables"]["filter"],
        json!({"assignee": {"isMe": {"eq": true}}, "state": {"type": {"in": ["started"]}}})
    );
    // The first page is no larger than the limit asks for, and never above 50.
    assert_eq!(req["variables"]["first"], 50);
    assert_eq!(
        mock.requests()[0].authorization.as_deref(),
        Some(KEY),
        "the key is sent as the Authorization header"
    );
}

#[test]
fn list_json_is_linears_shape_plus_the_workspace_and_the_origin_url() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(&sb, &mock, &["issue", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v.as_array().unwrap().len(), 1);
    let issue = &v[0];
    assert_eq!(issue["workspace"], "example");
    assert_eq!(issue["identifier"], "EX-23");
    assert_eq!(issue["state"]["type"], "started");
    assert_eq!(issue["assignee"]["isMe"], true);
    assert_eq!(issue["labels"]["nodes"][0]["parent"]["name"], "area");
    assert_eq!(issue["sourceUrl"], "https://example.com/source/1");
    // No filter given: Linear gets a null filter, not an empty object.
    assert_eq!(request(&mock, 0)["variables"]["filter"], Value::Null);
}

#[test]
fn list_quiet_prints_identifiers_only() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(&sb, &mock, &["issue", "list", "--quiet"]);
    assert_eq!(stdout(&o), "EX-23\n");
}

#[test]
fn an_empty_list_says_so_and_json_is_an_empty_array() {
    let sb = workspace();
    let empty =
        r#"{"data":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(empty), ok(empty)]);
    let o = linear(&sb, &mock, &["issue", "list"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o).trim(), "No issues found.");
    let o = linear(&sb, &mock, &["issue", "list", "--json"]);
    assert_eq!(stdout(&o).trim(), "[]");
}

#[test]
fn all_follows_the_cursor_to_the_last_page() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&list_page(true, "cursor-1")),
        ok(&list_page(false, "cursor-2")),
    ]);
    let o = linear(&sb, &mock, &["issue", "list", "--all", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "EX-23\nEX-23\n");
    assert_eq!(mock.requests().len(), 2);
    assert_eq!(request(&mock, 0)["variables"]["after"], Value::Null);
    assert_eq!(request(&mock, 1)["variables"]["after"], "cursor-1");
}

#[test]
fn limit_stops_early_and_says_that_more_exist_on_stderr() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&list_page(true, "cursor-1"))]);
    let o = linear(&sb, &mock, &["issue", "list", "--limit", "1", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o).as_array().unwrap().len(), 1);
    assert!(stderr(&o).contains("more exist"), "{}", stderr(&o));
    // The page asked for is no bigger than the limit, and no second page is fetched.
    assert_eq!(request(&mock, 0)["variables"]["first"], 1);
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn a_complete_listing_does_not_warn() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(&sb, &mock, &["issue", "list", "--limit", "1"]);
    assert_eq!(stderr(&o), "");
}

#[test]
fn project_names_are_resolved_before_filtering() {
    let sb = workspace();
    let refs = r#"{"data":{"projects":{"nodes":[
        {"id":"00000000-0000-4000-8000-000000000006","slugId":"aaaaaaaaaaaa","name":"Fixture Project","url":"https://linear.app/example/project/fixture-project-aaaaaaaaaaaa"}
    ],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(refs), ok(&fixture("issue_list"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "list",
            "--project",
            "fixture project",
            "--open",
            "--label",
            "Bug",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(request(&mock, 0)["operationName"], "ProjectRefs");
    let filter = &request(&mock, 1)["variables"]["filter"];
    assert_eq!(
        filter["project"],
        json!({"id": {"eq": "00000000-0000-4000-8000-000000000006"}})
    );
    assert_eq!(
        filter["state"]["type"]["nin"],
        json!(["completed", "canceled"])
    );
    assert_eq!(filter["labels"]["some"]["name"]["eqIgnoreCase"], "Bug");
}

#[test]
fn an_unknown_project_fails_before_any_issue_is_listed() {
    let sb = workspace();
    let refs =
        r#"{"data":{"projects":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(refs)]);
    let o = linear(&sb, &mock, &["issue", "list", "--project", "nope"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("no project \"nope\""), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn open_and_state_type_cannot_be_combined() {
    let sb = workspace();
    let o = sb.run(
        &["issue", "list", "--open", "--state-type", "started"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
}

#[test]
fn an_unknown_state_type_is_a_usage_error() {
    let sb = workspace();
    let o = sb.run(
        &["issue", "list", "--state-type", "started,doing"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("doing"), "{}", stderr(&o));
}

#[test]
fn view_shows_the_description_and_comments() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_view"))]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-23"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "EX-23  Write the fixture issue",
        "State:",
        "In Progress (started)",
        "area/api",
        "Milestone 1 (2026-11-15)",
        "High",
        "https://example.com/source/1",
        "Body of the fixture issue.",
        "Comments (1)",
        "Alice Example, 2026-10-06:",
        "    A fixture comment.",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let req = request(&mock, 0);
    assert_eq!(req["operationName"], "IssueView");
    assert_eq!(req["variables"]["id"], "EX-23");
}

#[test]
fn view_json_merges_the_detail_into_the_issue() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_view"))]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-23", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["description"], "Body of the fixture issue.");
    assert_eq!(v["priorityLabel"], "High");
    assert_eq!(v["comments"]["nodes"][0]["body"], "A fixture comment.");
    assert_eq!(v["sourceUrl"], "https://example.com/source/1");
}

#[test]
fn a_missing_issue_is_an_error_and_prints_nothing_on_stdout() {
    let sb = workspace();
    let body = r#"{"errors":[{"message":"Entity not found: Issue","extensions":{"code":"INPUT_ERROR","userPresentableMessage":"Could not find referenced Issue."}}],"data":null}"#;
    let mock = Mock::start(vec![Reply {
        status: 200,
        body: body.into(),
    }]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-999", "--json"]);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");
    let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Entity not found"));
}

#[test]
fn without_credentials_the_command_fails_with_the_auth_exit_code() {
    let sb = workspace();
    let mock = Mock::start(vec![]);
    let o = sb.run(&["issue", "list"], Some(&mock), &[]);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("workspace login"), "{}", stderr(&o));
    assert_eq!(mock.requests().len(), 0);
}
