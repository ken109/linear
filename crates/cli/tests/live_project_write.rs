//! Project write commands against the real sandbox workspace. Ignored by
//! default; they need the sandbox credentials in the environment and the
//! seeded data (see `tests/fixtures/README.md`; this uses `Finished Project`,
//! which nobody leads, and the `A project template` project template):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_project_write -- --ignored
//! ```
//!
//! The scenario is one test so its steps run in order. The CLI has no way to
//! delete a project, so the projects it makes are canceled at the end:
//! repeated runs leave only canceled projects behind.
//!
//! `--initiative` is checked live in `live_structure.rs`; the rollback of a
//! failed link is only covered by the mock tests in `project_write.rs`.

mod common;

use common::*;
use serde_json::Value;

struct Live {
    sb: Sandbox,
    key: String,
}

impl Live {
    fn new(rules: &[&str]) -> Live {
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
        if !rules.is_empty() {
            let path = sb.config_dir().join("workspaces.toml");
            let mut text = std::fs::read_to_string(&path).unwrap();
            let list: Vec<String> = rules.iter().map(|r| format!("\"{r}\"")).collect();
            text.push_str(&format!("\nrules = [{}]\n", list.join(", ")));
            std::fs::write(&path, text).unwrap();
        }
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

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.sb.cwd().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// `(sortOrder, prioritySortOrder)` of a project, read through `linear api`.
    fn order_of(&self, slug: &str) -> (f64, f64) {
        let v = self.json(&[
            "api",
            "query($id: String!) { project(id: $id) { sortOrder prioritySortOrder } }",
            "--var",
            &format!("id={slug}"),
        ]);
        let p = &v["project"];
        (
            p["sortOrder"].as_f64().unwrap(),
            p["prioritySortOrder"].as_f64().unwrap(),
        )
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn project_writes_end_to_end() {
    let live = Live::new(&[]);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let body = live.file(
        "body.md",
        "## Goal\n\nLive check.\n\n## Done when\n\n- passes\n",
    );
    let mut made: Vec<String> = Vec::new();

    // Create: the project exists, I lead it, and it carries what was given.
    let name_a = format!("live project A {stamp}");
    let a = live.json(&[
        "project",
        "create",
        "--name",
        &name_a,
        "--summary",
        "A live check",
        "--body-file",
        &body,
        "--target-date",
        "2026-12-31",
    ]);
    assert_eq!(a["existing"], false);
    let a_slug = a["slugId"].as_str().unwrap().to_owned();
    made.push(a_slug.clone());
    let viewed = live.json(&["project", "view", &a_slug, "--content"]);
    assert_eq!(viewed["name"], name_a.as_str());
    assert_eq!(viewed["description"], "A live check");
    assert_eq!(viewed["targetDate"], "2026-12-31");
    assert_eq!(viewed["lead"]["isMe"], true);
    assert!(viewed["content"].as_str().unwrap().contains("Live check."));

    // Idempotence: the same unfinished name returns the same project and creates nothing.
    let again = live.json(&["project", "create", "--name", &name_a]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], a["id"]);

    // Update: only what was asked changes, and a second run changes nothing.
    let u = live.json(&[
        "project",
        "update",
        &name_a,
        "--status",
        "in progress",
        "--summary",
        "Changed",
        "--target-date",
        "2027-01-31",
    ]);
    assert_eq!(
        u["changed"],
        serde_json::json!(["summary", "status", "targetDate"])
    );
    assert_eq!(u["status"]["name"], "In Progress");
    assert_eq!(u["targetDate"], "2027-01-31");
    let viewed = live.json(&["project", "view", &a_slug]);
    assert_eq!(viewed["description"], "Changed");
    assert_eq!(
        viewed["name"],
        name_a.as_str(),
        "the name was not asked for, so it was not touched"
    );
    let same = live.json(&["project", "update", &a_slug, "--status", "In Progress"]);
    assert_eq!(same["changed"], serde_json::json!([]));

    // Rename, and a name another unfinished project has is refused.
    let name_b = format!("live project B {stamp}");
    let b = live.json(&["project", "create", "--name", &name_b]);
    let b_slug = b["slugId"].as_str().unwrap().to_owned();
    made.push(b_slug.clone());
    let o = live.run(&["project", "update", &b_slug, "--name", &name_a]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let renamed = format!("live project B renamed {stamp}");
    let r = live.json(&["project", "update", &b_slug, "--name", &renamed]);
    assert_eq!(r["name"], renamed.as_str());

    // Status update.
    let update = live.file("update.md", "- Stage: live check\n- Next: cancel it\n");
    let s = live.json(&[
        "project",
        "status-update",
        &a_slug,
        "--health",
        "atRisk",
        "--body-file",
        &update,
    ]);
    assert_eq!(s["workspace"], "sandbox");
    assert_eq!(s["health"], "atRisk");
    let viewed = live.json(&["project", "view", &a_slug]);
    assert_eq!(viewed["lastUpdate"]["health"], "atRisk");
    assert!(viewed["lastUpdate"]["body"]
        .as_str()
        .unwrap()
        .contains("live check"));

    // Reorder: put B above A, then A above B; both ordering values follow.
    for wanted in [[&b_slug, &a_slug], [&a_slug, &b_slug]] {
        let r = live.json(&["project", "reorder", wanted[0], wanted[1]]);
        assert_eq!(r["workspace"], "sandbox");
        let (top, bottom) = (live.order_of(wanted[0]), live.order_of(wanted[1]));
        assert!(
            top.0 < bottom.0,
            "sortOrder: {top:?} should be above {bottom:?}"
        );
        assert!(
            top.1 < bottom.1,
            "prioritySortOrder: {top:?} should be above {bottom:?}"
        );
    }
    let r = live.json(&["project", "reorder", &a_slug, &b_slug]);
    assert_eq!(r["unchanged"], true);

    // The guard: a project nobody leads is not mine (exit 4), for every write.
    for args in [
        vec![
            "project",
            "update",
            "Finished Project",
            "--summary",
            "never",
        ],
        vec![
            "project",
            "status-update",
            "Finished Project",
            "--health",
            "onTrack",
            "--body-file",
            update.as_str(),
        ],
        vec!["project", "reorder", "Finished Project", a_slug.as_str()],
    ] {
        let o = live.run(&args);
        assert_eq!(code(&o), 4, "{args:?}: {}", stderr(&o));
    }
    let o = live.run(&["project", "update", &a_slug, "--lead", "none"]);
    assert_eq!(
        code(&o),
        2,
        "an unknown user is a usage error: {}",
        stderr(&o)
    );

    // Clean up: the CLI cannot delete, so cancel what this run made.
    for slug in &made {
        let o = live.run(&["project", "update", slug, "--status", "Canceled"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_project_template_can_be_required_live() {
    // `A project template` has no readable sections, so the rule refuses to
    // judge a body against it: a validator refusal (exit 5), nothing created.
    let live = Live::new(&["template-sections"]);
    let body = live.file("body.md", "## Goal\n\nLive check.\n");
    let name = format!("live project never {}", std::process::id());

    let o = live.run(&["project", "create", "--name", &name]);
    assert_eq!(code(&o), 5, "no template named: {}", stderr(&o));
    assert!(
        stderr(&o).contains("a template is required"),
        "{}",
        stderr(&o)
    );

    let o = live.run(&[
        "project",
        "create",
        "--name",
        &name,
        "--template",
        "A project template",
        "--body-file",
        &body,
    ]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));

    let listed = live.json(&["project", "list", "--all"]);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != name.as_str()),
        "a refused create made a project"
    );
}
