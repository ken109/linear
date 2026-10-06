//! Checks against a real Linear workspace. Ignored by default; they need the
//! sandbox credentials in the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live -- --ignored
//! ```

mod common;
use common::*;

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn whoami_against_the_sandbox_workspace() {
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

    let o = sb.run(
        &["workspace", "whoami", "--json"],
        None,
        &[("LINEAR_API_KEY_SANDBOX", &key)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    // Assert non-empty values, so an empty-but-successful response cannot pass.
    assert_eq!(v["organization"]["urlKey"], "ken109-sandbox");
    assert!(!v["user"]["id"].as_str().unwrap_or("").is_empty());
    assert!(!v["user"]["email"].as_str().unwrap_or("").is_empty());
    assert!(!stdout(&o).contains(&key) && !stderr(&o).contains(&key));
}
