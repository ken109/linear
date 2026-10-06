//! `linear issue create --held-on` against a mock Linear.
//!
//! The cycles come from `crates/core/tests/fixtures/cycles.json` (#41 starts
//! 2026-10-05T15:00Z, #42 a week later), so a meeting on 2026-10-05 goes into
//! #41 and one on 2026-10-30 has no cycle.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

const SOURCE: &str = "https://example.com/source/1";
const NO_CYCLE_DAY: &str = "2026-10-30";

fn create_with_cycle(extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes = create_routes(vec![("CycleList", vec![cycles()])]);
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

fn args<'a>(held_on: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec![
        "issue",
        "create",
        "--title",
        "Write the thing",
        "--project",
        "Fixture Project",
        "--source",
        SOURCE,
        "--held-on",
        held_on,
    ];
    a.extend_from_slice(extra);
    a
}

#[test]
fn a_new_issue_is_created_in_the_cycle_after_the_meeting() {
    let sb = workspace();
    let mock = Routed::start(create_with_cycle(vec![]));
    let o = run(&sb, &mock, &args(MEETING, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], false);
    assert_eq!(v["identifier"], "EX-30");
    assert_eq!(v["cycle"]["id"], CYCLE_41);
    assert_eq!(v["cycle"]["number"], 41);
    assert_eq!(v["changed"], json!([]), "nothing existing was changed");

    // The cycle is looked up before anything is written, and rides on the create itself.
    let ops = mock.ops();
    let lookup = ops.iter().position(|o| o == "CycleList").unwrap();
    let create = ops.iter().position(|o| o == "IssueCreate").unwrap();
    assert!(lookup < create, "{ops:?}");
    assert_eq!(mock.of("IssueCreate")[0]["input"]["cycleId"], CYCLE_41);
    // No separate update, and the cycles asked for are the team's.
    assert!(!ops.contains(&"IssueUpdate".to_owned()), "{ops:?}");
    assert_eq!(
        mock.of("CycleList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
}

#[test]
fn the_cycle_is_mentioned_in_the_text_output() {
    let sb = workspace();
    let mock = Routed::start(create_with_cycle(vec![]));
    let o = run(&sb, &mock, &args(MEETING, &[]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("created; in cycle #41"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn without_held_on_no_cycle_is_asked_for_or_sent() {
    let sb = workspace();
    let mock = Routed::start(create_with_cycle(vec![]));
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
            "--source",
            SOURCE,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(mock.of("CycleList").is_empty());
    assert!(mock.of("IssueCreate")[0]["input"].get("cycleId").is_none());
    let v = stdout_json(&o);
    assert!(v.get("cycle").is_none(), "{v}");
}

#[test]
fn a_meeting_without_a_cycle_stops_before_anything_is_written() {
    let sb = workspace();
    let mock = Routed::start(create_with_cycle(vec![]));
    let o = run(&sb, &mock, &args(NO_CYCLE_DAY, &[]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("no cycle of team EX contains 2026-10-31"),
        "{err}"
    );
    assert!(
        err.contains("#41"),
        "the cycles that exist are listed: {err}"
    );
    mock.assert_read_only();
    assert_eq!(stdout(&o), "");
}

#[test]
fn held_on_dates_are_checked_strictly_before_any_request() {
    let sb = workspace();
    for bad in ["2026-02-30", "2026-13-01", "tuesday"] {
        let mock = Routed::start(create_with_cycle(vec![]));
        let o = run(&sb, &mock, &args(bad, &[]));
        assert_eq!(code(&o), 2, "{bad}: {}", stderr(&o));
        assert!(mock.ops().is_empty(), "{bad}: a request was sent");
    }
}

/// An issue with the same source is only looked for when the `source-attachment` rule is on,
/// so these tests turn it on.
fn existing_routes(view: View) -> Vec<(&'static str, Vec<Reply>)> {
    create_with_cycle(vec![
        ("AttachmentsForUrlQuery", vec![issue_with_source()]),
        ("IssueWriteView", vec![view.reply()]),
        ("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]),
    ])
}

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

#[test]
fn an_existing_issue_without_a_cycle_gets_the_cycle_and_nothing_else() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing_routes(mine().in_cycle(None)));
    let o = run(&sb, &mock, &args(MEETING, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["changed"], json!(["cycle"]));
    assert_eq!(v["cycle"]["id"], CYCLE_41);

    // One update, carrying the cycle only: the description and the labels are not touched.
    assert_eq!(mock.of("IssueCreate").len(), 0, "nothing is created");
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({ "id": "id-EX-23", "input": { "cycleId": CYCLE_41 } })]
    );
    assert!(mock.of("AttachmentCreate").is_empty());
}

#[test]
fn an_existing_issue_without_a_cycle_says_so_in_text() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing_routes(mine().in_cycle(None)));
    let o = run(&sb, &mock, &args(MEETING, &[]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("already exists, nothing created; cycle set to #41"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn an_existing_issue_that_has_the_cycle_is_left_alone() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing_routes(mine().in_cycle(Some(CYCLE_41))));
    let o = run(&sb, &mock, &args(MEETING, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["changed"], json!([]));
    assert_eq!(v["cycle"]["id"], CYCLE_41);
    mock.assert_read_only();
}

#[test]
fn an_existing_issue_in_another_cycle_keeps_it_and_the_run_says_so() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing_routes(mine().in_cycle(Some(CYCLE_42))));
    let o = run(&sb, &mock, &args(MEETING, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["changed"], json!([]));
    assert_eq!(
        v["cycle"]["id"], CYCLE_42,
        "the cycle it is in, not the one asked for"
    );
    mock.assert_read_only();

    // Not with --json, where stderr stays quiet; in text mode the note is on stderr.
    let mock = Routed::start(existing_routes(mine().in_cycle(Some(CYCLE_42))));
    let o = run(&sb, &mock, &args(MEETING, &[]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("EX-23 is already in cycle #42; it was not moved to #41"),
        "{}",
        stderr(&o)
    );
    mock.assert_read_only();
}

#[test]
fn an_existing_issue_is_not_looked_at_again_without_held_on() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing_routes(mine().in_cycle(None)));
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
            "--source",
            SOURCE,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(mock.of("IssueWriteView").is_empty());
    assert!(mock.of("CycleList").is_empty());
    mock.assert_read_only();
}

#[test]
fn aligning_the_cycle_of_somebody_elses_issue_is_refused_by_the_ownership_rules() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let foreign = view("EX-23")
        .assigned_to(Some(BOT))
        .in_project(OTHER_PROJECT, Some(BOT))
        .in_cycle(None);
    let mock = Routed::start(existing_routes(foreign));
    let o = run(&sb, &mock, &args(MEETING, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_failed_cycle_update_on_an_existing_issue_is_an_error() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mut routes = existing_routes(mine().in_cycle(None));
    routes.retain(|(o, _)| *o != "IssueUpdate");
    routes.push(("IssueUpdate", vec![graphql_error("cycle refused")]));
    let mock = Routed::start(routes);
    let o = run(&sb, &mock, &args(MEETING, &[]));
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("cycle refused"), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
}
