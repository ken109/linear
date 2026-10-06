//! Source attachment metadata against the real sandbox workspace. Ignored by
//! default; it needs the sandbox credentials in the environment and the seeded
//! `Fixture Project` (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_attachment_meta -- --ignored
//! ```
//!
//! It creates one issue with a unique source URL and cancels it at the end
//! (the CLI has no delete), so repeated runs leave only canceled issues behind.

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
        text.push_str(
            "\nrules = [\"source-attachment\"]\nsource_kinds = [\"life-decision\", \"slack\"]\n",
        );
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

    /// `issue create` with the source and these `--meta` pairs.
    fn create(&self, source: &str, meta: &[&str]) -> Value {
        let mut args = vec![
            "issue",
            "create",
            "--title",
            "live attachment metadata",
            "--project",
            "Fixture Project",
            "--source",
            source,
            "--source-title",
            "Live source",
        ];
        for pair in meta {
            args.extend(["--meta", pair]);
        }
        self.json(&args)
    }

    /// The source attachment of an issue as `issue view --json` shows it.
    fn source_attachment(&self, identifier: &str, source: &str) -> Value {
        let viewed = self.json(&["issue", "view", identifier]);
        viewed["attachments"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["url"] == source)
            .unwrap_or_else(|| panic!("no attachment {source} on {identifier}: {viewed}"))
            .clone()
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn source_attachment_metadata_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("https://example.com/linear-cli-live/{stamp}/meta");

    // Create: the metadata is on the source attachment, numbers as numbers and
    // `str:` as text, and `issue view --json` reads it back with the other
    // attachment fields.
    let created = live.create(
        &source,
        &[
            "kind=life-decision",
            "ticket=42",
            "ratio=0.5",
            "build=str:123",
        ],
    );
    assert_eq!(created["existing"], false);
    assert_eq!(created["metadataUpdated"], false);
    let id = created["identifier"].as_str().unwrap().to_owned();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let a = live.source_attachment(&id, &source);
        assert_eq!(a["title"], "Live source");
        assert_eq!(
            a["metadata"],
            json!({ "kind": "life-decision", "ticket": 42, "ratio": 0.5, "build": "123" })
        );
        assert!(
            a.get("subtitle").is_some() && a.get("sourceType").is_some(),
            "{a}"
        );

        // The same source and the same metadata: the issue comes back, nothing is sent.
        let same = live.create(
            &source,
            &[
                "kind=life-decision",
                "ticket=42",
                "ratio=0.5",
                "build=str:123",
            ],
        );
        assert_eq!(same["existing"], true);
        assert_eq!(same["identifier"], id.as_str());
        assert_eq!(same["metadataUpdated"], false);

        // The same source with other metadata: no new issue, the stored metadata is replaced.
        let changed = live.create(&source, &["kind=slack", "ticket=43"]);
        assert_eq!(changed["existing"], true);
        assert_eq!(changed["identifier"], id.as_str());
        assert_eq!(changed["metadataUpdated"], true);
        let a = live.source_attachment(&id, &source);
        assert_eq!(a["metadata"], json!({ "kind": "slack", "ticket": 43 }));
        // Title is untouched, and there is still exactly one attachment with the URL.
        assert_eq!(a["title"], "Live source");
        let viewed = live.json(&["issue", "view", &id]);
        let with_url = viewed["attachments"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["url"] == source.as_str())
            .count();
        assert_eq!(with_url, 1);

        // And again with what is now stored: nothing to do.
        let settled = live.create(&source, &["kind=slack", "ticket=43"]);
        assert_eq!(settled["existing"], true);
        assert_eq!(settled["metadataUpdated"], false);

        // source_kinds: a kind outside the list is refused before anything is created.
        let other = format!("https://example.com/linear-cli-live/{stamp}/refused");
        let o = live.run(&[
            "issue",
            "create",
            "--title",
            "never",
            "--project",
            "Fixture Project",
            "--source",
            &other,
            "--meta",
            "kind=email",
        ]);
        assert_eq!(code(&o), 5, "{}", stderr(&o));
        assert!(stderr(&o).contains("metadata.kind"), "{}", stderr(&o));
        let listed = live.json(&["issue", "list", "--source-url", &other]);
        assert!(listed.as_array().unwrap().is_empty(), "{listed}");
    }));

    // Clean up even when a check failed: the CLI cannot delete, so cancel what this run made.
    let o = live.run(&["issue", "update", &id, "--state", "Canceled"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}
