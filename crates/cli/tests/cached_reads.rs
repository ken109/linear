//! `--cached` on `issue list|view` and `project list|view`, and `linear status`,
//! against a cache that a refresh wrote from the mock server.
//!
//! The reads themselves are run without a mock and without credentials: a
//! `--cached` read that tried the network could not succeed, so every success
//! here is a read of the file alone.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use std::path::PathBuf;

fn replies() -> Vec<Reply> {
    vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(&fixture("projects")),
    ]
}

fn entry_file(sb: &Sandbox, workspace: &str) -> PathBuf {
    sb.root
        .path()
        .join(".cache")
        .join("linear")
        .join(format!("{workspace}.json"))
}

/// A sandbox whose `example` workspace has been refreshed from the fixtures.
fn refreshed() -> Sandbox {
    let sb = workspace();
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    sb
}

fn edit_entry(sb: &Sandbox, workspace: &str, change: impl FnOnce(&mut Value)) {
    let file = entry_file(sb, workspace);
    let mut e: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    change(&mut e);
    std::fs::write(&file, e.to_string()).unwrap();
}

fn hours_ago(h: i64) -> Value {
    json!((chrono::Utc::now() - chrono::Duration::hours(h)).to_rfc3339())
}

fn age(sb: &Sandbox) {
    edit_entry(sb, "example", |e| e["fetchedAt"] = hours_ago(1));
}

fn run(sb: &Sandbox, args: &[&str]) -> std::process::Output {
    sb.run(args, None, &[])
}

// ------------------------------------------------------------- issue list

#[test]
fn issue_list_cached_reads_the_snapshot() {
    let sb = refreshed();
    let o = run(&sb, &["issue", "list", "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let rows = stdout_json(&o);
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["identifier"], "EX-23");
    assert_eq!(rows[0]["workspace"], "example");
    assert_eq!(rows[0]["state"]["type"], "started");
    assert_eq!(rows[0]["assignee"]["isMe"], true);
    assert!(
        stderr(&o).contains("from the cache of example"),
        "the answer says it is cached: {}",
        stderr(&o)
    );

    // Same keys as a live row, plus nothing else.
    let live_rows: Value = {
        let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
        let o = linear(&sb, &mock, &["issue", "list", "--json", "--all"]);
        stdout_json(&o)
    };
    let live = live_rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["identifier"] == "EX-23")
        .unwrap();
    assert_eq!(&rows[0], live);

    let o = run(&sb, &["issue", "list", "--cached"]);
    assert!(stdout(&o).contains("EX-23"), "{}", stdout(&o));
    let o = run(&sb, &["issue", "list", "--cached", "--quiet"]);
    assert_eq!(stdout(&o).trim(), "EX-23");
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
}

#[test]
fn issue_list_cached_accepts_the_filters_the_cache_already_applied() {
    let sb = refreshed();
    let o = run(
        &sb,
        &[
            "issue",
            "list",
            "--cached",
            "--assignee",
            "me",
            "--state-type",
            "started",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "EX-23");

    let o = run(
        &sb,
        &["issue", "list", "--cached", "--limit", "1", "--quiet"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn issue_list_cached_refuses_filters_the_cache_does_not_hold() {
    // Checked before the cache is read: it is a mistake in the command, whatever the cache has.
    let sb = workspace();
    for args in [
        &["--assignee", "none"][..],
        &["--assignee", "someone@example.com"],
        &["--state-type", "backlog"],
        &["--state-type", "started,backlog"],
        &["--state", "In Progress"],
        &["--open"],
        &["--team", "EX"],
        &["--project", "Fixture Project"],
        &["--milestone", "M1"],
        &["--label", "bug"],
        &["--source-url", "https://example.com/a"],
    ] {
        let mut full = vec!["issue", "list", "--cached"];
        full.extend_from_slice(args);
        let o = run(&sb, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("--cached") && stderr(&o).contains(args[0]),
            "{args:?}: {}",
            stderr(&o)
        );
        assert!(stdout(&o).is_empty());
    }
    // Several at once are all named.
    let o = run(
        &sb,
        &["issue", "list", "--cached", "--team", "EX", "--label", "x"],
    );
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("--team") && stderr(&o).contains("--label"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn ttl_needs_cached() {
    let sb = refreshed();
    let o = run(&sb, &["issue", "list", "--ttl", "10"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = run(&sb, &["project", "view", "aaaaaaaaaaaa", "--ttl", "10"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

// ------------------------------------------- the entry cannot be trusted

/// Every read that takes `--cached`, with what it needs.
fn reads() -> Vec<Vec<&'static str>> {
    vec![
        vec!["issue", "list", "--cached"],
        vec!["issue", "view", "EX-23", "--cached"],
        vec!["project", "list", "--cached"],
        vec!["project", "view", "aaaaaaaaaaaa", "--cached"],
    ]
}

fn assert_unknown(o: &std::process::Output, what: &str, args: &[&str]) {
    assert_eq!(code(o), 1, "{args:?}: {}", stderr(o));
    assert!(stdout(o).is_empty(), "{args:?}: nothing on stdout");
    assert!(stderr(o).contains(what), "{args:?}: {}", stderr(o));
    assert!(
        stderr(o).contains("linear cache refresh"),
        "{args:?}: says what to do: {}",
        stderr(o)
    );
}

#[test]
fn nothing_cached_is_an_error_not_an_empty_list() {
    let sb = workspace();
    for args in reads() {
        assert_unknown(&run(&sb, &args), "nothing cached", &args);
    }
    // With --json too: the error is JSON, and stdout stays empty.
    let o = run(&sb, &["issue", "list", "--cached", "--json"]);
    assert_eq!(code(&o), 1);
    assert!(stdout(&o).is_empty());
    let err: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(err["error"]["code"], "error");
}

#[test]
fn a_stale_snapshot_is_an_error_and_a_longer_ttl_accepts_it() {
    let sb = refreshed();
    age(&sb);
    for args in reads() {
        assert_unknown(&run(&sb, &args), "past the 300s TTL", &args);
    }
    for args in reads() {
        let mut full = args.clone();
        full.extend(["--ttl", "7200"]);
        let o = run(&sb, &full);
        assert_eq!(code(&o), 0, "{full:?}: {}", stderr(&o));
    }
    // --ttl 0 is as strict as it gets: an hour-old snapshot fails.
    let o = run(&sb, &["issue", "list", "--cached", "--ttl", "0"]);
    assert_unknown(&o, "past the 0s TTL", &["--ttl", "0"]);
}

#[test]
fn a_snapshot_that_a_failed_refresh_could_not_renew_is_not_served_once_stale() {
    let sb = refreshed();
    age(&sb);
    edit_entry(&sb, "example", |e| {
        e["status"] = json!("failed");
        e["failure"] = json!({ "at": chrono::Utc::now().to_rfc3339(), "message": "boom" });
    });
    let o = run(&sb, &["issue", "list", "--cached"]);
    assert_unknown(&o, "the last refresh failed: boom", &["issue", "list"]);
}

#[test]
fn a_refresh_that_never_worked_is_unknown() {
    let sb = workspace();
    let mock = Mock::start(vec![]);
    // No credentials: the refresh fails and records it with nothing to keep.
    assert_eq!(
        code(&sb.run(&["cache", "refresh"], Some(&mock), &[])),
        1,
        "the refresh fails"
    );
    let o = run(&sb, &["project", "list", "--cached"]);
    assert_unknown(
        &o,
        "never fetched; the refresh failed",
        &["project", "list"],
    );
}

#[test]
fn another_schema_version_is_unusable() {
    let sb = refreshed();
    edit_entry(&sb, "example", |e| e["schemaVersion"] = json!(99));
    for args in reads() {
        assert_unknown(&run(&sb, &args), "schemaVersion 99", &args);
    }
}

#[test]
fn a_file_that_is_not_json_is_unusable() {
    let sb = refreshed();
    std::fs::write(entry_file(&sb, "example"), "{ not json").unwrap();
    for args in reads() {
        assert_unknown(&run(&sb, &args), "not valid JSON", &args);
    }
}

#[test]
fn an_entry_of_another_workspace_is_not_read() {
    let sb = refreshed();
    // `other` is configured but has never been refreshed; `example` has.
    let o = sb.run(
        &["workspace", "add", "other", "--url-key", "other"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    for args in reads() {
        let mut full = args.clone();
        full.extend(["-w", "other"]);
        let o = run(&sb, &full);
        assert_unknown(&o, "other: nothing cached", &full);
        assert!(!stderr(&o).contains("EX-23"));
    }

    // A file under the name of one workspace that holds another's entry.
    std::fs::copy(entry_file(&sb, "example"), entry_file(&sb, "other")).unwrap();
    for args in reads() {
        let mut full = args.clone();
        full.extend(["-w", "other"]);
        assert_unknown(&run(&sb, &full), "holds workspace \"example\"", &full);
    }
    // ... and the one it belongs to still reads.
    assert_eq!(code(&run(&sb, &["issue", "list", "--cached"])), 0);
}

// ------------------------------------------------------------- issue view

#[test]
fn issue_view_cached_shows_what_the_snapshot_has() {
    let sb = refreshed();
    let o = run(&sb, &["issue", "view", "ex-23", "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["workspace"], "example");
    // The detail (priority label, comments) is not in the snapshot, so it is absent, not empty.
    assert!(v.get("priorityLabel").is_none() && v.get("comments").is_none());

    let o = run(&sb, &["issue", "view", "EX-23", "--cached"]);
    let text = stdout(&o);
    assert!(text.contains("EX-23") && text.contains("started"), "{text}");
    assert!(text.contains("(not in the cache)"), "{text}");
    assert!(!text.contains("Comments"), "{text}");

    let o = run(&sb, &["issue", "view", "EX-23", "--cached", "--quiet"]);
    assert_eq!(stdout(&o).trim(), "EX-23");
}

#[test]
fn issue_view_cached_of_an_issue_outside_the_snapshot_says_so() {
    let sb = refreshed();
    let o = run(&sb, &["issue", "view", "EX-999", "--cached"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stdout(&o).is_empty());
    assert!(
        stderr(&o).contains("EX-999 is not in the cache") && stderr(&o).contains("In Progress"),
        "{}",
        stderr(&o)
    );
}

// ----------------------------------------------------------- project list

#[test]
fn project_list_cached_reads_the_projects_of_the_issues_in_progress() {
    let sb = refreshed();
    let o = run(&sb, &["project", "list", "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let rows = stdout_json(&o);
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["name"], "Fixture Project");
    assert_eq!(rows[0]["workspace"], "example");
    assert!(rows[0]["issueCounts"].is_object());

    let o = run(&sb, &["project", "list", "--cached"]);
    assert!(stdout(&o).contains("Fixture Project"), "{}", stdout(&o));
    let o = run(&sb, &["project", "list", "--cached", "--quiet"]);
    assert_eq!(stdout(&o).trim(), "aaaaaaaaaaaa");
}

#[test]
fn project_list_cached_refuses_filters() {
    let sb = workspace();
    for args in [
        &["--lead", "me"][..],
        &["--status-type", "started"],
        &["--open"],
        &["--initiative", "Q4"],
    ] {
        let mut full = vec!["project", "list", "--cached"];
        full.extend_from_slice(args);
        let o = run(&sb, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(args[0]), "{args:?}: {}", stderr(&o));
    }
}

// ----------------------------------------------------------- project view

#[test]
fn project_view_cached_finds_the_project_the_way_a_live_view_does() {
    let sb = refreshed();
    for reference in [
        "aaaaaaaaaaaa",
        "Fixture Project",
        "fixture project",
        "https://linear.app/example/project/fixture-project-aaaaaaaaaaaa",
    ] {
        let o = run(&sb, &["project", "view", reference, "--cached", "--json"]);
        assert_eq!(code(&o), 0, "{reference}: {}", stderr(&o));
        let v = stdout_json(&o);
        assert_eq!(v["name"], "Fixture Project");
        assert!(v.get("description").is_none() && v.get("projectUpdates").is_none());
    }
    let o = run(&sb, &["project", "view", "aaaaaaaaaaaa", "--cached"]);
    let text = stdout(&o);
    assert!(
        text.contains("Fixture Project") && text.contains("Issues"),
        "{text}"
    );
    let o = run(
        &sb,
        &["project", "view", "aaaaaaaaaaaa", "--cached", "--quiet"],
    );
    assert_eq!(stdout(&o).trim(), "aaaaaaaaaaaa");
}

#[test]
fn project_view_cached_outside_the_snapshot_or_with_content_fails() {
    let sb = refreshed();
    let o = run(&sb, &["project", "view", "No Such Project", "--cached"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("the cache holds only the projects"),
        "{}",
        stderr(&o)
    );

    // The content document is not in the snapshot.
    let o = run(
        &sb,
        &["project", "view", "aaaaaaaaaaaa", "--cached", "--content"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

// ----------------------------------------------------------------- nothing silent

#[test]
fn without_cached_the_commands_still_ask_linear() {
    let sb = refreshed();
    // A fresh snapshot exists, but a plain read must not look at it: with no
    // mock and no key it fails (no credentials), never serving the cache.
    for args in [
        &["issue", "list"][..],
        &["issue", "view", "EX-23"],
        &["project", "list"],
        &["project", "view", "aaaaaaaaaaaa"],
    ] {
        let o = run(&sb, args);
        assert_eq!(code(&o), 3, "{args:?}: {}", stderr(&o));
        assert!(stdout(&o).is_empty(), "{args:?}");
    }
}

#[test]
fn a_cached_read_makes_no_request() {
    let sb = refreshed();
    let mock = Mock::start(vec![]);
    for args in reads() {
        // With a key and a mock to talk to: it still does not.
        let o = linear(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    }
    assert!(mock.requests().is_empty());
}
