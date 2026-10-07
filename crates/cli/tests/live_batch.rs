//! `linear issue batch` against the real sandbox workspace. Ignored by default; it needs the
//! sandbox credentials and the seeded data (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_batch -- --ignored
//! ```
//!
//! It makes two issues to write to, runs a batch that succeeds (two creations and an update)
//! and reads everything back, then a batch whose last item Linear refuses (an estimate outside
//! the team's scale) and checks that the earlier items were undone: the issue it created is
//! gone and the issue it updated has its old priority again. Every issue it made is canceled
//! at the end.

mod common;

use common::*;
use serde_json::{json, Value};

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
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\nrules = [\"source-attachment\"]\nsource_kinds = [\"slack\"]\n");
        std::fs::write(&path, text).unwrap();
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

    fn batch(&self, batch: &Value, extra: &[&str]) -> std::process::Output {
        let path = self.sb.cwd().join("batch.json");
        std::fs::write(&path, batch.to_string()).unwrap();
        let mut args = vec!["issue", "batch", "--file", path.to_str().unwrap(), "--json"];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn make(&self, title: &str, source: &str) -> String {
        let v = self.json(&[
            "issue",
            "create",
            "--title",
            title,
            "--project",
            "Fixture Project",
            "--source",
            source,
            "--meta",
            "kind=slack",
        ]);
        v["identifier"].as_str().unwrap().to_owned()
    }

    fn with_source(&self, source: &str) -> Vec<Value> {
        self.json(&["issue", "list", "--source-url", source, "--all"])
            .as_array()
            .unwrap()
            .clone()
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_batch_is_all_or_nothing_against_the_sandbox() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let url = |name: &str| format!("https://example.com/linear-cli-live/{stamp}/{name}");
    let (base, other, a, b, c) = (url("base"), url("other"), url("a"), url("b"), url("c"));

    let x = live.make("live batch base", &base);
    let y = live.make("live batch other", &other);
    let mut made = vec![x.clone(), y.clone()];

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let before = live.json(&["issue", "view", &x]);

        // A refusal sends nothing: the source of item 2 is the one X already carries elsewhere.
        // (Here: an unknown project in the last item.)
        let o = live.batch(
            &json!({ "issues": [
                { "op": "create", "title": "live batch a", "project": "Fixture Project",
                  "source": a, "meta": { "kind": "slack" } },
                { "op": "create", "title": "live batch lost", "project": "No such project",
                  "source": b, "meta": { "kind": "slack" } },
            ] }),
            &[],
        );
        assert_eq!(code(&o), 2, "{}", stderr(&o));
        assert!(stderr(&o).contains("issues[1]"), "{}", stderr(&o));
        assert!(live.with_source(&a).is_empty(), "item 0 was not sent");

        // A dry run changes nothing.
        let o = live.batch(
            &json!({ "issues": [
                { "op": "create", "title": "live batch a", "project": "Fixture Project",
                  "source": a, "meta": { "kind": "slack" } },
                { "op": "update", "issue": x, "priority": "urgent" },
            ] }),
            &["--dry-run"],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let plan: Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(plan["dryRun"], true);
        assert!(live.with_source(&a).is_empty());
        assert_eq!(
            live.json(&["issue", "view", &x])["priority"],
            before["priority"]
        );

        // A batch that works: two creations and an update.
        let o = live.batch(
            &json!({ "issues": [
                { "op": "create", "title": "live batch a", "project": "Fixture Project",
                  "source": a, "meta": { "kind": "slack" }, "priority": "high" },
                { "op": "update", "issue": x, "priority": "urgent" },
                { "op": "create", "title": "live batch b", "project": "Fixture Project",
                  "source": b, "meta": { "kind": "slack" } },
            ] }),
            &[],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let done: Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(
            (done["created"].clone(), done["updated"].clone()),
            (json!(2), json!(1))
        );
        for r in done["results"].as_array().unwrap() {
            if r["op"] == "create" {
                made.push(r["identifier"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(live.with_source(&a).len(), 1);
        assert_eq!(live.with_source(&b).len(), 1);
        assert_eq!(live.json(&["issue", "view", &x])["priority"], 1.0);

        // The same batch again makes nothing new: the sources are on issues already.
        let o = live.batch(
            &json!({ "issues": [
                { "op": "create", "title": "live batch a", "project": "Fixture Project",
                  "source": a, "meta": { "kind": "slack" } },
                { "op": "update", "issue": x, "priority": "urgent" },
            ] }),
            &[],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let again: Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(again["created"], 0);
        assert_eq!(again["results"][0]["existing"], true);
        assert_eq!(live.with_source(&a).len(), 1);

        // A batch whose last item Linear refuses: an estimate outside the team's scale. The
        // update of X and the creation of C are undone.
        let o = live.batch(
            &json!({ "issues": [
                { "op": "update", "issue": x, "priority": "low" },
                { "op": "create", "title": "live batch c", "project": "Fixture Project",
                  "source": c, "meta": { "kind": "slack" } },
                { "op": "update", "issue": y, "estimate": 99999 },
            ] }),
            &[],
        );
        assert_eq!(code(&o), 1, "{}", stderr(&o));
        let err = stderr(&o);
        assert!(
            err.contains("issues[2]") && err.contains("rolled back"),
            "{err}"
        );
        assert!(
            live.with_source(&c).is_empty(),
            "the issue it created is gone"
        );
        assert_eq!(
            live.json(&["issue", "view", &x])["priority"],
            1.0,
            "X has the priority it had before the batch"
        );
        assert!(live.json(&["issue", "view", &y])["estimate"].is_null());
    }));

    for id in &made {
        let o = live.run(&["issue", "update", id, "--state", "Canceled"]);
        assert_eq!(code(&o), 0, "{id}: {}", stderr(&o));
    }
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}
