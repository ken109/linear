//! Write commands against the real sandbox workspace. Ignored by default; they
//! need the sandbox credentials in the environment and the seeded data (see
//! `tests/fixtures/README.md`; this also uses the `Sectioned Template`,
//! `Fixture Project` and `Finished Project`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_write -- --ignored
//! ```
//!
//! The scenario is one test so its steps run in order. It creates issues with
//! a unique source URL and cancels them at the end (the CLI has no delete),
//! so repeated runs leave only canceled issues behind.

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

    fn create(&self, title: &str, source: &str, body_file: &str) -> std::process::Output {
        self.run(&[
            "issue",
            "create",
            "--title",
            title,
            "--project",
            "Fixture Project",
            "--template",
            "Sectioned Template",
            "--body-file",
            body_file,
            "--source",
            source,
            "--json",
        ])
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn issue_writes_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = live.sb.cwd().join("body.md");
    std::fs::write(&body, BODY).unwrap();
    let body = body.to_string_lossy().into_owned();
    let mut made: Vec<String> = Vec::new();

    // Create: the issue exists and carries its source.
    let source_a = format!("https://example.com/linear-cli-live/{stamp}/a");
    let o = live.create("live write A", &source_a, &body);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let a: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(a["existing"], false);
    let a_id = a["identifier"].as_str().unwrap().to_owned();
    made.push(a_id.clone());
    let viewed = live.json(&["issue", "view", &a_id]);
    assert_eq!(viewed["sourceUrl"], source_a);
    assert!(viewed["description"]
        .as_str()
        .unwrap()
        .contains("Live check."));

    // Idempotence: the same source returns the same issue and creates nothing.
    let o = live.create("live write A, again", &source_a, &body);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let again: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(again["existing"], true);
    assert_eq!(again["identifier"], a_id.as_str());

    // Rollback: a source Linear refuses leaves no issue behind.
    let bad_source = format!("https://example.com/{}", "a".repeat(3000));
    let o = live.create(&format!("live write rollback {stamp}"), &bad_source, &body);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    let deleted = err
        .split("rolled back: deleted ")
        .nth(1)
        .unwrap_or_else(|| panic!("no rollback reported: {err}"))
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let titles = live.json(&["issue", "list", "--project", "Fixture Project", "--all"]);
    assert!(
        titles
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["identifier"] != deleted.as_str()),
        "{deleted} is still listed after the rollback"
    );

    // The guard: a project nobody leads is not mine (exit 4); a thin body is refused (exit 5).
    let o = live.run(&[
        "issue",
        "create",
        "--title",
        "never",
        "--project",
        "Finished Project",
        "--template",
        "Sectioned Template",
        "--body-file",
        &body,
        "--source",
        "https://example.com/never",
    ]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    let thin = live.sb.cwd().join("thin.md");
    std::fs::write(&thin, "## Background\n\nOnly this.\n").unwrap();
    let o = live.create(
        "never",
        "https://example.com/never",
        &thin.to_string_lossy(),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));

    // Update and comment.
    let updated = live.json(&[
        "issue",
        "update",
        &a_id,
        "--state",
        "Todo",
        "--due",
        "2026-12-31",
    ]);
    assert_eq!(updated["state"]["name"], "Todo");
    assert_eq!(updated["dueDate"], "2026-12-31");
    let comment = live.sb.cwd().join("comment.md");
    std::fs::write(&comment, "Live write check.\n").unwrap();
    let c = live.json(&[
        "issue",
        "comment",
        &a_id,
        "--body-file",
        &comment.to_string_lossy(),
    ]);
    assert_eq!(c["issue"], a_id.as_str());
    assert!(!c["url"].as_str().unwrap().is_empty());

    // Reorder: a second and third issue, then ask for A on top of them.
    for (n, title) in ["B", "C"].iter().enumerate() {
        let source = format!("https://example.com/linear-cli-live/{stamp}/{n}");
        let o = live.create(&format!("live write {title}"), &source, &body);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
        made.push(v["identifier"].as_str().unwrap().to_owned());
    }
    let (b_id, c_id) = (made[1].clone(), made[2].clone());
    for wanted in [[&a_id, &b_id, &c_id], [&c_id, &a_id, &b_id]] {
        let r = live.json(&["issue", "reorder", wanted[0], wanted[1], wanted[2]]);
        assert_eq!(r["workspace"], "sandbox");
        let listed = live.json(&["issue", "list", "--project", "Fixture Project", "--all"]);
        let order = |field: &str| -> Vec<String> {
            let mut rows: Vec<(f64, String)> = listed
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| wanted.iter().any(|w| i["identifier"] == w.as_str()))
                .map(|i| {
                    (
                        i[field].as_f64().unwrap(),
                        i["identifier"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            rows.sort_by(|x, y| x.0.total_cmp(&y.0));
            rows.into_iter().map(|r| r.1).collect()
        };
        let want: Vec<String> = wanted.iter().map(|s| s.to_string()).collect();
        assert_eq!(order("sortOrder"), want, "sortOrder");
        assert_eq!(order("prioritySortOrder"), want, "prioritySortOrder");
    }

    // Clean up: the CLI cannot delete, so cancel what this run made.
    for id in &made {
        let o = live.run(&["issue", "update", id, "--state", "Canceled"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
}
