//! `initiative update|add-project|remove-project|status-update|status-updates`
//! against the real sandbox workspace. Ignored by default; they need the
//! sandbox credentials in the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_initiative_write -- --ignored
//! ```
//!
//! The initiative and projects are created under unique names and, when the
//! test ends (even on failure), the projects are trashed and the initiative is
//! archived with the CLI's own commands. A status update cannot be removed by
//! the CLI; it goes away with the archived initiative's data and is only seen
//! on that initiative.

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

struct Cleanup<'a> {
    live: &'a Live,
    projects: Vec<String>,
    initiatives: Vec<String>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
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

fn project_slugs(view: &Value) -> Vec<String> {
    view["projects"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slugId"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiative_update_projects_and_status_updates_end_to_end() {
    let live = Live::new();
    let stamp = stamp();
    let name = format!("live initiative update {stamp}");
    let renamed = format!("{name} renamed");
    let mut cleanup = Cleanup {
        live: &live,
        projects: Vec::new(),
        initiatives: Vec::new(),
    };

    let made = live.json(&["initiative", "create", "--name", &name]);
    cleanup
        .initiatives
        .push(made["id"].as_str().unwrap().to_owned());
    let slug = made["slugId"].as_str().unwrap().to_owned();

    // update: only what differs is written; a second identical run changes nothing.
    let description = live.sb.cwd().join("description.md");
    std::fs::write(&description, "Changed by a live test.\n").unwrap();
    let description = description.to_string_lossy().into_owned();
    let updated = live.json(&[
        "initiative",
        "update",
        &name,
        "--name",
        &renamed,
        "--description-file",
        &description,
        "--status",
        "planned",
        "--target-date",
        "2030-01-31",
        "--owner",
        "me",
    ]);
    assert_eq!(
        updated["changed"],
        json!(["name", "description", "status", "targetDate", "owner"])
    );
    assert_eq!(updated["name"], renamed.as_str());
    assert_eq!(updated["description"], "Changed by a live test.");
    assert_eq!(updated["status"], "Planned");
    assert_eq!(updated["targetDate"], "2030-01-31");
    assert!(updated["owner"]["isMe"].as_bool().unwrap());
    let same = live.json(&[
        "initiative",
        "update",
        &slug,
        "--name",
        &renamed,
        "--status",
        "planned",
        "--target-date",
        "2030-01-31",
    ]);
    assert_eq!(same["changed"], json!([]));
    let viewed = live.json(&["initiative", "view", &slug]);
    assert_eq!(viewed["name"], renamed.as_str());
    assert_eq!(viewed["status"], "Planned");
    // The status filter of `list` sees the new status.
    let planned = live.json(&["initiative", "list", "--all", "--status", "planned"]);
    assert!(planned
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["slugId"] == slug.as_str()));
    let o = live.run(&["initiative", "update", &slug]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    // add-project / remove-project: idempotent both ways.
    let project = live.json(&[
        "project",
        "create",
        "--name",
        &format!("live initiative link {stamp}"),
    ]);
    let p_slug = project["slugId"].as_str().unwrap().to_owned();
    cleanup.projects.push(p_slug.clone());
    assert!(project_slugs(&live.json(&["initiative", "view", &slug])).is_empty());

    let added = live.json(&["initiative", "add-project", &slug, &p_slug]);
    assert_eq!(
        (added["action"].as_str(), added["unchanged"].as_bool()),
        (Some("added"), Some(false))
    );
    assert_eq!(
        project_slugs(&live.json(&["initiative", "view", &slug])),
        vec![p_slug.clone()]
    );
    let again = live.json(&["initiative", "add-project", &renamed, &p_slug]);
    assert_eq!(again["unchanged"], true);
    assert_eq!(
        project_slugs(&live.json(&["initiative", "view", &slug])).len(),
        1,
        "linked twice"
    );

    let removed = live.json(&["initiative", "remove-project", &slug, &p_slug]);
    assert_eq!(
        (removed["action"].as_str(), removed["unchanged"].as_bool()),
        (Some("removed"), Some(false))
    );
    assert!(project_slugs(&live.json(&["initiative", "view", &slug])).is_empty());
    // The project itself is still there.
    assert_eq!(
        live.json(&["project", "view", &p_slug])["slugId"],
        p_slug.as_str()
    );
    let again = live.json(&["initiative", "remove-project", &slug, &p_slug]);
    assert_eq!(again["unchanged"], true);

    // status-update then status-updates, newest first.
    assert!(
        live.json(&["initiative", "status-updates", &slug])["updates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let body = live.sb.cwd().join("update.md");
    std::fs::write(&body, "First update.\n\n- on track\n").unwrap();
    let body = body.to_string_lossy().into_owned();
    let first = live.json(&[
        "initiative",
        "status-update",
        &slug,
        "--health",
        "onTrack",
        "--body-file",
        &body,
    ]);
    assert_eq!(first["health"], "onTrack");
    assert_eq!(first["body"], "First update.\n\n- on track");
    assert_eq!(first["initiative"]["name"], renamed.as_str());
    std::fs::write(&body, "Second update.\n").unwrap();
    let second = live.json(&[
        "initiative",
        "status-update",
        &slug,
        "--health",
        "atRisk",
        "--body-file",
        &body,
    ]);
    assert_eq!(second["health"], "atRisk");
    let listed = live.json(&["initiative", "status-updates", &slug]);
    let updates = listed["updates"].as_array().unwrap();
    assert_eq!(updates.len(), 2, "{listed}");
    assert_eq!(updates[0]["id"], second["id"]);
    assert_eq!(updates[1]["id"], first["id"]);
    let text = live.run(&["initiative", "status-updates", &slug]);
    assert!(stdout(&text).contains("atRisk") && stdout(&text).contains("Second update."));
}
