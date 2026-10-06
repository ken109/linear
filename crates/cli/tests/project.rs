//! `linear project list|view` against a mock Linear server.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

#[test]
fn list_prints_lead_status_target_counts_and_the_latest_update() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("projects"))]);
    let o = linear(&sb, &mock, &["project", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "SLUG",
        "aaaaaaaaaaaa",
        "Fixture Project",
        "In Progress",
        "Alice Example",
        "1/2",
        "2026-10-06 onTrack",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let req = request(&mock, 0);
    assert_eq!(req["operationName"], "ProjectList");
    // The page stays small: a project selects up to 100 issues and 50 milestones.
    assert_eq!(req["variables"]["first"], 10);
    assert_eq!(req["variables"]["filter"], Value::Null);
}

#[test]
fn open_lead_and_initiative_become_one_filter() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("projects"))]);
    let o = linear(
        &sb,
        &mock,
        &[
            "project",
            "list",
            "--open",
            "--lead",
            "me",
            "--initiative",
            "Human Sim",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "aaaaaaaaaaaa\n");
    assert_eq!(
        request(&mock, 0)["variables"]["filter"],
        json!({
            "status": {"type": {"nin": ["completed", "canceled"]}},
            "lead": {"isMe": {"eq": true}},
            "initiatives": {"some": {"name": {"eqIgnoreCase": "Human Sim"}}}
        })
    );
}

#[test]
fn list_json_carries_the_workspace_and_issue_counts() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("projects"))]);
    let o = linear(&sb, &mock, &["project", "list", "--json"]);
    let v = stdout_json(&o);
    let p = &v[0];
    assert_eq!(p["workspace"], "example");
    assert_eq!(p["lead"]["name"], "Alice Example");
    assert_eq!(p["lastUpdate"]["health"], "onTrack");
    assert_eq!(p["issueCounts"]["started"], 1);
    assert_eq!(p["issueCounts"]["completed"], 1);
    assert_eq!(p["issueCounts"]["complete"], true);
}

#[test]
fn view_resolves_the_project_then_shows_updates_newest_first() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("project_view"))]);
    let o = linear(&sb, &mock, &["project", "view", "fixture project"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Fixture Project  (aaaaaaaaaaaa)",
        "In Progress (started)",
        "Alice Example",
        "Fixture Initiative",
        "Issues:",
        "2 total (1 started",
        "A project used to capture test fixtures.",
        "[ ] Milestone 1 (2026-11-15, 25%, next)",
        "Latest status update (2026-10-06, atRisk, Alice Example)",
        "  Second status update.",
        "Earlier status updates",
        "2026-10-06 onTrack: On track.",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    // The latest update is not repeated among the earlier ones.
    assert_eq!(out.matches("Second status update.").count(), 1, "{out}");
    // Content is long; it is opt-in.
    assert!(!out.contains("Keep the fixtures honest."));

    assert_eq!(request(&mock, 0)["operationName"], "ProjectRefs");
    let req = request(&mock, 1);
    assert_eq!(req["operationName"], "ProjectView");
    assert_eq!(
        req["variables"]["id"],
        "00000000-0000-4000-8000-000000000006"
    );
}

#[test]
fn view_content_flag_prints_the_content_document() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("project_view"))]);
    let o = linear(
        &sb,
        &mock,
        &["project", "view", "aaaaaaaaaaaa", "--content"],
    );
    assert!(
        stdout(&o).contains("  Keep the fixtures honest."),
        "{}",
        stdout(&o)
    );
}

#[test]
fn view_json_has_description_content_and_updates() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("project_view"))]);
    let o = linear(&sb, &mock, &["project", "view", "aaaaaaaaaaaa", "--json"]);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["slugId"], "aaaaaaaaaaaa");
    assert_eq!(v["description"], "A project used to capture test fixtures.");
    assert_eq!(v["content"], "## Goal\n\nKeep the fixtures honest.");
    assert_eq!(v["projectUpdates"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(v["lastUpdate"]["health"], "atRisk");
}

#[test]
fn view_of_a_project_without_updates_says_so() {
    let sb = workspace();
    let mut v: Value = serde_json::from_str(&fixture("project_view")).unwrap();
    v["data"]["project"]["lastUpdate"] = Value::Null;
    v["data"]["detail"]["projectUpdates"] = json!({"nodes": []});
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&v.to_string())]);
    let o = linear(&sb, &mock, &["project", "view", "aaaaaaaaaaaa"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("Latest status update: none"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn an_unknown_project_is_a_usage_error_listing_the_known_ones() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS)]);
    let o = linear(&sb, &mock, &["project", "view", "Nope"]);
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("available: Fixture Project"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn open_and_status_type_cannot_be_combined() {
    let sb = workspace();
    let o = sb.run(
        &["project", "list", "--open", "--status-type", "started"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
}
