//! `linear issue search` against a mock Linear server.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

/// The `issue_search` fixture with its page info replaced.
fn page(has_next: bool, cursor: &str) -> String {
    let mut v: Value = serde_json::from_str(&fixture("issue_search")).unwrap();
    v["data"]["searchIssues"]["pageInfo"] = json!({"hasNextPage": has_next, "endCursor": cursor});
    v.to_string()
}

#[test]
fn search_prints_the_table_of_issue_list_and_sends_the_term() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
    let o = linear(&sb, &mock, &["issue", "search", "fixture"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("ID"), "{out}");
    assert!(
        out.contains("EX-23") && out.contains("In Progress") && out.contains("Fixture Project"),
        "{out}"
    );
    assert_no_leak(&o);

    let req = request(&mock, 0);
    assert_eq!(req["operationName"], "IssueSearch");
    assert_eq!(req["variables"]["term"], "fixture");
    assert_eq!(req["variables"]["first"], 50);
    // No filter and no comments: Linear's defaults, not an empty object or `false`.
    assert_eq!(req["variables"]["filter"], Value::Null);
    assert_eq!(req["variables"]["includeComments"], Value::Null);
}

#[test]
fn several_words_are_searched_together_quoted_or_not() {
    let sb = workspace();
    for args in [
        &["issue", "search", "duplicate", "check"][..],
        &["issue", "search", "duplicate check"],
    ] {
        let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
        let o = linear(&sb, &mock, args);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(request(&mock, 0)["variables"]["term"], "duplicate check");
    }
}

#[test]
fn search_json_is_the_shape_of_issue_list_json() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("issue_search")),
        ok(&fixture("issue_list")),
    ]);
    let o = linear(&sb, &mock, &["issue", "search", "x", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let found = stdout_json(&o);
    let o = linear(&sb, &mock, &["issue", "list", "--json"]);
    assert_eq!(found, stdout_json(&o));
    assert_eq!(found[0]["workspace"], "example");
    assert_eq!(found[0]["sourceUrl"], "https://example.com/source/1");
}

#[test]
fn search_quiet_prints_identifiers_only() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
    let o = linear(&sb, &mock, &["issue", "search", "x", "--quiet"]);
    assert_eq!(stdout(&o), "EX-23\n");
}

#[test]
fn nothing_found_says_so_and_json_is_an_empty_array() {
    let sb = workspace();
    let empty = r#"{"data":{"searchIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(empty), ok(empty)]);
    let o = linear(&sb, &mock, &["issue", "search", "nothing"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o).trim(), "No issues found.");
    let o = linear(&sb, &mock, &["issue", "search", "nothing", "--json"]);
    assert_eq!(stdout(&o).trim(), "[]");
}

#[test]
fn team_state_and_project_narrow_the_search() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("issue_search"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "search",
            "x",
            "--team",
            "EX",
            "--project",
            "fixture project",
            "--state",
            "Todo",
            "--state-type",
            "unstarted,started",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // The project is resolved first, then the one search carries the whole filter.
    assert_eq!(request(&mock, 0)["operationName"], "ProjectRefs");
    let search = request(&mock, 1);
    assert_eq!(search["operationName"], "IssueSearch");
    assert_eq!(
        search["variables"]["filter"],
        json!({
            "team": {"key": {"eqIgnoreCase": "EX"}},
            "project": {"id": {"eq": "00000000-0000-4000-8000-000000000006"}},
            "state": {"type": {"in": ["unstarted", "started"]}},
            "or": [{"state": {"name": {"eqIgnoreCase": "Todo"}}}],
        })
    );
}

#[test]
fn open_leaves_out_the_closed_states() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
    let o = linear(&sb, &mock, &["issue", "search", "x", "--open", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        request(&mock, 0)["variables"]["filter"],
        json!({"state": {"type": {"nin": ["completed", "canceled"]}}})
    );
}

#[test]
fn comments_asks_linear_to_search_them() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
    let o = linear(
        &sb,
        &mock,
        &["issue", "search", "x", "--comments", "--quiet"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(request(&mock, 0)["variables"]["includeComments"], true);
}

#[test]
fn an_unknown_project_fails_before_the_search_is_sent() {
    let sb = workspace();
    let refs =
        r#"{"data":{"projects":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(refs)]);
    let o = linear(&sb, &mock, &["issue", "search", "x", "--project", "nope"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn all_follows_the_cursor_and_limit_stops_early() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&page(true, "cursor-1")),
        ok(&page(false, "cursor-2")),
    ]);
    let o = linear(&sb, &mock, &["issue", "search", "x", "--all", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "EX-23\nEX-23\n");
    assert_eq!(request(&mock, 0)["variables"]["after"], Value::Null);
    assert_eq!(request(&mock, 1)["variables"]["after"], "cursor-1");

    let mock = Mock::start(vec![ok(&page(true, "cursor-1"))]);
    let o = linear(
        &sb,
        &mock,
        &["issue", "search", "x", "--limit", "1", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).contains("more exist"), "{}", stderr(&o));
    assert_eq!(request(&mock, 0)["variables"]["first"], 1);
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn an_empty_query_or_a_conflicting_flag_is_a_usage_error_before_any_request() {
    let sb = workspace();
    for args in [
        &["issue", "search"][..],
        &["issue", "search", ""],
        &["issue", "search", "   "],
        &["issue", "search", "x", "--open", "--state-type", "started"],
        &["issue", "search", "x", "--state-type", "doing"],
    ] {
        let mock = Mock::start(vec![ok(&fixture("issue_search"))]);
        let o = linear(&sb, &mock, args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(mock.requests().is_empty(), "{args:?}: a request was sent");
    }
}

#[test]
fn an_empty_page_that_claims_more_without_a_cursor_is_the_end() {
    // What Linear answers a filtered search that matches nothing.
    let sb = workspace();
    let odd = r#"{"data":{"searchIssues":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(odd)]);
    let o = linear(
        &sb,
        &mock,
        &["issue", "search", "x", "--state-type", "completed"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "No issues found.");
    assert_eq!(mock.requests().len(), 1);
}
