//! `linear issue create|update --priority|--estimate|--parent|--cycle`, the matching
//! `issue list` filters and `issue search`, against the real sandbox workspace. Ignored by
//! default; it needs the sandbox credentials in the environment and the seeded `Fixture
//! Project` (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_issue_attributes -- --ignored
//! ```
//!
//! It creates two issues with a unique word in the title (a parent and a child), changes
//! and reads back every attribute, runs each update a second time to see that nothing is
//! sent, lists and searches for them, and cancels both at the end (the CLI has no delete).
//! The sandbox team has no cycles, so `--cycle` is exercised only as far as an empty team
//! allows: a number that no cycle has is refused, `none` is a no-op, and the list filters
//! are accepted by Linear.

mod common;

use common::*;
use serde_json::Value;
use std::time::Duration;

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

    /// `issue update <id> ...` and the `changed` list it reports.
    fn update(&self, id: &str, extra: &[&str]) -> Vec<String> {
        let mut args = vec!["issue", "update", id];
        args.extend_from_slice(extra);
        let v = self.json(&args);
        v["changed"]
            .as_array()
            .unwrap_or_else(|| panic!("no `changed` in {v}"))
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect()
    }

    fn view(&self, id: &str) -> Value {
        self.json(&["issue", "view", id])
    }

    /// The identifiers `issue list <filters>` or `issue search <query>` returns.
    fn identifiers(&self, args: &[&str]) -> Vec<String> {
        self.json(args)
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["identifier"].as_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn priority_estimate_parent_filters_and_search_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let word = format!("livecheck{stamp}");

    let create = |title: &str, extra: &[&str]| -> String {
        let mut args = vec![
            "issue",
            "create",
            "--title",
            title,
            "--project",
            "Fixture Project",
        ];
        args.extend_from_slice(extra);
        live.json(&args)["identifier"].as_str().unwrap().to_owned()
    };
    let parent = create(&format!("{word} parent"), &[]);
    let child = create(
        &format!("{word} child"),
        &["--priority", "high", "--estimate", "2", "--parent", &parent],
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // What the create set, read back.
        let v = live.view(&child);
        assert_eq!(v["priority"], 2.0, "{v}");
        assert_eq!(v["priorityLabel"], "High");
        assert_eq!(v["estimate"], 2.0);
        assert_eq!(v["parent"]["identifier"], parent.as_str());

        // Each update changes its field once; the same update again sends nothing.
        assert_eq!(live.update(&child, &["--priority", "urgent"]), ["priority"]);
        assert_eq!(live.view(&child)["priorityLabel"], "Urgent");
        assert_eq!(
            live.json(&["issue", "list", "--parent", &parent])[0]["priority"],
            1.0
        );
        assert_eq!(
            live.update(&child, &["--priority", "urgent"]),
            Vec::<String>::new()
        );
        assert_eq!(live.update(&child, &["--estimate", "3"]), ["estimate"]);
        assert_eq!(live.view(&child)["estimate"], 3.0);
        assert_eq!(
            live.update(&child, &["--estimate", "3"]),
            Vec::<String>::new()
        );
        assert_eq!(
            live.update(&child, &["--parent", &parent]),
            Vec::<String>::new()
        );

        // The list filters find it.
        let child_only = [child.clone()];
        assert_eq!(
            live.identifiers(&["issue", "list", "--parent", &parent]),
            child_only
        );
        let urgent = live.identifiers(&["issue", "list", "--priority", "urgent", "--all"]);
        assert!(urgent.contains(&child), "{urgent:?}");
        let recent = live.identifiers(&["issue", "list", "--updated-after", "1d", "--all"]);
        assert!(
            recent.contains(&child) && recent.contains(&parent),
            "{recent:?}"
        );
        let top_level = live.identifiers(&["issue", "list", "--parent", "none", "--all"]);
        assert!(top_level.contains(&parent) && !top_level.contains(&child));
        let uncycled = live.identifiers(&["issue", "list", "--cycle", "none", "--all"]);
        assert!(uncycled.contains(&child), "no cycle: {uncycled:?}");
        // A cycle number that nothing has lists nothing (Linear accepts the filter).
        assert!(live
            .identifiers(&["issue", "list", "--cycle", "9999", "--all"])
            .is_empty());

        // Cycles: the sandbox team has none, so a number is refused and `none` changes nothing.
        let o = live.run(&["issue", "update", &child, "--cycle", "1"]);
        assert_eq!(code(&o), 2, "{}", stderr(&o));
        assert!(stderr(&o).contains("has no cycle #1"), "{}", stderr(&o));
        assert_eq!(
            live.update(&child, &["--cycle", "none"]),
            Vec::<String>::new()
        );

        // Clearing: the estimate and the parent go; again is a no-op.
        assert_eq!(
            live.update(&child, &["--estimate", "none", "--parent", "none"]),
            ["estimate", "parent"]
        );
        let v = live.view(&child);
        assert!(v["estimate"].is_null() && v["parent"].is_null(), "{v}");
        assert_eq!(
            live.update(&child, &["--estimate", "none", "--parent", "none"]),
            Vec::<String>::new()
        );
        assert_eq!(live.update(&child, &["--priority", "none"]), ["priority"]);
        assert_eq!(live.view(&child)["priority"], 0.0);

        // Search finds both by the unique word (it is a full-text and vector search, so it may
        // return other issues too). Linear indexes a little after the write, so look for a while.
        let mut found = Vec::new();
        for _ in 0..12 {
            found = live.identifiers(&["issue", "search", &word, "--team", "SAND"]);
            if found.contains(&parent) && found.contains(&child) {
                break;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        assert!(
            found.contains(&parent) && found.contains(&child),
            "search for {word}: {found:?}"
        );
        // The filters narrow it: neither issue is completed, and the child is still open.
        let completed = live.identifiers(&["issue", "search", &word, "--state-type", "completed"]);
        assert!(
            !completed.contains(&parent) && !completed.contains(&child),
            "{completed:?}"
        );
        let open = live.identifiers(&["issue", "search", &word, "--open"]);
        assert!(open.contains(&child), "{open:?}");
    }));

    // Clean up even when a check failed: cancel what this run made.
    for id in [&child, &parent] {
        let o = live.run(&["issue", "update", id, "--state", "Canceled"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}
