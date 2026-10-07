//! `cycle list` and `cycle view` against the real sandbox workspace. Ignored by
//! default; they need the sandbox credentials in the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_cycle -- --ignored
//! ```
//!
//! The sandbox team has cycles turned off and the CLI cannot create one, so
//! this only proves that the queries are accepted by Linear (filter and
//! pagination variables included) and that a missing number is a usage error.
//! The shape of a real cycle (state flags, progress, team, issues) was checked
//! once with the same commands, read-only, against a workspace that has cycles.

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

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn cycle_list_and_view_are_accepted_by_linear() {
    let (sb, key) = sandbox();
    let env = [("LINEAR_API_KEY_SANDBOX", key.as_str())];

    for state in ["active", "upcoming", "past"] {
        let o = sb.run(&["cycle", "list", "--state", state, "--json"], None, &env);
        assert_eq!(code(&o), 0, "{state}: {}", stderr(&o));
        let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert!(v.is_array(), "{state}: {v}");
    }

    let o = sb.run(&["cycle", "view", "1"], None, &env);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("has no cycle #1"), "{}", stderr(&o));
    assert!(!stdout(&o).contains(&key) && !stderr(&o).contains(&key));
}
