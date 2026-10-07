//! `--dry-run` against the real sandbox workspace: every write is run as a dry run and then
//! the workspace is read back to see that nothing changed. Ignored by default; it needs the
//! sandbox credentials and the seeded data (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_dry_run -- --ignored
//! ```
//!
//! It makes one issue (with a unique source URL), dry-runs a create, an update, a comment, an
//! attached file, a delete and an archive against it, checks the issue and the workspace are
//! as they were, and cancels the issue at the end.

mod common;

use common::*;
use serde_json::Value;

const BODY: &str = "## Background\n\nLive check.\n\n## Acceptance criteria\n\n- passes\n\n## Out of scope\n\nNothing.\n";

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
            "\nrules = [\"template-sections\", \"source-attachment\", \"label-groups-exclusive\"]\n",
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

    /// A dry run: succeeds, says so, and plans these operations.
    fn plan(&self, args: &[&str], operations: &[&str]) -> Value {
        let mut full = args.to_vec();
        full.push("--dry-run");
        let v = self.json(&full);
        assert_eq!(v["dryRun"], true, "{args:?}: {v}");
        let planned: Vec<&str> = v["mutations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["operation"].as_str().unwrap())
            .collect();
        assert_eq!(planned, operations, "{args:?}");
        v
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_dry_run_changes_nothing_in_the_workspace() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("https://example.com/linear-cli-live/{stamp}/dry");
    let other = format!("https://example.com/linear-cli-live/{stamp}/other");
    let title = format!("live dry run {stamp}");
    let body = live.sb.cwd().join("body.md");
    std::fs::write(&body, BODY).unwrap();
    let body = body.to_string_lossy().into_owned();
    let png = live.sb.cwd().join("shot.png");
    std::fs::write(&png, b"\x89PNG").unwrap();
    let png = png.to_string_lossy().into_owned();
    let text = live.sb.cwd().join("comment.md");
    std::fs::write(&text, "A dry-run comment.\n").unwrap();
    let text = text.to_string_lossy().into_owned();

    // A dry-run create of a new issue makes nothing: no issue carries its source afterwards.
    let create = [
        "issue",
        "create",
        "--title",
        &title,
        "--project",
        "Fixture Project",
        "--template",
        "Sectioned Template",
        "--body-file",
        &body,
        "--source",
        &source,
    ];
    let v = live.plan(&create, &["IssueCreate", "AttachmentCreate"]);
    assert_eq!(v["target"]["new"], true);
    let found = live.json(&["issue", "list", "--source-url", &source]);
    assert_eq!(found.as_array().unwrap().len(), 0, "{found}");

    // The real one, so the other writes have something to be run against.
    let created = live.json(&create);
    let id = created["identifier"].as_str().unwrap().to_owned();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let before = live.json(&["issue", "view", &id]);

        // The same create again, as a dry run: the issue with that source is returned.
        let v = live.plan(&create, &[]);
        assert_eq!(v["target"]["name"], id.as_str());
        assert!(v["reason"].as_str().unwrap().contains("already exists"));

        // An update of the state, labels and a new source.
        let v = live.plan(
            &[
                "issue",
                "update",
                &id,
                "--state",
                "Done",
                "--labels",
                "Bug",
                "--source",
                &other,
                "--priority",
                "urgent",
            ],
            &["IssueUpdate", "AttachmentCreate"],
        );
        let changed: Vec<&str> = v["changed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert_eq!(changed, ["state", "labels", "priority", "source"]);
        // A body without its template is refused exactly as the real run refuses it.
        let o = live.run(&["issue", "update", &id, "--body-file", &body, "--dry-run"]);
        assert_eq!(code(&o), 5, "{}", stderr(&o));

        live.plan(
            &["issue", "comment", &id, "--body-file", &text],
            &["CommentCreate"],
        );
        live.plan(
            &["issue", "attach-file", &id, &png],
            &["FileUpload", "AttachmentCreate"],
        );
        live.plan(&["file", "upload", &png], &["FileUpload"]);
        live.plan(&["issue", "archive", &id], &["IssueArchive"]);
        live.plan(&["issue", "delete", &id], &["IssueDelete"]);
        live.plan(
            &["label", "create", "--name", &format!("live-dry-{stamp}")],
            &["LabelCreate"],
        );
        live.plan(
            &[
                "milestone",
                "create",
                "--project",
                "Fixture Project",
                "--name",
                &title,
                "--target-date",
                "2027-01-01",
            ],
            &["MilestoneCreate"],
        );

        // Read it all back: the issue is as it was, no label or milestone appeared.
        let after = live.json(&["issue", "view", &id]);
        for field in [
            "title",
            "description",
            "priority",
            "updatedAt",
            "archivedAt",
        ] {
            assert_eq!(before[field], after[field], "{field}");
        }
        assert_eq!(before["state"], after["state"]);
        assert_eq!(before["labels"], after["labels"]);
        assert_eq!(before["attachments"], after["attachments"]);
        assert_eq!(before["comments"], after["comments"]);
        let labels = live.json(&["label", "list"]);
        assert!(!labels.to_string().contains(&format!("live-dry-{stamp}")));
        let milestones = live.json(&["milestone", "list", "--project", "Fixture Project"]);
        assert!(!milestones.to_string().contains(&title));
    }));

    // Cancel what this run made, whatever happened.
    let o = live.run(&["issue", "update", &id, "--state", "Canceled"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}
