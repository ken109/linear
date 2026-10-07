//! `--dry-run` as a global flag: which commands take it, what it refuses, how it is documented.
//! What each write command prints is checked next to that command's own tests (`plan` in
//! `write_support`), so every write has a test that proves no mutation reaches the server.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

/// Commands that do not write to Linear: `--dry-run` is a usage error for each, and they
/// never get as far as asking Linear anything.
#[test]
fn a_command_that_does_not_write_refuses_the_flag() {
    let sb = workspace();
    let mock = Routed::start(vec![("Whoami", vec![whoami()])]);
    let not_writes: &[&[&str]] = &[
        &["issue", "list"],
        &["issue", "view", "EX-1"],
        &["issue", "search", "x"],
        &["project", "list"],
        &["milestone", "list", "--project", "p"],
        &["initiative", "status-updates", "x"],
        &["label", "list"],
        &["document", "list"],
        &["webhook", "list"],
        &["webhook", "verify", "--signature", "00"],
        &["team", "list"],
        &["user", "list"],
        &["cycle", "2026-10-05"],
        &["audit"],
        &["status"],
        &["brief"],
        &["cache", "refresh"],
        &["cache", "clear"],
        &["workspace", "list"],
        &["workspace", "whoami"],
        &["workspace", "add", "x", "--url-key", "x"],
        &["file", "download", "https://uploads.linear.app/a/b/c"],
        &["completions", "bash"],
        &["api", "{ viewer { id } }"],
    ];
    for args in not_writes {
        let mut full = args.to_vec();
        full.push("--dry-run");
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("--dry-run is for the commands that write to Linear"),
            "{args:?}: {}",
            stderr(&o)
        );
        assert_eq!(stdout(&o), "", "{args:?}");
    }
    assert!(
        mock.ops().is_empty(),
        "nothing was asked of Linear: {:?}",
        mock.ops()
    );
    // The refusal is a usage error in JSON too.
    let o = run(&sb, &mock, &["issue", "list", "--dry-run", "--json"]);
    assert_eq!(code(&o), 2);
    let err: serde_json::Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(err["error"]["code"], "usage");
}

#[test]
fn the_flag_goes_before_or_after_the_subcommand() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        (
            "IssueWriteView",
            vec![view("EX-23")
                .assigned_to(Some(ALICE))
                .in_project(PROJECT, Some(ALICE))
                .reply()],
        ),
    ]);
    for args in [
        vec!["--dry-run", "issue", "delete", "EX-23", "--json"],
        vec!["issue", "--dry-run", "delete", "EX-23", "--json"],
        vec!["issue", "delete", "EX-23", "--json", "--dry-run"],
    ] {
        let v = plan(&run(&sb, &mock, &args), &mock);
        assert_eq!(planned(&v), ["IssueDelete"], "{args:?}");
    }
}

#[test]
fn the_help_says_what_the_flag_does() {
    let sb = workspace();
    let o = sb.run(&["--help"], None, &[]);
    assert_eq!(code(&o), 0);
    let help = stdout(&o);
    assert!(help.contains("--dry-run"), "{help}");
    assert!(help.contains("mutations a write would send"), "{help}");
    // A write command shows it too (the flag is global).
    let o = sb.run(&["issue", "update", "--help"], None, &[]);
    assert!(stdout(&o).contains("--dry-run"), "{}", stdout(&o));
}

#[test]
fn a_plan_has_the_same_shape_in_text_and_names_the_workspace() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        (
            "IssueWriteView",
            vec![view("EX-23")
                .assigned_to(Some(ALICE))
                .in_project(PROJECT, Some(ALICE))
                .reply()],
        ),
    ]);
    let o = run(&sb, &mock, &["issue", "archive", "EX-23", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    assert!(
        text.starts_with(
            "dry run, nothing was sent: issue archive (issue EX-23, workspace example)"
        ),
        "{text}"
    );
    assert!(text.contains("1. IssueArchive"), "{text}");
    assert!(text.contains(&json!("id-EX-23").to_string()), "{text}");
    mock.assert_read_only();
}
