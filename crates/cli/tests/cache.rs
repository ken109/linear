//! `linear cache refresh|show|clear` against a mock Linear server.
//!
//! The mock answers in the order requests arrive, so a test that talks to the
//! network uses one workspace; a second one without credentials fails without
//! touching it.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use std::path::PathBuf;

/// What one refresh asks for, in order: who am I, the issues, the projects.
fn replies() -> Vec<Reply> {
    vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(&fixture("projects")),
    ]
}

fn cache_dir(sb: &Sandbox) -> PathBuf {
    // The sandbox sets HOME to its root and no XDG_CACHE_HOME.
    sb.root.path().join(".cache").join("linear")
}

fn entry_file(sb: &Sandbox) -> PathBuf {
    cache_dir(sb).join("example.json")
}

fn read_entry(sb: &Sandbox) -> Value {
    serde_json::from_str(&std::fs::read_to_string(entry_file(sb)).unwrap()).unwrap()
}

fn show(sb: &Sandbox, extra: &[&str]) -> Value {
    let mut args = vec!["cache", "show", "--json"];
    args.extend_from_slice(extra);
    let o = sb.run(&args, None, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    stdout_json(&o)
}

#[test]
fn nothing_is_cached_until_a_refresh() {
    let sb = workspace();
    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["workspace"], "example");
    assert_eq!(rows[0]["freshness"]["state"], "missing");
    assert!(rows[0]["entry"].is_null());
    assert!(!cache_dir(&sb).exists());
}

#[test]
fn a_refresh_stores_the_viewer_the_issues_in_progress_and_the_audit() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);

    let out = stdout_json(&o);
    assert_eq!(out["unreachable"], json!([]));
    let row = &out["workspaces"][0];
    assert_eq!(row["workspace"], "example");
    assert_eq!(row["status"], "ok");
    // Nothing was cached before, so whatever the audit found is new.
    assert_eq!(
        row["findings"],
        row["new_findings"].as_array().unwrap().len()
    );

    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    let names: Vec<_> = (0..3)
        .map(|n| {
            request(&mock, n)["operationName"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(names, ["Whoami", "IssueList", "Projects"]);

    let e = read_entry(&sb);
    assert_eq!(e["schemaVersion"], linear_core::SCHEMA_VERSION);
    assert_eq!(e["workspace"], "example");
    assert_eq!(e["status"], "ok");
    assert!(e["failure"].is_null());
    assert!(e["fetchedAt"].is_string());
    assert_eq!(e["data"]["viewer"]["isMe"], true);
    // EX-23 is assigned to the viewer and In Progress; its project comes with it.
    assert_eq!(e["data"]["issues"][0]["identifier"], "EX-23");
    assert_eq!(e["data"]["projects"][0]["name"], "Fixture Project");
    assert!(e["data"]["findings"].is_array());
    assert!(!std::fs::read_to_string(entry_file(&sb))
        .unwrap()
        .contains(KEY));
}

#[test]
fn the_entry_is_private_and_nothing_is_left_beside_it() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&entry_file(&sb)), 0o600);
        assert_eq!(mode(&cache_dir(&sb)), 0o700);
    }
    let names: Vec<String> = std::fs::read_dir(cache_dir(&sb))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["example.json"], "no temporary or lock file stays");
}

#[test]
fn show_reads_a_fresh_entry_without_touching_the_network() {
    let sb = workspace();
    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);

    // No credentials and no mock: showing must not need either.
    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["freshness"]["state"], "fresh");
    assert_eq!(rows[0]["ttl_secs"], 300);
    assert_eq!(rows[0]["entry"]["data"]["issues"][0]["identifier"], "EX-23");

    let o = sb.run(&["cache", "show"], None, &[]);
    assert_eq!(code(&o), 0);
    let text = stdout(&o);
    assert!(text.contains("example") && text.contains("fresh"), "{text}");
}

#[test]
fn an_entry_older_than_the_ttl_is_unknown() {
    let sb = workspace();
    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);

    let mut e = read_entry(&sb);
    let hour_ago = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    e["fetchedAt"] = json!(hour_ago);
    std::fs::write(entry_file(&sb), e.to_string()).unwrap();

    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["freshness"]["state"], "expired");
    assert!(rows[0]["freshness"]["ageSecs"].as_u64().unwrap() >= 3600);
    // The data is still shown for people; it is just not vouched for.
    assert!(rows[0]["entry"]["data"].is_object());

    let o = sb.run(&["cache", "show"], None, &[]);
    assert!(stdout(&o).contains("unknown (expired)"), "{}", stdout(&o));

    // A longer TTL accepts it.
    let rows = show(&sb, &["--ttl", "7200"]);
    assert_eq!(rows[0]["freshness"]["state"], "fresh");
}

#[test]
fn a_failed_refresh_keeps_the_old_snapshot_and_records_the_failure() {
    let sb = workspace();
    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);
    let before = read_entry(&sb);

    let broken = Mock::start(vec![Reply {
        status: 500,
        body: r#"{"errors":[{"message":"boom"}]}"#.into(),
    }]);
    let o = linear(&sb, &broken, &["cache", "refresh", "--json"]);
    assert_eq!(code(&o), 1, "a failed refresh must not look like success");
    let out = stdout_json(&o);
    assert_eq!(out["unreachable"], json!(["example"]));
    assert_eq!(out["workspaces"][0]["status"], "failed");
    assert!(stderr(&o).contains("example"), "{}", stderr(&o));

    let after = read_entry(&sb);
    assert_eq!(after["status"], "failed");
    assert_eq!(
        after["data"], before["data"],
        "the snapshot is kept, not emptied"
    );
    assert_eq!(after["fetchedAt"], before["fetchedAt"]);
    assert!(after["failure"]["message"]
        .as_str()
        .unwrap()
        .contains("boom"));
    assert!(after["failure"]["at"].is_string());

    // The kept snapshot is still within its TTL, and the failure is visible.
    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["freshness"]["state"], "fresh");
    assert_eq!(rows[0]["entry"]["status"], "failed");
    let o = sb.run(&["cache", "show"], None, &[]);
    assert!(stdout(&o).contains("boom"), "{}", stdout(&o));

    // A later success clears the failure.
    let again = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &again, &["cache", "refresh"])), 0);
    let healed = read_entry(&sb);
    assert_eq!(healed["status"], "ok");
    assert!(healed["failure"].is_null());
}

#[test]
fn a_workspace_without_credentials_fails_without_a_request() {
    let sb = workspace();
    let mock = Mock::start(vec![]);
    let o = sb.run(&["cache", "refresh", "--json"], Some(&mock), &[]);
    assert_eq!(code(&o), 1);
    assert!(mock.requests().is_empty());
    let out = stdout_json(&o);
    assert_eq!(out["unreachable"], json!(["example"]));
    assert!(out["workspaces"][0]["message"]
        .as_str()
        .unwrap()
        .contains("no credentials"));
    // The failure is recorded even with nothing to keep.
    let e = read_entry(&sb);
    assert_eq!(e["status"], "failed");
    assert!(e["data"].is_null() && e["fetchedAt"].is_null());
    assert_eq!(show(&sb, &[])[0]["freshness"]["state"], "missing");
}

#[test]
fn only_findings_that_are_new_since_the_last_refresh_are_reported() {
    let sb = workspace();
    let first = linear(
        &sb,
        &Mock::start(replies()),
        &["cache", "refresh", "--json"],
    );
    assert_eq!(code(&first), 0);
    let first = stdout_json(&first);

    let second = linear(
        &sb,
        &Mock::start(replies()),
        &["cache", "refresh", "--json"],
    );
    assert_eq!(code(&second), 0);
    let second = stdout_json(&second);

    assert_eq!(
        first["workspaces"][0]["findings"],
        second["workspaces"][0]["findings"]
    );
    assert_eq!(second["workspaces"][0]["new_findings"], json!([]));
    assert_eq!(read_entry(&sb)["data"]["newFindings"], json!([]));
}

#[test]
fn without_a_selection_every_workspace_is_refreshed_and_the_unreachable_are_named() {
    let sb = workspace();
    // A second workspace that has no credentials.
    let o = sb.run(
        &["workspace", "add", "other", "--url-key", "other"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh", "--json"]);
    assert_eq!(code(&o), 1);
    let out = stdout_json(&o);
    let names: Vec<_> = out["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["workspace"].as_str().unwrap(),
                w["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(names, [("example", "ok"), ("other", "failed")]);
    assert_eq!(out["unreachable"], json!(["other"]));
    assert!(cache_dir(&sb).join("example.json").exists());
    assert!(cache_dir(&sb).join("other.json").exists());

    // --workspace narrows it to one, and then nothing else is touched.
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh", "-w", "example"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn a_refresh_that_is_already_running_is_not_started_twice() {
    let sb = workspace();
    std::fs::create_dir_all(cache_dir(&sb)).unwrap();
    std::fs::write(cache_dir(&sb).join("example.lock"), "").unwrap();

    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
    assert_eq!(stdout_json(&o)["workspaces"][0]["status"], "busy");
    assert!(
        cache_dir(&sb).join("example.lock").exists(),
        "someone else's lock is not ours to remove"
    );
}

#[test]
fn an_entry_of_another_schema_version_is_unusable_and_a_refresh_replaces_it() {
    let sb = workspace();
    std::fs::create_dir_all(cache_dir(&sb)).unwrap();
    std::fs::write(
        entry_file(&sb),
        json!({ "schema_version": 99, "workspace": "example" }).to_string(),
    )
    .unwrap();

    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["freshness"]["state"], "missing");
    assert!(rows[0]["problem"]
        .as_str()
        .unwrap()
        .contains("schema_version 99"));

    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);
    assert_eq!(
        read_entry(&sb)["schemaVersion"],
        linear_core::SCHEMA_VERSION
    );
    assert!(show(&sb, &[])[0]["problem"].is_null());
}

/// What v0.1.0 and v0.2.0 wrote: schema version 2, every key in snake_case.
fn written_by_schema_version_2() -> Value {
    json!({
        "schema_version": 2,
        "workspace": "example",
        "status": "ok",
        "attempted_at": chrono::Utc::now().to_rfc3339(),
        "fetched_at": chrono::Utc::now().to_rfc3339(),
        "failure": null,
        "data": {
            "viewer": {},
            "issues": [],
            "projects": [],
            "findings": [],
            "new_findings": []
        }
    })
}

#[test]
fn an_entry_written_before_the_camel_case_keys_is_discarded_not_an_error() {
    let sb = workspace();
    std::fs::create_dir_all(cache_dir(&sb)).unwrap();
    std::fs::write(entry_file(&sb), written_by_schema_version_2().to_string()).unwrap();

    // It is within the TTL, but this build cannot read it: unknown, with the reason.
    let rows = show(&sb, &[]);
    assert_eq!(rows[0]["freshness"]["state"], "missing");
    assert!(rows[0]["entry"].is_null());
    assert!(
        rows[0]["problem"]
            .as_str()
            .unwrap()
            .contains("schema_version 2"),
        "{}",
        rows[0]["problem"]
    );
    let o = sb.run(&["audit", "--cached"], None, &[]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("unusable cache file"), "{}", stderr(&o));

    // A refresh replaces it, and treats it as no previous snapshot (everything is new).
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["cache", "refresh", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let e = read_entry(&sb);
    assert_eq!(e["schemaVersion"], linear_core::SCHEMA_VERSION);
    assert!(e.get("schema_version").is_none());
    assert_eq!(
        e["data"]["newFindings"].as_array().unwrap().len(),
        e["data"]["findings"].as_array().unwrap().len()
    );
    assert!(show(&sb, &[])[0]["problem"].is_null());
}

#[test]
fn a_file_that_is_not_json_is_unusable_not_a_crash() {
    let sb = workspace();
    std::fs::create_dir_all(cache_dir(&sb)).unwrap();
    std::fs::write(entry_file(&sb), "{ not json").unwrap();
    let rows = show(&sb, &[]);
    assert!(rows[0]["problem"]
        .as_str()
        .unwrap()
        .contains("not valid JSON"));
}

#[test]
fn clear_removes_one_entry_or_all_of_them() {
    let sb = workspace();
    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);
    // An entry of a workspace that is no longer configured.
    std::fs::write(cache_dir(&sb).join("gone.json"), "{}").unwrap();
    // Something that is not ours stays.
    std::fs::write(cache_dir(&sb).join("notes.txt"), "x").unwrap();

    let o = sb.run(&["cache", "clear", "-w", "example", "--json"], None, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["cleared"], json!(["example"]));
    assert!(!entry_file(&sb).exists());
    assert!(cache_dir(&sb).join("gone.json").exists());

    let o = sb.run(&["cache", "clear", "--json"], None, &[]);
    assert_eq!(stdout_json(&o)["cleared"], json!(["gone"]));
    assert!(!cache_dir(&sb).join("gone.json").exists());
    assert!(cache_dir(&sb).join("notes.txt").exists());

    let o = sb.run(&["cache", "clear"], None, &[]);
    assert!(stdout(&o).contains("Nothing was cached"), "{}", stdout(&o));
}

#[test]
fn the_cache_location_follows_xdg_cache_home() {
    let sb = workspace();
    let xdg = sb.root.path().join("xdg");
    let mock = Mock::start(replies());
    let o = sb.run(
        &["cache", "refresh"],
        Some(&mock),
        &[
            ("LINEAR_API_KEY_EXAMPLE", KEY),
            ("XDG_CACHE_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(xdg.join("linear").join("example.json").exists());
    assert!(!cache_dir(&sb).exists());
}

#[test]
fn a_workspace_name_cannot_point_outside_the_cache() {
    let sb = workspace();
    let o = sb.run(&["cache", "show", "-w", "../evil"], None, &[]);
    assert_ne!(code(&o), 0);
    assert!(!sb.root.path().join("evil.json").exists());
}
