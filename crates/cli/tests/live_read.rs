//! Read commands against the real sandbox workspace. Ignored by default; they
//! need the sandbox credentials in the environment and the seeded data
//! (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_read -- --ignored
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

/// Run `linear <args> --json` and parse stdout.
fn json(sb: &Sandbox, key: &str, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let o = sb.run(&full, None, &[("LINEAR_API_KEY_SANDBOX", key)]);
    assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    assert!(!stdout(&o).contains(key) && !stderr(&o).contains(key));
    serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stdout(&o)))
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn my_started_issues_are_listed() {
    let (sb, key) = sandbox();
    let v = json(
        &sb,
        &key,
        &[
            "issue",
            "list",
            "--assignee",
            "me",
            "--state-type",
            "started",
        ],
    );
    let rows = v.as_array().unwrap();
    assert!(
        !rows.is_empty(),
        "no started issue is assigned to the key's owner"
    );
    for r in rows {
        assert_eq!(r["state"]["type"], "started");
        assert_eq!(r["assignee"]["isMe"], true);
        assert_eq!(r["workspace"], "sandbox");
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn an_issue_is_found_by_its_origin_url_and_viewed() {
    let (sb, key) = sandbox();
    let v = json(
        &sb,
        &key,
        &[
            "issue",
            "list",
            "--source-url",
            "https://example.com/source/1",
        ],
    );
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{v}");
    let identifier = rows[0]["identifier"].as_str().unwrap().to_owned();
    assert_eq!(rows[0]["sourceUrl"], "https://example.com/source/1");

    let view = json(&sb, &key, &["issue", "view", &identifier]);
    assert_eq!(view["identifier"], identifier.as_str());
    assert!(!view["description"].as_str().unwrap_or("").is_empty());
    assert!(!view["comments"]["nodes"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn paging_with_a_small_limit_and_with_all_agree() {
    let (sb, key) = sandbox();
    let all = json(&sb, &key, &["issue", "list", "--all"]);
    let total = all.as_array().unwrap().len();
    assert!(total >= 3, "the sandbox is seeded with at least 3 issues");

    let o = sb.run(
        &["issue", "list", "--limit", "2", "--quiet"],
        None,
        &[("LINEAR_API_KEY_SANDBOX", &key)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).lines().count(), 2);
    assert!(stderr(&o).contains("more exist"), "{}", stderr(&o));
}
