//! `linear status`: the cache in one line, from the file alone.

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
    sb.cache_dir().join(format!("{workspace}.json"))
}

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

/// `linear status <args>` with no mock and no credentials.
fn status(sb: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut full = vec!["status"];
    full.extend_from_slice(args);
    sb.run(&full, None, &[])
}

/// What the refresh found: (in progress, findings, actionable, new).
fn counts(sb: &Sandbox) -> (usize, usize, usize, usize) {
    let e: Value =
        serde_json::from_str(&std::fs::read_to_string(entry_file(sb, "example")).unwrap()).unwrap();
    let d = &e["data"];
    let n = |k: &str| d[k].as_array().unwrap().len();
    let actionable = d["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["actionable"] == true)
        .count();
    (n("issues"), n("findings"), actionable, n("newFindings"))
}

#[test]
fn a_fresh_snapshot_prints_one_line() {
    let sb = refreshed();
    let (issues, _, actionable, _) = counts(&sb);
    assert!(
        issues > 0 && actionable > 0,
        "the fixtures give something to count"
    );

    let o = status(&sb, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        format!("example: {issues} in progress, {actionable} actionable\n")
    );
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
}

#[test]
fn json_has_the_state_the_numbers_and_the_line() {
    let sb = refreshed();
    let (issues, findings, actionable, new) = counts(&sb);
    let o = status(&sb, &["--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let rows = stdout_json(&o);
    let r = &rows[0];
    assert_eq!(r["workspace"], "example");
    assert_eq!(r["state"], "fresh");
    assert_eq!(r["ttlSecs"], 300);
    assert_eq!(r["inProgress"], issues);
    assert_eq!(r["findings"], findings);
    assert_eq!(r["actionable"], actionable);
    assert_eq!(r["newFindings"], new);
    assert_eq!(r["refreshFailed"], false);
    assert!(r["ageSecs"].as_u64().unwrap() < 60);
    assert!(r["fetchedAt"].is_string());
    assert!(r["reason"].is_null());
    assert_eq!(
        r["line"],
        format!("example: {issues} in progress, {actionable} actionable")
    );

    let o = status(&sb, &["--quiet"]);
    assert_eq!(stdout(&o), "example fresh\n");
}

#[test]
fn it_makes_no_request_and_needs_no_credentials() {
    let sb = refreshed();
    let mock = Mock::start(vec![]);
    // A key and a server to talk to are at hand; they are not used.
    let o = linear(&sb, &mock, &["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
    assert_no_leak(&o);
}

#[test]
fn a_stale_snapshot_is_unknown_and_shows_no_numbers() {
    let sb = refreshed();
    edit_entry(&sb, "example", |e| {
        e["fetchedAt"] = json!((chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339());
    });
    let o = status(&sb, &[]);
    assert_eq!(
        code(&o),
        0,
        "it reports unknown; it is not a failure of the command"
    );
    assert_eq!(stdout(&o), "example: unknown (snapshot 2h old)\n");
    assert!(!stdout(&o).contains("in progress"));

    let rows = stdout_json(&status(&sb, &["--json"]));
    let r = &rows[0];
    assert_eq!(r["state"], "expired");
    assert!(r["ageSecs"].as_u64().unwrap() >= 7200);
    for k in [
        "inProgress",
        "findings",
        "actionable",
        "newFindings",
        "fetchedAt",
    ] {
        assert!(
            r[k].is_null(),
            "{k} must not be shown for a stale snapshot: {r}"
        );
    }
    assert!(r["reason"].as_str().unwrap().contains("past the 300s TTL"));

    assert_eq!(stdout(&status(&sb, &["--quiet"])), "example expired\n");

    // A longer TTL vouches for it again.
    let o = status(&sb, &["--ttl", "86400"]);
    assert!(stdout(&o).contains("in progress"), "{}", stdout(&o));
    // And a TTL of 0 refuses even a snapshot from a moment ago.
    let fresh = refreshed();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = status(&fresh, &["--ttl", "0"]);
    assert!(stdout(&o).contains("unknown"), "{}", stdout(&o));
}

#[test]
fn nothing_cached_is_unknown() {
    let sb = workspace();
    let o = status(&sb, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "example: unknown (nothing cached)\n");
    let r = &stdout_json(&status(&sb, &["--json"]))[0];
    assert_eq!(r["state"], "missing");
    assert!(r["inProgress"].is_null());
    assert_eq!(stdout(&status(&sb, &["-q"])), "example missing\n");
}

#[test]
fn another_schema_version_or_workspace_or_garbage_is_unusable() {
    let sb = refreshed();
    edit_entry(&sb, "example", |e| e["schemaVersion"] = json!(99));
    assert_eq!(
        stdout(&status(&sb, &[])),
        "example: unknown (unusable cache file)\n"
    );
    let r = &stdout_json(&status(&sb, &["--json"]))[0];
    assert_eq!(r["state"], "unusable");
    assert!(r["reason"].as_str().unwrap().contains("schemaVersion 99"));

    let sb = refreshed();
    edit_entry(&sb, "example", |e| e["workspace"] = json!("other"));
    let r = &stdout_json(&status(&sb, &["--json"]))[0];
    assert_eq!(r["state"], "unusable");
    assert!(r["inProgress"].is_null());

    let sb = refreshed();
    std::fs::write(entry_file(&sb, "example"), "{ not json").unwrap();
    assert_eq!(stdout(&status(&sb, &["-q"])), "example unusable\n");
}

#[test]
fn a_failed_refresh_is_visible_while_the_snapshot_is_still_within_the_ttl() {
    let sb = refreshed();
    let (issues, _, actionable, _) = counts(&sb);
    edit_entry(&sb, "example", |e| {
        e["status"] = json!("failed");
        e["failure"] = json!({ "at": chrono::Utc::now().to_rfc3339(), "message": "boom" });
    });
    assert_eq!(
        stdout(&status(&sb, &[])),
        format!("example: {issues} in progress, {actionable} actionable (last refresh failed)\n")
    );
    let r = &stdout_json(&status(&sb, &["--json"]))[0];
    assert_eq!(r["state"], "fresh");
    assert_eq!(r["refreshFailed"], true);
    assert_eq!(r["reason"], "boom");
}

#[test]
fn every_workspace_gets_a_segment_unless_one_is_selected() {
    let sb = refreshed();
    let o = sb.run(
        &["workspace", "add", "other", "--url-key", "other"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (issues, _, actionable, _) = counts(&sb);

    assert_eq!(
        stdout(&status(&sb, &[])),
        format!(
            "example: {issues} in progress, {actionable} actionable | other: unknown (nothing cached)\n"
        )
    );
    assert_eq!(
        stdout(&status(&sb, &["-w", "other"])),
        "other: unknown (nothing cached)\n"
    );
    assert_eq!(
        stdout(&status(&sb, &["-q"])),
        "example fresh\nother missing\n"
    );
}

#[test]
fn a_workspace_name_cannot_point_outside_the_cache() {
    let sb = workspace();
    let o = status(&sb, &["-w", "../evil"]);
    assert_ne!(code(&o), 0);
}
