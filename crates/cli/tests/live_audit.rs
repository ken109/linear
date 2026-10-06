//! `cache` and `audit` against the real sandbox workspace. Ignored by default;
//! they need the sandbox credentials in the environment and the seeded data
//! (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_audit -- --ignored
//! ```
//!
//! Every check asserts a non-empty result, so an empty-but-successful
//! response cannot pass.

mod common;

use common::*;

/// A sandbox with the `sandbox` workspace configured, and the key to pass.
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

/// Run `linear <args>` with the key and return the output.
fn run(sb: &Sandbox, key: &str, args: &[&str]) -> std::process::Output {
    let o = sb.run(args, None, &[("LINEAR_API_KEY_SANDBOX", key)]);
    assert!(
        !stdout(&o).contains(key) && !stderr(&o).contains(key),
        "the key must not be printed"
    );
    o
}

/// Run `linear <args> --json`, expect success and parse stdout.
fn json(sb: &Sandbox, key: &str, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let o = run(sb, key, &full);
    assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stdout(&o)))
}

fn entry(sb: &Sandbox) -> serde_json::Value {
    let path = sb.root.path().join(".cache/linear/sandbox.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the cache entry")).unwrap()
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_refresh_caches_what_is_in_progress_and_survives_a_failed_one() {
    let (sb, key) = sandbox();
    let out = json(&sb, &key, &["cache", "refresh"]);
    assert_eq!(out["workspaces"][0]["status"], "ok", "{out}");
    assert_eq!(out["unreachable"], serde_json::json!([]));

    let e = entry(&sb);
    assert_eq!(e["data"]["viewer"]["isMe"], true);
    let issues = e["data"]["issues"].as_array().unwrap();
    assert!(
        !issues.is_empty(),
        "an issue assigned to the key's owner is In Progress"
    );
    for i in issues {
        assert_eq!(i["state"]["type"], "started");
        assert_eq!(i["assignee"]["isMe"], true);
    }
    assert!(!e["data"]["projects"].as_array().unwrap().is_empty());

    let rows = json(&sb, &key, &["cache", "show"]);
    assert_eq!(rows[0]["freshness"]["state"], "fresh");

    // A refresh with a key Linear rejects fails, and keeps what was there.
    let o = run(
        &sb,
        "lin_api_not_a_real_key",
        &["cache", "refresh", "--json"],
    );
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    let after = entry(&sb);
    assert_eq!(after["status"], "failed");
    assert_eq!(after["data"], e["data"]);
    assert!(after["failure"]["message"].is_string());
}
