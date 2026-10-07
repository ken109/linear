//! `comment update|delete` and `issue relate|unrelate` against the real sandbox workspace.
//! Ignored by default; they need the sandbox credentials in the environment and `Fixture
//! Project` (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_comment_relation -- --ignored
//! ```
//!
//! The scenario creates three issues with a unique title, and cancels them at the end (the CLI
//! has no issue delete), even when an assertion fails; the relations and the comment are removed
//! by the scenario itself, so repeated runs leave only canceled issues behind.

mod common;

use common::*;
use serde_json::Value;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

struct Live {
    sb: Sandbox,
    key: String,
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
        Live { sb, key }
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

    fn create(&self, title: &str, body_file: &str) -> String {
        let v = self.json(&[
            "issue",
            "create",
            "--title",
            title,
            "--project",
            "Fixture Project",
            "--body-file",
            body_file,
        ]);
        v["identifier"].as_str().unwrap().to_owned()
    }
}

/// The identifiers of the issues at the other end of `nodes` (a `relations` or `inverseRelations`).
fn ends(nodes: &Value, of: &str, kind: &str) -> Vec<String> {
    nodes["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["type"] == kind)
        .map(|r| r[of]["identifier"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn relations_and_comments_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = live.sb.cwd().join("body.md");
    std::fs::write(&body, "Live relation check.\n").unwrap();
    let body = body.to_string_lossy().into_owned();

    let ids: Vec<String> = ["A", "B", "C"]
        .iter()
        .map(|t| live.create(&format!("live relation {t} {stamp}"), &body))
        .collect();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        scenario(&live, &ids, &stamp.to_string())
    }));
    for id in &ids {
        let o = live.run(&["issue", "update", id, "--state", "Canceled"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
    if let Err(panic) = outcome {
        resume_unwind(panic);
    }
}

fn scenario(live: &Live, ids: &[String], stamp: &str) {
    let (a, b, c) = (&ids[0], &ids[1], &ids[2]);

    // blocks: made, found again, and shown on both issues.
    let r = live.json(&["issue", "relate", a, "--blocks", b]);
    assert_eq!(r["type"], "blocks");
    assert_eq!(r["alreadyRelated"], false);
    let again = live.json(&["issue", "relate", a, "--blocks", b]);
    assert_eq!(again["alreadyRelated"], true);
    assert_eq!(again["id"], r["id"]);
    let va = live.json(&["issue", "view", a]);
    assert_eq!(
        ends(&va["relations"], "relatedIssue", "blocks"),
        [b.as_str()]
    );
    let vb = live.json(&["issue", "view", b]);
    assert_eq!(
        ends(&vb["inverseRelations"], "issue", "blocks"),
        [a.as_str()]
    );
    // The reverse is another statement, not "already there".
    let rev = live.run(&["issue", "unrelate", b, "--blocks", a, "--json"]);
    assert_eq!(code(&rev), 0, "{}", stderr(&rev));
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&rev)).unwrap()["removed"],
        serde_json::json!([])
    );

    // related has no direction: removed from the other end.
    live.json(&["issue", "relate", a, "--related", c]);
    let gone = live.json(&["issue", "unrelate", c, "--related", a]);
    assert_eq!(gone["removed"].as_array().unwrap().len(), 1);
    let va = live.json(&["issue", "view", a]);
    assert!(ends(&va["relations"], "relatedIssue", "related").is_empty());

    // duplicate: Linear moves the issue to Duplicate, and back when the relation goes.
    live.json(&["issue", "relate", c, "--duplicate", a]);
    let vc = live.json(&["issue", "view", c]);
    assert_eq!(vc["state"]["type"], "duplicate");
    live.json(&["issue", "unrelate", c, "--duplicate", a]);
    let vc = live.json(&["issue", "view", c]);
    assert_ne!(vc["state"]["type"], "duplicate");

    // unrelate: removed once, then there is nothing to remove.
    let removed = live.json(&["issue", "unrelate", a, "--blocks", b]);
    assert_eq!(removed["removed"].as_array().unwrap().len(), 1);
    let none = live.json(&["issue", "unrelate", a, "--blocks", b]);
    assert_eq!(none["removed"], serde_json::json!([]));

    // A relation to oneself, or to an issue that does not exist, is refused.
    assert_eq!(code(&live.run(&["issue", "relate", a, "--blocks", a])), 2);
    assert_eq!(
        code(&live.run(&["issue", "relate", a, "--blocks", "SAND-99999999"])),
        1
    );

    // Comments: update, the same text again, a delete that needs --yes, then the delete.
    let first = live.sb.cwd().join("first.md");
    std::fs::write(&first, format!("first {stamp}\n")).unwrap();
    let second = live.sb.cwd().join("second.md");
    std::fs::write(&second, format!("second {stamp}\n")).unwrap();
    let made = live.json(&[
        "issue",
        "comment",
        a,
        "--body-file",
        &first.to_string_lossy(),
    ]);
    let id = made["id"].as_str().unwrap().to_owned();

    let u = live.json(&[
        "comment",
        "update",
        &id,
        "--body-file",
        &second.to_string_lossy(),
    ]);
    assert_eq!(u["changed"], true);
    assert_eq!(u["issue"], a.as_str());
    let va = live.json(&["issue", "view", a]);
    let bodies: Vec<&str> = va["comments"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, [format!("second {stamp}")]);
    let same = live.json(&[
        "comment",
        "update",
        &id,
        "--body-file",
        &second.to_string_lossy(),
    ]);
    assert_eq!(same["changed"], false);

    let o = live.run(&["comment", "delete", &id]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_eq!(
        live.json(&["issue", "view", a])["comments"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "a delete without --yes must not delete"
    );
    let d = live.json(&["comment", "delete", &id, "--yes"]);
    assert_eq!(d["deleted"], true);
    assert!(live.json(&["issue", "view", a])["comments"]["nodes"]
        .as_array()
        .unwrap()
        .is_empty());
    // Gone for good: Linear no longer knows the comment.
    let o = live.run(&[
        "comment",
        "update",
        &id,
        "--body-file",
        &first.to_string_lossy(),
    ]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
}
