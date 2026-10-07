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
        "Relations\n  blocks  EX-24  Ship the fixture  (Backlog)\n  related to  EX-22  Prepare the fixture  (In Progress)",
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
    // Linear's own shape: the relations that start here, and the ones that point here.
    let blocks = &v["relations"]["nodes"][0];
    assert_eq!(blocks["type"], "blocks");
    assert_eq!(blocks["issue"]["identifier"], "EX-23");
    assert_eq!(blocks["relatedIssue"]["identifier"], "EX-24");
    assert_eq!(blocks["relatedIssue"]["state"]["name"], "Backlog");
    let related = &v["inverseRelations"]["nodes"][0];
    assert_eq!(related["type"], "related");
    assert_eq!(related["issue"]["identifier"], "EX-22");
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

// ------------------------------------------------------------------ --completed-since

#[test]
fn completed_since_days_lists_the_issues_closed_in_that_window() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(&sb, &mock, &["issue", "list", "--completed-since", "14d"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    let filter = request(&mock, 0)["variables"]["filter"].clone();
    let alternatives = &filter["and"][0]["or"];
    assert_eq!(alternatives.as_array().unwrap().len(), 2, "{filter}");
    let from = |i: usize, field: &str| {
        chrono::DateTime::parse_from_rfc3339(alternatives[i][field]["gte"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let expected = chrono::Utc::now() - chrono::Duration::days(14);
    for (i, field) in [(0, "completedAt"), (1, "canceledAt")] {
        let off = (from(i, field) - expected).num_seconds().abs();
        assert!(off < 120, "{field}: {off}s from 14 days ago");
    }
    // On its own it adds no state filter: closed issues are the whole point.
    assert!(filter.get("state").is_none(), "{filter}");
}

#[test]
fn completed_since_a_date_starts_at_midnight_utc() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "list",
            "--completed-since",
            "2026-10-01",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        request(&mock, 0)["variables"]["filter"],
        json!({"and": [{"or": [
            {"completedAt": {"gte": "2026-10-01T00:00:00Z"}},
            {"canceledAt": {"gte": "2026-10-01T00:00:00Z"}},
        ]}]})
    );
}

#[test]
fn open_with_completed_since_is_the_open_issues_plus_the_recently_closed_ones() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "list",
            "--open",
            "--completed-since",
            "2026-10-01",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        request(&mock, 0)["variables"]["filter"],
        json!({"and": [{"or": [
            {"state": {"type": {"nin": ["completed", "canceled"]}}},
            {"completedAt": {"gte": "2026-10-01T00:00:00Z"}},
            {"canceledAt": {"gte": "2026-10-01T00:00:00Z"}},
        ]}]})
    );
}

#[test]
fn completed_since_narrows_with_the_other_filters() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue",
            "list",
            "--state-type",
            "completed",
            "--assignee",
            "me",
            "--completed-since",
            "2026-10-01",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let filter = &request(&mock, 0)["variables"]["filter"];
    assert_eq!(filter["state"], json!({"type": {"in": ["completed"]}}));
    assert_eq!(filter["assignee"], json!({"isMe": {"eq": true}}));
    assert_eq!(filter["and"][0]["or"].as_array().unwrap().len(), 2);
}

#[test]
fn a_bad_completed_since_is_a_usage_error_before_any_request() {
    let sb = workspace();
    for bad in ["0d", "14", "soon", "2026-02-30"] {
        let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
        let o = linear(&sb, &mock, &["issue", "list", "--completed-since", bad]);
        assert_eq!(code(&o), 2, "{bad}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("expected a number of days such as 14d"),
            "{bad}: {}",
            stderr(&o)
        );
        assert_eq!(mock.requests().len(), 0, "{bad}: a request was sent");
    }
}

#[test]
fn completed_since_cannot_be_read_from_the_cache() {
    let sb = workspace();
    let o = sb.run(
        &["issue", "list", "--cached", "--completed-since", "14d"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--completed-since"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ --order

/// A page of the listed issues: each `(identifier, sortOrder)` becomes the fixture issue with those.
fn page_of(rows: &[(&str, f64)], has_next: bool, cursor: &str) -> String {
    let mut v: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    let template = v["data"]["issues"]["nodes"][0].clone();
    let nodes: Vec<Value> = rows
        .iter()
        .map(|(id, sort)| {
            let mut n = template.clone();
            n["identifier"] = json!(id);
            n["id"] = json!(format!("id-{id}"));
            n["sortOrder"] = json!(sort);
            n
        })
        .collect();
    v["data"]["issues"]["nodes"] = json!(nodes);
    v["data"]["issues"]["pageInfo"] = json!({"hasNextPage": has_next, "endCursor": cursor});
    v.to_string()
}

#[test]
fn the_default_order_is_the_one_linear_returns() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&page_of(
        &[("EX-1", 30.0), ("EX-2", -5.0), ("EX-3", 10.0)],
        false,
        "c",
    ))]);
    let o = linear(&sb, &mock, &["issue", "list", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "EX-1\nEX-2\nEX-3\n");
}

#[test]
fn manual_order_sorts_by_sort_order_across_pages() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&page_of(&[("EX-1", 30.0), ("EX-2", -5.0)], true, "c1")),
        ok(&page_of(&[("EX-3", -1003.5), ("EX-4", 10.0)], false, "c2")),
    ]);
    let o = linear(
        &sb,
        &mock,
        &["issue", "list", "--order", "manual", "--quiet"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // Ascending, top first, whichever page an issue came from.
    assert_eq!(stdout(&o), "EX-3\nEX-2\nEX-4\nEX-1\n");
    assert_eq!(mock.requests().len(), 2, "both pages are fetched");
    assert_eq!(stderr(&o), "");
}

#[test]
fn manual_order_fetches_everything_before_it_applies_the_limit() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&page_of(&[("EX-1", 30.0), ("EX-2", -5.0)], true, "c1")),
        ok(&page_of(&[("EX-3", -1003.5), ("EX-4", 10.0)], false, "c2")),
    ]);
    let o = linear(
        &sb,
        &mock,
        &[
            "issue", "list", "--order", "manual", "--limit", "2", "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // The first two of the sorted list, not of the first page.
    assert_eq!(stdout(&o), "EX-3\nEX-2\n");
    assert_eq!(mock.requests().len(), 2);
    assert!(stderr(&o).contains("more exist"), "{}", stderr(&o));
}

#[test]
fn manual_order_keeps_linears_order_between_equal_values() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&page_of(
        &[("EX-1", 1.0), ("EX-2", 1.0), ("EX-3", 0.0)],
        false,
        "c",
    ))]);
    let o = linear(
        &sb,
        &mock,
        &["issue", "list", "--order", "manual", "--quiet"],
    );
    assert_eq!(stdout(&o), "EX-3\nEX-1\nEX-2\n");
}

#[test]
fn an_unknown_order_is_a_usage_error() {
    let sb = workspace();
    let o = sb.run(&["issue", "list", "--order", "priority"], None, &[]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("manual"), "{}", stderr(&o));
}
