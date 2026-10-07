//! Webhook commands against the real sandbox workspace. Ignored by default;
//! they need the sandbox credentials in the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_webhook -- --ignored
//! ```
//!
//! The test creates one webhook under a unique label, pointing at a URL that
//! nothing listens on (`.invalid` never resolves), and deletes it when it ends,
//! even on failure. Nothing is ever delivered.

mod common;

use common::*;
use serde_json::Value;

struct Live {
    sb: Sandbox,
    key: String,
    /// Ids to delete when the test ends.
    created: std::cell::RefCell<Vec<String>>,
}

impl Live {
    fn new() -> Live {
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
        Live {
            sb,
            key,
            created: Default::default(),
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let o = self
            .sb
            .run(args, None, &[("LINEAR_API_KEY_SANDBOX", &self.key)]);
        assert!(!stdout(&o).contains(&self.key) && !stderr(&o).contains(&self.key));
        o
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut full = args.to_vec();
        full.push("--json");
        let o = self.run(&full);
        assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
        serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        for id in self.created.borrow().iter() {
            let _ = self.run(&["webhook", "delete", id]);
        }
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_webhook_is_created_listed_verified_against_and_deleted() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let label = format!("live webhook {stamp}");
    let url = format!("https://webhook-{stamp}.invalid/linear");

    let made = live.json(&[
        "webhook",
        "create",
        "--url",
        &url,
        "--resource-types",
        "Issue,Comment",
        "--team",
        "SAND",
        "--label",
        &label,
    ]);
    let id = made["id"].as_str().unwrap().to_owned();
    live.created.borrow_mut().push(id.clone());
    assert_eq!(made["workspace"], "sandbox");
    assert_eq!(made["label"], label.as_str());
    assert_eq!(made["url"], url.as_str());
    assert_eq!(made["team"]["key"], "SAND");
    assert_eq!(made["enabled"], true);
    let mut types: Vec<&str> = made["resourceTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    types.sort_unstable();
    assert_eq!(types, ["Comment", "Issue"]);
    let secret = made["secret"].as_str().expect("create prints the secret");
    assert!(!secret.is_empty());

    // The list has it, and never shows a secret.
    let all = live.json(&["webhook", "list", "--all"]);
    let listed = all
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == id.as_str())
        .expect("the new webhook is listed");
    assert_eq!(listed["label"], label.as_str());
    assert!(all
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w.get("secret").is_none()));

    // A delivery signed with that secret verifies; one signed with another does not.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let body = format!(r#"{{"action":"create","type":"Issue","webhookTimestamp":{now}}}"#);
    let signature = linear_core::webhook::sign(secret, &body);
    let verify = |signature: &str| {
        live.sb.run_stdin(
            &["webhook", "verify", "--signature", signature, "--json"],
            None,
            &[("LINEAR_WEBHOOK_SECRET", secret)],
            Some(&body),
        )
    };
    let o = verify(&signature);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("\"valid\""), "{}", stdout(&o));
    let o = verify(&linear_core::webhook::sign("another secret", &body));
    assert_eq!(code(&o), 1, "{}", stderr(&o));

    // Delete by label; it is gone, and deleting it again finds nothing.
    let gone = live.json(&["webhook", "delete", &label]);
    assert_eq!(gone["deleted"], true);
    assert_eq!(gone["id"], id.as_str());
    live.created.borrow_mut().clear();
    let after = live.json(&["webhook", "list", "--all"]);
    assert!(after
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["id"] != id.as_str()));
    let o = live.run(&["webhook", "delete", &label]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}
