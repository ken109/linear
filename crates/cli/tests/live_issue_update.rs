//! `linear issue update --body-file|--source|--meta|--labels` against the real
//! sandbox workspace. Ignored by default; it needs the sandbox credentials in
//! the environment and the seeded data (see `tests/fixtures/README.md`: the
//! `Sectioned Template`, `Fixture Project` and the labels `api` and `Bug`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_issue_update -- --ignored
//! ```
//!
//! It creates one issue with unique source URLs, updates it, reads every change
//! back with `issue view --json`, runs each update a second time to see that
//! nothing is sent, and cancels the issue at the end (the CLI has no delete), so
//! repeated runs leave only canceled issues behind.

mod common;

use common::*;
use linear_core::markdown::same_description;
use serde_json::{json, Value};

const BODY: &str = "## Background\n\nLive check.\n\n## Acceptance criteria\n\n- passes\n\n## Out of scope\n\nNothing.\n";
const NEW_BODY: &str = "## Background\n\nRewritten by `issue update`.\n\n## Acceptance criteria\n\n- still passes\n- and a second line\n\n## Out of scope\n\nNothing.\n";

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
            "\nrules = [\"template-sections\", \"source-attachment\", \"label-groups-exclusive\"]\n\
             source_kinds = [\"life-decision\", \"slack\"]\n",
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

    fn attachment(&self, id: &str, url: &str) -> Option<Value> {
        self.view(id)["attachments"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["url"] == url)
            .cloned()
    }

    fn label_names(&self, id: &str) -> Vec<String> {
        let mut names: Vec<String> = self.view(id)["labels"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn issue_update_body_source_meta_and_labels_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let first = format!("https://example.com/linear-cli-live/{stamp}/first");
    let second = format!("https://example.com/linear-cli-live/{stamp}/second");

    let body = live.sb.cwd().join("body.md");
    std::fs::write(&body, BODY).unwrap();
    let new_body = live.sb.cwd().join("new-body.md");
    std::fs::write(&new_body, NEW_BODY).unwrap();
    let (body, new_body) = (
        body.to_string_lossy().into_owned(),
        new_body.to_string_lossy().into_owned(),
    );

    let created = live.json(&[
        "issue",
        "create",
        "--title",
        "live issue update",
        "--project",
        "Fixture Project",
        "--template",
        "Sectioned Template",
        "--body-file",
        &body,
        "--source",
        &first,
        "--meta",
        "kind=slack",
    ]);
    let id = created["identifier"].as_str().unwrap().to_owned();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The body is replaced, and a second identical run sends nothing.
        let args = ["--body-file", &new_body, "--template", "Sectioned Template"];
        assert_eq!(live.update(&id, &args), ["description"]);
        // Linear stores markdown in its own form (`- item` comes back as `* item`).
        let viewed = live.view(&id);
        assert!(
            same_description(viewed["description"].as_str(), NEW_BODY),
            "{}",
            viewed["description"]
        );
        assert_eq!(live.update(&id, &args), Vec::<String>::new());

        // A body that does not fill the template is refused (exit 5) and changes nothing.
        let thin = live.sb.cwd().join("thin.md");
        std::fs::write(&thin, "## Background\n\nOnly this.\n").unwrap();
        let o = live.run(&[
            "issue",
            "update",
            &id,
            "--body-file",
            thin.to_str().unwrap(),
            "--template",
            "Sectioned Template",
        ]);
        assert_eq!(code(&o), 5, "{}", stderr(&o));
        assert!(same_description(
            live.view(&id)["description"].as_str(),
            NEW_BODY
        ));

        // A new source with a title and metadata; read back as asked.
        let args = [
            "--source",
            &second,
            "--source-title",
            "Second source",
            "--meta",
            "kind=slack",
            "--meta",
            "ticket=42",
            "--meta",
            "build=str:123",
        ];
        assert_eq!(live.update(&id, &args), ["source"]);
        let a = live.attachment(&id, &second).expect("the second source");
        assert_eq!(a["title"], "Second source");
        assert_eq!(
            a["metadata"],
            json!({ "kind": "slack", "ticket": 42, "build": "123" })
        );
        assert_eq!(live.update(&id, &args), Vec::<String>::new());

        // Other metadata for the same URL replaces the stored object and keeps the title;
        // there is still exactly one attachment with that URL.
        let args = ["--source", &second, "--meta", "kind=life-decision"];
        assert_eq!(live.update(&id, &args), ["source"]);
        let a = live.attachment(&id, &second).unwrap();
        assert_eq!(a["title"], "Second source");
        assert_eq!(a["metadata"], json!({ "kind": "life-decision" }));
        assert_eq!(live.update(&id, &args), Vec::<String>::new());
        let urls: Vec<Value> = live.view(&id)["attachments"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["url"] == second.as_str())
            .cloned()
            .collect();
        assert_eq!(urls.len(), 1);
        // The first source is untouched.
        assert_eq!(live.attachment(&id, &first).unwrap()["title"], "Source");

        // A kind outside `source_kinds` is refused (exit 5).
        let o = live.run(&[
            "issue",
            "update",
            &id,
            "--source",
            &second,
            "--meta",
            "kind=email",
        ]);
        assert_eq!(code(&o), 5, "{}", stderr(&o));

        // Labels: replace, then edit; each repeated run is a no-op.
        assert_eq!(live.update(&id, &["--labels", "Bug"]), ["labels"]);
        assert_eq!(live.label_names(&id), ["Bug"]);
        assert_eq!(live.update(&id, &["--labels", "Bug"]), Vec::<String>::new());
        assert_eq!(live.update(&id, &["--add-labels", "api"]), ["labels"]);
        assert_eq!(live.label_names(&id), ["Bug", "api"]);
        assert_eq!(
            live.update(&id, &["--add-labels", "api", "--remove-labels", "Bug"]),
            ["labels"]
        );
        assert_eq!(live.label_names(&id), ["api"]);
        assert_eq!(
            live.update(&id, &["--add-labels", "api", "--remove-labels", "Bug"]),
            Vec::<String>::new()
        );

        // Everything at once, as it already is: nothing is sent.
        let all = [
            "--body-file",
            &new_body,
            "--template",
            "Sectioned Template",
            "--source",
            &second,
            "--meta",
            "kind=life-decision",
            "--labels",
            "api",
        ];
        assert_eq!(live.update(&id, &all), Vec::<String>::new());
    }));

    // Clean up even when a check failed: cancel what this run made.
    let o = live.run(&["issue", "update", &id, "--state", "Canceled"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}
