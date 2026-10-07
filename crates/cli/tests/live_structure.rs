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
//! later runs find it (that is also what is checked). The initiative scenario
//! creates an initiative and two projects under unique names; the CLI cannot
//! delete either, so it cancels the projects and archives the initiative
//! (through a raw mutation) when it ends, even on failure.

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

/// Cancels the projects and archives the initiatives a test made, whether it
/// passed or not (the CLI cannot delete either). Failures here are ignored: a
/// leftover is only a canceled project or an archived initiative.
struct Cleanup<'a> {
    live: &'a Live,
    projects: Vec<String>,
    initiatives: Vec<String>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for slug in &self.projects {
            let _ = self
                .live
                .run(&["project", "update", slug, "--status", "Canceled"]);
        }
        for id in &self.initiatives {
            let _ = self.live.run(&[
                "api",
                "--mutation",
                "mutation($id: String!) { initiativeArchive(id: $id) { success } }",
                "--var",
                &format!("id={id}"),
            ]);
        }
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiative_writes_end_to_end() {
    let live = Live::new();
    // Archiving an initiative is only possible through a raw mutation, which
    // the workspace has to allow; it is for the cleanup only.
    let path = live.sb.config_dir().join("workspaces.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\nallow_raw_mutation = true\n");
    std::fs::write(&path, text).unwrap();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("live initiative {stamp}");
    let mut cleanup = Cleanup {
        live: &live,
        projects: Vec::new(),
        initiatives: Vec::new(),
    };

    // Create: new, with its description.
    let description = live.sb.cwd().join("description.md");
    std::fs::write(&description, "What this initiative is for.\n").unwrap();
    let description = description.to_string_lossy().into_owned();
    let made = live.json(&[
        "initiative",
        "create",
        "--name",
        &name,
        "--description-file",
        &description,
    ]);
    cleanup
        .initiatives
        .push(made["id"].as_str().unwrap().to_owned());
    assert_eq!(made["existing"], false);
    assert_eq!(made["name"], name.as_str());
    assert_eq!(made["description"], "What this initiative is for.");
    let slug = made["slugId"].as_str().unwrap().to_owned();
    assert!(!slug.is_empty());

    // The same name returns it; nothing new is created and nothing is overwritten.
    let again = live.json(&["initiative", "create", "--name", &name]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], made["id"]);
    assert_eq!(again["description"], "What this initiative is for.");

    // Reads on real data: it is listed (also by its status), and viewed by name and by slug.
    let listed = live.json(&["initiative", "list", "--all"]);
    let found: Vec<&Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["name"] == name.as_str())
        .collect();
    assert_eq!(found.len(), 1, "created twice? {listed}");
    assert_eq!(found[0]["slugId"], slug.as_str());
    let status = made["status"].as_str().unwrap().to_lowercase();
    let by_status = live.json(&["initiative", "list", "--all", "--status", &status]);
    assert!(
        by_status
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == name.as_str()),
        "not listed under its status {status}"
    );
    let viewed = live.json(&["initiative", "view", &name]);
    assert_eq!(viewed["id"], made["id"]);
    assert_eq!(viewed["description"], "What this initiative is for.");
    assert_eq!(viewed["projects"]["nodes"], serde_json::json!([]));
    assert_eq!(live.json(&["initiative", "view", &slug])["id"], made["id"]);

    // `project create --initiative`: the project is created and linked.
    let a = live.json(&[
        "project",
        "create",
        "--name",
        &format!("live initiative project A {stamp}"),
        "--initiative",
        &name,
    ]);
    let a_slug = a["slugId"].as_str().unwrap().to_owned();
    cleanup.projects.push(a_slug.clone());
    assert_eq!(a["existing"], false);
    let linked = |view: &Value| -> bool {
        view["initiatives"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == name.as_str())
    };
    let a_view = live.json(&["project", "view", &a_slug]);
    assert!(
        linked(&a_view),
        "project A is not under the initiative: {a_view}"
    );

    // `project update --initiative`: links an existing project; a second run changes nothing.
    let b = live.json(&[
        "project",
        "create",
        "--name",
        &format!("live initiative project B {stamp}"),
    ]);
    let b_slug = b["slugId"].as_str().unwrap().to_owned();
    cleanup.projects.push(b_slug.clone());
    assert!(!linked(&live.json(&["project", "view", &b_slug])));
    let u = live.json(&["project", "update", &b_slug, "--initiative", &name]);
    assert_eq!(u["changed"], serde_json::json!(["initiative"]));
    let same = live.json(&["project", "update", &b_slug, "--initiative", &slug]);
    assert_eq!(same["changed"], serde_json::json!([]));
    assert!(linked(&live.json(&["project", "view", &b_slug])));

    // Both projects are under the initiative, seen from either side.
    let viewed = live.json(&["initiative", "view", &name]);
    let slugs: Vec<&str> = viewed["projects"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slugId"].as_str().unwrap())
        .collect();
    assert_eq!(slugs.len(), 2, "{viewed}");
    assert!(slugs.contains(&a_slug.as_str()) && slugs.contains(&b_slug.as_str()));
    let projects = live.json(&["project", "list", "--all", "--initiative", &name]);
    assert_eq!(projects.as_array().unwrap().len(), 2, "{projects}");

    // An initiative that does not exist is refused before anything is created.
    let ghost = format!("live project ghost {stamp}");
    let o = live.run(&[
        "project",
        "create",
        "--name",
        &ghost,
        "--initiative",
        "no such live initiative",
    ]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let all = live.json(&["project", "list", "--all"]);
    assert!(
        all.as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != ghost.as_str()),
        "a refused create made a project"
    );
    // The rollback of a failed link (the new project is deleted, the other
    // fields of an update are put back) cannot be provoked on a real workspace
    // without breaking it; the mock tests in `project_write.rs` cover it.
}
