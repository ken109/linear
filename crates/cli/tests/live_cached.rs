//! `--cached` reads and `linear status` against the real sandbox workspace.
//! Ignored by default; they need the sandbox credentials in the environment and
//! the seeded data (see `tests/fixtures/README.md`). They only read.
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_cached -- --ignored --test-threads=1
//! ```
//!
//! Every check asserts a non-empty result, so an empty-but-successful
//! response cannot pass.

mod common;

use common::*;

fn sandbox() -> (Sandbox, String) {
    let key = std::env::var("LINEAR_API_KEY_SANDBOX").expect("LINEAR_API_KEY_SANDBOX");
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "sandbox",
            "--url-key",
            "ken109-sandbox",
            "--team",
            "SAND",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    (sb, key)
}

/// `linear <args>` with the key.
fn run(sb: &Sandbox, key: &str, args: &[&str]) -> std::process::Output {
    let o = sb.run(args, None, &[("LINEAR_API_KEY_SANDBOX", key)]);
    assert!(
        !stdout(&o).contains(key) && !stderr(&o).contains(key),
        "the key must not be printed"
    );
    o
}

/// `linear <args> --json`, expecting success.
fn json(sb: &Sandbox, key: &str, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let o = run(sb, key, &full);
    assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stdout(&o)))
}

/// `linear <args>` without the key: a read of the cache file alone.
fn offline(sb: &Sandbox, args: &[&str]) -> std::process::Output {
    sb.run(args, None, &[])
}

fn identifiers(rows: &serde_json::Value) -> Vec<String> {
    let mut ids: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["identifier"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn cached_reads_follow_a_refresh_and_go_unknown_when_it_is_stale() {
    let (sb, key) = sandbox();

    // Nothing cached yet: unknown, not an empty list.
    let o = offline(&sb, &["issue", "list", "--cached"]);
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    assert!(stderr(&o).contains("nothing cached"), "{}", stderr(&o));
    let o = offline(&sb, &["status"]);
    assert_eq!(stdout(&o), "sandbox: unknown (nothing cached)\n");

    let refreshed = json(&sb, &key, &["cache", "refresh"]);
    assert_eq!(refreshed["unreachable"], serde_json::json!([]));

    // The cached list is the live list of the same range.
    let live = json(
        &sb,
        &key,
        &[
            "issue",
            "list",
            "--assignee",
            "me",
            "--state-type",
            "started",
            "--all",
        ],
    );
    assert!(
        !live.as_array().unwrap().is_empty(),
        "something is In Progress"
    );
    let o = offline(&sb, &["issue", "list", "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let cached: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(identifiers(&cached), identifiers(&live));
    assert!(
        stderr(&o).contains("from the cache of sandbox"),
        "{}",
        stderr(&o)
    );

    // One issue, and the project it belongs to.
    let first = cached[0]["identifier"].as_str().unwrap();
    let o = offline(&sb, &["issue", "view", first, "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout(&o)).unwrap()["identifier"],
        first
    );
    let o = offline(&sb, &["project", "list", "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let projects: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let projects = projects.as_array().unwrap();
    assert!(!projects.is_empty(), "an In Progress issue has a project");
    let slug = projects[0]["slugId"].as_str().unwrap();
    let o = offline(&sb, &["project", "view", slug, "--cached", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // The live project is the same one.
    let live_project = json(&sb, &key, &["project", "view", slug]);
    assert_eq!(live_project["slugId"], slug);
    assert_eq!(live_project["name"], projects[0]["name"]);

    // The status line counts what the snapshot holds.
    let o = offline(&sb, &["status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let line = stdout(&o);
    assert!(
        line.starts_with(&format!(
            "sandbox: {} in progress, ",
            live.as_array().unwrap().len()
        )),
        "{line}"
    );

    // A filter the cache does not hold is refused, whatever the cache says.
    let o = offline(&sb, &["issue", "list", "--cached", "--team", "SAND"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    // Past the TTL (0 seconds, after the clock has moved) everything goes unknown.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    for args in [
        &["issue", "list", "--cached", "--ttl", "0"][..],
        &["issue", "view", first, "--cached", "--ttl", "0"],
        &["project", "list", "--cached", "--ttl", "0"],
        &["project", "view", slug, "--cached", "--ttl", "0"],
    ] {
        let o = offline(&sb, args);
        assert_eq!(code(&o), 1, "{args:?}: {}", stderr(&o));
        assert!(stdout(&o).is_empty(), "{args:?}");
        assert!(
            stderr(&o).contains("past the 0s TTL"),
            "{args:?}: {}",
            stderr(&o)
        );
    }
    let o = offline(&sb, &["status", "--ttl", "0"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).starts_with("sandbox: unknown (snapshot ")
            && !stdout(&o).contains("in progress"),
        "{}",
        stdout(&o)
    );

    // A refresh makes it known again.
    json(&sb, &key, &["cache", "refresh"]);
    assert_eq!(code(&offline(&sb, &["issue", "list", "--cached"])), 0);
}
