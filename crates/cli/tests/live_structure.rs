//! Milestone, initiative and template writes against the real sandbox
//! workspace. Ignored by default; they need the sandbox credentials in the
//! environment and the seeded data (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_structure -- --ignored
//! ```
//!
//! The milestone scenario removes what it creates. A template cannot be
//! deleted by the CLI, so the template is created once under a fixed name and
//! later runs find it (that is also what is checked). The sandbox is on the
//! free plan, where Linear refuses to create an initiative; the test checks
//! that the refusal is reported and, should the plan change, that creating
//! one works.

mod common;

use common::*;
use serde_json::Value;

const TEMPLATE_NAME: &str = "Live Created Template";
const TEMPLATE_BODY: &str = "## Summary\n\nWhat this is.\n\n## Steps\n\n1. one\n2. **two**\n";

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
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn milestone_writes_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("live milestone {stamp}");
    let renamed = format!("{name} renamed");
    let project = "Fixture Project";

    let created = live.json(&[
        "milestone",
        "create",
        "--project",
        project,
        "--name",
        &name,
        "--target-date",
        "2026-12-01",
    ]);
    assert_eq!(created["existing"], false);
    assert_eq!(created["targetDate"], "2026-12-01");
    let id = created["id"].as_str().unwrap().to_owned();

    // The same name returns it; nothing new is created.
    let again = live.json(&[
        "milestone",
        "create",
        "--project",
        project,
        "--name",
        &name,
        "--target-date",
        "2027-01-01",
    ]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], id.as_str());
    assert_eq!(
        again["targetDate"], "2026-12-01",
        "the date is not overwritten"
    );

    // A name another milestone has is refused.
    let o = live.run(&[
        "milestone",
        "update",
        &name,
        "--project",
        project,
        "--new-name",
        "Milestone 1",
    ]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let updated = live.json(&[
        "milestone",
        "update",
        &name,
        "--project",
        project,
        "--new-name",
        &renamed,
        "--target-date",
        "2026-12-15",
    ]);
    assert_eq!(updated["changed"], true);
    assert_eq!(updated["name"], renamed.as_str());
    assert_eq!(updated["targetDate"], "2026-12-15");
    let viewed = live.json(&["milestone", "view", &renamed, "--project", project]);
    assert_eq!(viewed["id"], id.as_str());

    // A milestone with issues in it is not deleted.
    let o = live.run(&["milestone", "delete", "Milestone 1", "--project", project]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("still has"), "{}", stderr(&o));

    // A project nobody leads is not written to.
    let o = live.run(&[
        "milestone",
        "create",
        "--project",
        "Finished Project",
        "--name",
        "denied",
        "--target-date",
        "2026-12-01",
    ]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));

    let deleted = live.json(&["milestone", "delete", &renamed, "--project", project]);
    assert_eq!(deleted["deleted"], true);
    let list = live.json(&["milestone", "list", "--project", project]);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"] != id.as_str()),
        "the milestone is gone"
    );
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn template_create_end_to_end() {
    let live = Live::new();
    let body = live.sb.cwd().join("template.md");
    std::fs::write(&body, TEMPLATE_BODY).unwrap();
    let body = body.to_string_lossy().into_owned();

    // The first run creates it; later runs find it. Either way it ends up there with its sections.
    let made = live.json(&[
        "template",
        "create",
        "--name",
        TEMPLATE_NAME,
        "--body-file",
        &body,
    ]);
    assert_eq!(made["sections"], serde_json::json!(["Summary", "Steps"]));
    let again = live.json(&[
        "template",
        "create",
        "--name",
        TEMPLATE_NAME,
        "--body-file",
        &body,
    ]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], made["id"]);

    // What was written is what the read side sees as the template's sections.
    let skeleton = live.run(&["template", "skeleton", TEMPLATE_NAME]);
    assert_eq!(code(&skeleton), 0, "{}", stderr(&skeleton));
    assert_eq!(stdout(&skeleton), "## Summary\n\n## Steps\n");
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiative_create_is_reported_when_the_plan_does_not_allow_it() {
    let live = Live::new();
    let o = live.run(&["initiative", "create", "--name", "Live Created Initiative"]);
    if code(&o) == 0 {
        // The workspace gained initiatives: creating one is idempotent too.
        let v = live.json(&["initiative", "create", "--name", "Live Created Initiative"]);
        assert_eq!(v["existing"], true);
    } else {
        assert!(
            stderr(&o).contains("Initiatives are disabled"),
            "{}",
            stderr(&o)
        );
    }
}
