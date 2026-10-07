//! `issue delete|archive|unarchive`: one line, the issue's ownership rule, and
//! exactly one mutation. Against a mock Linear that answers by operation name.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn routes(view: View) -> Vec<(&'static str, Vec<Reply>)> {
    let ok = |field: &str| data(json!({ field: { "success": true } }));
    vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("IssueDelete", vec![ok("issueDelete")]),
        ("IssueArchive", vec![ok("issueArchive")]),
        ("IssueUnarchive", vec![ok("issueUnarchive")]),
    ]
}

/// (command, the mutation it sends, what it reports).
const CASES: [(&str, &str, &str); 3] = [
    ("delete", "IssueDelete", "deleted"),
    ("archive", "IssueArchive", "archived"),
    ("unarchive", "IssueUnarchive", "unarchived"),
];

#[test]
fn each_command_sends_its_one_mutation_with_the_issue_id() {
    let sb = workspace();
    for (command, mutation, action) in CASES {
        let mock = Routed::start(routes(mine()));
        let o = run(&sb, &mock, &["issue", command, "EX-23", "--json"]);
        assert_eq!(code(&o), 0, "{command}: {}", stderr(&o));
        let v = stdout_json(&o);
        assert_eq!(v["workspace"], "example");
        assert_eq!(v["identifier"], "EX-23");
        assert_eq!(v["id"], "id-EX-23");
        assert_eq!(v["action"], action);
        assert_eq!(
            mock.of(mutation),
            vec![json!({ "id": "id-EX-23" })],
            "{command}"
        );
        let sent: Vec<String> = mock
            .ops()
            .into_iter()
            .filter(|o| o.starts_with("Issue") && *o != "IssueWriteView")
            .collect();
        assert_eq!(sent, [mutation], "{command}");
    }
}

#[test]
fn text_is_one_line_and_quiet_is_the_identifier() {
    let sb = workspace();
    for (command, _, action) in CASES {
        let mock = Routed::start(routes(mine()));
        let o = run(&sb, &mock, &["issue", command, "EX-23"]);
        assert_eq!(code(&o), 0, "{command}: {}", stderr(&o));
        let out = stdout(&o);
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(
            out.starts_with("EX-23  ") && out.trim_end().ends_with(&format!("({action})")),
            "{out}"
        );

        let o = run(&sb, &mock, &["issue", command, "EX-23", "--quiet"]);
        assert_eq!(stdout(&o).trim(), "EX-23");
    }
}

#[test]
fn nobody_elses_issue_is_touched_in_a_strict_workspace() {
    let sb = workspace();
    for (command, _, _) in CASES {
        let mock = Routed::start(routes(
            view("EX-23")
                .assigned_to(Some(BOT))
                .in_project(PROJECT, Some(BOT)),
        ));
        let o = run(&sb, &mock, &["issue", command, "EX-23"]);
        assert_eq!(code(&o), 4, "{command}: {}", stderr(&o));
        mock.assert_read_only();
    }
}

#[test]
fn a_project_you_lead_makes_its_issues_yours() {
    let sb = workspace();
    let mock = Routed::start(routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(ALICE)),
    ));
    let o = run(&sb, &mock, &["issue", "archive", "EX-23"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueArchive").len(), 1);
}

#[test]
fn a_lenient_workspace_allows_a_colleagues_issue() {
    let sb = lenient_workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
    ));
    let o = run(&sb, &mock, &["issue", "delete", "EX-23"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueDelete").len(), 1);
}

#[test]
fn a_refusal_from_linear_is_an_error() {
    let sb = workspace();
    let mut r = routes(mine());
    r.retain(|(o, _)| *o != "IssueArchive");
    r.push((
        "IssueArchive",
        vec![data(json!({ "issueArchive": { "success": false } }))],
    ));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["issue", "archive", "EX-23"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("could not change EX-23"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn a_dry_run_of_each_command_plans_its_one_mutation() {
    let sb = workspace();
    for (command, mutation, _) in CASES {
        let mock = Routed::start(routes(mine()));
        let v = plan(
            &run(
                &sb,
                &mock,
                &["issue", command, "EX-23", "--dry-run", "--json"],
            ),
            &mock,
        );
        assert_eq!(v["command"], format!("issue {command}"));
        assert_eq!(planned(&v), [mutation], "{command}");
        assert_eq!(v["mutations"][0]["variables"], json!({ "id": "id-EX-23" }));
        assert_eq!(v["target"]["id"], "id-EX-23");
    }

    // Somebody else's issue is refused in a strict workspace.
    let theirs = view("EX-23")
        .assigned_to(Some(BOT))
        .in_project(PROJECT, Some(BOT));
    let mock = Routed::start(routes(theirs));
    let o = run(&sb, &mock, &["issue", "delete", "EX-23", "--dry-run"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}
