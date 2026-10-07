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
//! later runs find it (that is also what is checked). The initiative,
//! project and issue scenarios create their things under unique names and
//! exercise the reversible deletes (`delete`, `archive`, `unarchive`) and
//! `issue unlink`; when they end, even on failure, the cleanup trashes the
//! projects and issues and archives the initiatives with those same commands.

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

/// Trashes the projects and issues and archives the initiatives a test made,
/// whether it passed or not. Failures here are ignored: a leftover is only a
/// trashed project or issue, or an archived initiative (and one the test
/// already removed cannot be removed again).
struct Cleanup<'a> {
    live: &'a Live,
    projects: Vec<String>,
    initiatives: Vec<String>,
    issues: Vec<String>,
}

impl<'a> Cleanup<'a> {
    fn new(live: &'a Live) -> Self {
        Cleanup {
            live,
            projects: Vec::new(),
            initiatives: Vec::new(),
            issues: Vec::new(),
        }
    }
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for issue in &self.issues {
            let _ = self.live.run(&["issue", "delete", issue]);
        }
        for slug in &self.projects {
            let _ = self.live.run(&["project", "delete", slug]);
        }
        for id in &self.initiatives {
            let _ = self.live.run(&["initiative", "archive", id]);
        }
    }
}

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// Whether `list` (a JSON array of entities) holds one whose `key` is `value`.
fn lists(list: &Value, key: &str, value: &str) -> bool {
    list.as_array().unwrap().iter().any(|i| i[key] == value)
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiative_writes_end_to_end() {
    let live = Live::new();
    let stamp = stamp();
    let name = format!("live initiative {stamp}");
    let mut cleanup = Cleanup::new(&live);

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

    // Archive -> unarchive -> archive. An archived initiative leaves the
    // listing and is found again by `unarchive`, which restores it with its projects.
    let archived = live.json(&["initiative", "archive", &name]);
    assert_eq!(archived["action"], "archived");
    assert_eq!(archived["id"], made["id"]);
    let listed = live.json(&["initiative", "list", "--all"]);
    assert!(!lists(&listed, "name", &name), "still listed: {listed}");
    // Already archived: it is not among the live ones any more.
    let o = live.run(&["initiative", "archive", &name]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let restored = live.json(&["initiative", "unarchive", &name]);
    assert_eq!(restored["action"], "unarchived");
    assert_eq!(restored["id"], made["id"]);
    let listed = live.json(&["initiative", "list", "--all"]);
    assert!(lists(&listed, "name", &name), "not back: {listed}");
    assert_eq!(live.json(&["initiative", "view", &slug])["id"], made["id"]);

    let archived = live.json(&["initiative", "archive", &slug]);
    assert_eq!(archived["action"], "archived");
    let listed = live.json(&["initiative", "list", "--all"]);
    assert!(!lists(&listed, "name", &name), "still listed: {listed}");
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiative_delete_end_to_end() {
    let live = Live::new();
    let mut cleanup = Cleanup::new(&live);
    let name = format!("live initiative delete {}", stamp());
    let made = live.json(&["initiative", "create", "--name", &name]);
    cleanup
        .initiatives
        .push(made["id"].as_str().unwrap().to_owned());

    // Deleted (trashed): gone from the listing.
    let deleted = live.json(&["initiative", "delete", &name]);
    assert_eq!(deleted["action"], "deleted");
    assert_eq!(deleted["id"], made["id"]);
    let listed = live.json(&["initiative", "list", "--all"]);
    assert!(!lists(&listed, "name", &name), "still listed: {listed}");
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn project_delete_and_unarchive_end_to_end() {
    let live = Live::new();
    let mut cleanup = Cleanup::new(&live);
    let name = format!("live project delete {}", stamp());
    let made = live.json(&["project", "create", "--name", &name]);
    let slug = made["slugId"].as_str().unwrap().to_owned();
    cleanup.projects.push(slug.clone());

    // Delete (trash): it leaves the listing, and a second delete cannot find it.
    let deleted = live.json(&["project", "delete", &slug]);
    assert_eq!(deleted["action"], "deleted");
    assert_eq!(deleted["id"], made["id"]);
    let listed = live.json(&["project", "list", "--all"]);
    assert!(!lists(&listed, "name", &name), "still listed: {listed}");
    let o = live.run(&["project", "delete", &name]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    // Unarchive finds it among the deleted ones (by name too) and restores it.
    let restored = live.json(&["project", "unarchive", &name]);
    assert_eq!(restored["action"], "unarchived");
    assert_eq!(restored["id"], made["id"]);
    let listed = live.json(&["project", "list", "--all"]);
    assert!(lists(&listed, "name", &name), "not back: {listed}");

    // And again by slug id, to be sure the cycle repeats.
    assert_eq!(
        live.json(&["project", "delete", &slug])["action"],
        "deleted"
    );
    assert_eq!(
        live.json(&["project", "unarchive", &slug])["action"],
        "unarchived"
    );
    assert!(lists(
        &live.json(&["project", "list", "--all"]),
        "name",
        &name
    ));
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn issue_archive_delete_unarchive_and_unlink_end_to_end() {
    let live = Live::new();
    let mut cleanup = Cleanup::new(&live);
    let stamp = stamp();
    let source = format!("https://example.com/linear-cli-live/{stamp}/unlink");
    let other = format!("https://example.com/linear-cli-live/{stamp}/other");

    let made = live.json(&[
        "issue",
        "create",
        "--title",
        &format!("live archive {stamp}"),
        "--project",
        "Fixture Project",
        "--source",
        &source,
    ]);
    let id = made["identifier"].as_str().unwrap().to_owned();
    cleanup.issues.push(id.clone());
    let in_project =
        |live: &Live| live.json(&["issue", "list", "--project", "Fixture Project", "--all"]);

    // unlink: without --yes it says what it would delete and sends nothing (exit 2).
    let o = live.run(&["issue", "unlink", &id, &source]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains(&source), "{}", stderr(&o));
    assert_eq!(
        live.json(&["issue", "view", &id])["sourceUrl"],
        source.as_str()
    );

    // A URL the issue does not have is left alone (and succeeds).
    let none = live.json(&["issue", "unlink", &id, &other, "--yes"]);
    assert_eq!(none["notLinked"], true);

    // With --yes the attachment goes; asking again finds nothing.
    let gone = live.json(&["issue", "unlink", &id, &source, "--yes"]);
    assert_eq!(gone["notLinked"], false);
    assert!(gone["attachmentId"].as_str().is_some_and(|a| !a.is_empty()));
    let viewed = live.json(&["issue", "view", &id]);
    assert!(viewed["sourceUrl"].is_null(), "still attached: {viewed}");
    let again = live.json(&["issue", "unlink", &id, &source, "--yes"]);
    assert_eq!(again["notLinked"], true);

    // archive -> unarchive: it leaves the listing and comes back.
    assert!(lists(&in_project(&live), "identifier", &id));
    let archived = live.json(&["issue", "archive", &id]);
    assert_eq!(archived["action"], "archived");
    assert_eq!(archived["identifier"], id.as_str());
    assert!(
        !lists(&in_project(&live), "identifier", &id),
        "still listed"
    );
    let restored = live.json(&["issue", "unarchive", &id]);
    assert_eq!(restored["action"], "unarchived");
    assert!(lists(&in_project(&live), "identifier", &id), "not back");

    // delete (trash) -> unarchive: the same.
    let deleted = live.json(&["issue", "delete", &id]);
    assert_eq!(deleted["action"], "deleted");
    assert!(
        !lists(&in_project(&live), "identifier", &id),
        "still listed"
    );
    let restored = live.json(&["issue", "unarchive", &id]);
    assert_eq!(restored["action"], "unarchived");
    assert!(lists(&in_project(&live), "identifier", &id), "not back");
}
