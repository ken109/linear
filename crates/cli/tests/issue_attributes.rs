//! `linear issue create|update --priority|--estimate|--parent|--cycle` against a mock Linear
//! that answers by operation name: the same path every write takes (guard, validators,
//! mutation, rollback), and the rule that only what differs from now is sent.
//!
//! The cycles come from `crates/core/tests/fixtures/cycles.json` (#41 and #42 of team EX).

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

const SOURCE: &str = "https://example.com/source/1";
const NEW_SOURCE: &str = "https://example.com/source/2";

/// The fixture issue (EX-23): mine, in a project I lead; priority 3 (medium), estimate 3,
/// no parent, no cycle.
fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn update_routes(view: View, extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes = vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("IssueById", vec![issue_by_id("EX-1")]),
        ("CycleList", vec![cycles()]),
        ("Templates", vec![templates()]),
        ("AttachmentsForUrlQuery", vec![no_issue_with_source()]),
        ("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]),
        ("AttachmentCreate", vec![attachment_ok()]),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

fn update<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["issue", "update", "EX-23"];
    args.extend_from_slice(extra);
    args
}

fn create<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "issue",
        "create",
        "--title",
        "Write the thing",
        "--project",
        "Fixture Project",
        "--source",
        SOURCE,
    ];
    args.extend_from_slice(extra);
    args
}

fn create_with(extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes = create_routes(vec![
        ("CycleList", vec![cycles()]),
        ("IssueById", vec![issue_by_id("EX-1")]),
    ]);
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

// ------------------------------------------------------------------ create

#[test]
fn create_sends_the_priority_estimate_parent_and_cycle() {
    let sb = workspace();
    let mock = Routed::start(create_with(vec![]));
    let o = run(
        &sb,
        &mock,
        &create(&[
            "--priority",
            "High",
            "--estimate",
            "5",
            "--parent",
            "EX-1",
            "--cycle",
            "#41",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], false);
    assert_eq!(v["cycle"]["id"], CYCLE_41);

    let input = &mock.of("IssueCreate")[0]["input"];
    assert_eq!(input["priority"], 2);
    assert_eq!(input["estimate"], 5);
    assert_eq!(input["parentId"], "id-EX-1");
    assert_eq!(input["cycleId"], CYCLE_41);

    // Everything is looked up before anything is written; the cycle is the team's.
    let ops = mock.ops();
    let create_at = ops.iter().position(|o| o == "IssueCreate").unwrap();
    assert!(ops.iter().position(|o| o == "CycleList").unwrap() < create_at);
    assert!(ops.iter().position(|o| o == "IssueById").unwrap() < create_at);
    assert_eq!(
        mock.of("CycleList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
    assert_eq!(mock.of("IssueById")[0]["id"], "EX-1");
}

#[test]
fn priority_none_is_sent_as_zero_and_a_number_is_accepted() {
    let sb = workspace();
    for (spec, sent) in [("none", 0), ("0", 0), ("urgent", 1), ("4", 4)] {
        let mock = Routed::start(create_with(vec![]));
        let o = run(&sb, &mock, &create(&["--priority", spec]));
        assert_eq!(code(&o), 0, "{spec}: {}", stderr(&o));
        assert_eq!(
            mock.of("IssueCreate")[0]["input"]["priority"],
            sent,
            "{spec}"
        );
    }
}

#[test]
fn without_the_new_flags_none_of_their_fields_is_sent() {
    let sb = workspace();
    let mock = Routed::start(create_with(vec![]));
    let o = run(&sb, &mock, &create(&[]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let input = &mock.of("IssueCreate")[0]["input"];
    for key in ["priority", "estimate", "parentId", "cycleId"] {
        assert!(input.get(key).is_none(), "{key} in {input}");
    }
    assert!(mock.of("CycleList").is_empty());
    assert!(mock.of("IssueById").is_empty());
}

#[test]
fn bad_values_are_usage_errors_before_any_request() {
    let sb = workspace();
    for args in [
        &["--priority", "5"][..],
        &["--priority", "critical"],
        &["--estimate", "-1"],
        &["--estimate", "three"],
        &["--estimate", "none"],
        &["--cycle", "next"],
        &["--cycle", "none"],
        &["--cycle", "41", "--held-on", "2026-10-05"],
    ] {
        let mock = Routed::start(create_with(vec![]));
        let o = run(&sb, &mock, &create(args));
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(mock.ops().is_empty(), "{args:?}: a request was sent");
    }
}

#[test]
fn a_cycle_the_team_does_not_have_stops_before_anything_is_written() {
    let sb = workspace();
    let mock = Routed::start(create_with(vec![]));
    let o = run(&sb, &mock, &create(&["--cycle", "7"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("team EX has no cycle #7"),
        "{}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("#41"), "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_parent_linear_does_not_know_stops_before_anything_is_written() {
    let sb = workspace();
    let mock = Routed::start(create_with(vec![("IssueById", vec![no_such_issue()])]));
    let o = run(&sb, &mock, &create(&["--parent", "EX-999"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("no issue \"EX-999\""), "{}", stderr(&o));
    mock.assert_read_only();
}

fn existing(view: View) -> Vec<(&'static str, Vec<Reply>)> {
    create_with(vec![
        ("AttachmentsForUrlQuery", vec![issue_with_source()]),
        ("IssueWriteView", vec![view.reply()]),
        ("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]),
    ])
}

#[test]
fn an_issue_that_already_exists_keeps_its_priority_estimate_and_parent_and_says_so() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing(mine().in_cycle(Some(CYCLE_42))));
    let o = run(
        &sb,
        &mock,
        &create(&["--priority", "urgent", "--estimate", "8", "--json"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["existing"], true);
    mock.assert_read_only();

    // In text mode the note is on stderr.
    let mock = Routed::start(existing(mine().in_cycle(Some(CYCLE_42))));
    let o = run(
        &sb,
        &mock,
        &create(&["--priority", "urgent", "--estimate", "8"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--priority, --estimate were not applied"),
        "{}",
        stderr(&o)
    );
    mock.assert_read_only();
}

#[test]
fn an_issue_that_already_exists_gets_the_cycle_only_when_it_has_none() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(existing(mine().in_cycle(None)));
    let o = run(&sb, &mock, &create(&["--cycle", "41", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["changed"], json!(["cycle"]));
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({ "id": "id-EX-23", "input": { "cycleId": CYCLE_41 } })]
    );

    let mock = Routed::start(existing(mine().in_cycle(Some(CYCLE_42))));
    let o = run(&sb, &mock, &create(&["--cycle", "41", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["cycle"]["id"], CYCLE_42);
    mock.assert_read_only();
}

// ------------------------------------------------------------------ update: priority

#[test]
fn update_priority_sends_the_number_only_when_it_differs() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--priority", "urgent", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["priority"]));
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({ "id": "id-EX-23", "input": { "priority": 1 } })]
    );

    // Already medium (3): nothing is sent.
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--priority", "medium", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();
}

#[test]
fn update_priority_none_sets_zero_and_is_a_no_op_when_there_is_none() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--priority", "none"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueUpdate")[0]["input"], json!({ "priority": 0 }));

    let mock = Routed::start(update_routes(mine().with_priority(0.0), vec![]));
    let o = run(&sb, &mock, &update(&["--priority", "0", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ update: estimate

#[test]
fn update_estimate_sets_changes_and_clears() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--estimate", "8", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["estimate"]));
    assert_eq!(mock.of("IssueUpdate")[0]["input"], json!({ "estimate": 8 }));

    // The same estimate again: nothing.
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--estimate", "3", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();

    // `none` removes it with an explicit null.
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--estimate", "none", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!(["estimate"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "estimate": null })
    );

    // `none` on an issue without one: nothing.
    let mock = Routed::start(update_routes(mine().with_estimate(None), vec![]));
    let o = run(&sb, &mock, &update(&["--estimate", "None", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();

    // Zero is an estimate, not "none".
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--estimate", "0"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueUpdate")[0]["input"], json!({ "estimate": 0 }));
}

// ------------------------------------------------------------------ update: parent

#[test]
fn update_parent_sets_it_once_and_none_makes_the_issue_top_level() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--parent", "EX-1", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["parent"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "parentId": "id-EX-1" })
    );
    assert_eq!(mock.of("IssueById")[0]["id"], "EX-1");

    // Already its parent: nothing is sent.
    let mock = Routed::start(update_routes(mine().with_parent(Some("EX-1")), vec![]));
    let o = run(&sb, &mock, &update(&["--parent", "EX-1", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();

    // Another parent replaces it.
    let mock = Routed::start(update_routes(mine().with_parent(Some("EX-2")), vec![]));
    let o = run(&sb, &mock, &update(&["--parent", "EX-1", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!(["parent"]));

    // `none` clears it with an explicit null, and is a no-op without a parent.
    let mock = Routed::start(update_routes(mine().with_parent(Some("EX-1")), vec![]));
    let o = run(&sb, &mock, &update(&["--parent", "none", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!(["parent"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "parentId": null })
    );
    assert!(mock.of("IssueById").is_empty(), "none is not looked up");

    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--parent", "none", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();
}

#[test]
fn an_issue_cannot_be_its_own_parent_and_an_unknown_parent_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(
        mine(),
        vec![("IssueById", vec![issue_by_id("EX-23")])],
    ));
    let o = run(&sb, &mock, &update(&["--parent", "EX-23"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("EX-23 cannot be its own parent"),
        "{}",
        stderr(&o)
    );
    mock.assert_read_only();

    let mock = Routed::start(update_routes(
        mine(),
        vec![("IssueById", vec![no_such_issue()])],
    ));
    let o = run(&sb, &mock, &update(&["--parent", "EX-999"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("no issue \"EX-999\""), "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ update: cycle

#[test]
fn update_cycle_looks_the_number_up_in_the_issues_team() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "41", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["cycle"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "cycleId": CYCLE_41 })
    );
    assert_eq!(
        mock.of("CycleList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );

    // Already in it: nothing is sent.
    let mock = Routed::start(update_routes(mine().in_cycle(Some(CYCLE_41)), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "41", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();

    // Another cycle is moved to (unlike `create`, which never takes one back).
    let mock = Routed::start(update_routes(mine().in_cycle(Some(CYCLE_42)), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "41", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!(["cycle"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "cycleId": CYCLE_41 })
    );
}

#[test]
fn update_cycle_none_takes_the_issue_out_of_its_cycle() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine().in_cycle(Some(CYCLE_41)), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "none", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["cycle"]));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "cycleId": null })
    );
    assert!(mock.of("CycleList").is_empty(), "none is not looked up");

    let mock = Routed::start(update_routes(mine().in_cycle(None), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "none", "--json"]));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    mock.assert_read_only();
}

#[test]
fn update_with_a_cycle_the_team_does_not_have_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--cycle", "7"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("team EX has no cycle #7"),
        "{}",
        stderr(&o)
    );
    mock.assert_read_only();
}

// ------------------------------------------------------------------ update: together

#[test]
fn update_bad_values_are_usage_errors_before_any_request() {
    let sb = workspace_with_rules(&[]);
    for args in [
        &["--priority", "5"][..],
        &["--estimate", "1.5"],
        &["--estimate", "-2"],
        &["--cycle", "soon"],
    ] {
        let mock = Routed::start(update_routes(mine(), vec![]));
        let o = run(&sb, &mock, &update(args));
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(mock.ops().is_empty(), "{args:?}: a request was sent");
    }
}

#[test]
fn all_four_at_once_are_one_update_and_a_failed_source_puts_every_one_back() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(
        mine().with_parent(Some("EX-2")).in_cycle(Some(CYCLE_42)),
        vec![],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&[
            "--priority",
            "low",
            "--estimate",
            "none",
            "--parent",
            "EX-1",
            "--cycle",
            "41",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout_json(&o)["changed"],
        json!(["priority", "estimate", "parent", "cycle"])
    );
    let writes = mock.of("IssueUpdate");
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0]["input"],
        json!({
            "priority": 4, "estimate": null, "parentId": "id-EX-1", "cycleId": CYCLE_41,
        })
    );

    // With a source that cannot be attached, the old values go back, including the nulls.
    let mock = Routed::start(update_routes(
        mine(),
        vec![(
            "AttachmentCreate",
            vec![graphql_error("attachment refused")],
        )],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&[
            "--priority",
            "low",
            "--estimate",
            "8",
            "--parent",
            "EX-1",
            "--cycle",
            "41",
            "--source",
            NEW_SOURCE,
        ]),
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("rolled back"), "{}", stderr(&o));
    let writes = mock.of("IssueUpdate");
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[1]["input"],
        json!({
            "priority": 3, "estimate": 3, "parentId": null, "cycleId": null,
        })
    );
}

#[test]
fn somebody_elses_issue_is_refused_for_the_new_fields_too() {
    let sb = workspace_with_rules(&[]);
    for args in [
        &["--priority", "high"][..],
        &["--estimate", "2"],
        &["--parent", "EX-1"],
        &["--cycle", "41"],
    ] {
        let mock = Routed::start(update_routes(
            view("EX-23")
                .assigned_to(Some(BOT))
                .in_project(OTHER_PROJECT, Some(BOT)),
            vec![],
        ));
        let o = run(&sb, &mock, &update(args));
        assert_eq!(code(&o), 4, "{args:?}: {}", stderr(&o));
        mock.assert_read_only();
    }
}

#[test]
fn nothing_to_change_names_the_new_flags() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&[]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    for flag in ["--priority", "--estimate", "--parent", "--cycle"] {
        assert!(stderr(&o).contains(flag), "{flag}: {}", stderr(&o));
    }
}
