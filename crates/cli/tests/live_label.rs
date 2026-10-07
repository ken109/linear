//! `label create` and `label update` against the real sandbox workspace.
//! Ignored by default; they need the sandbox credentials in the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_label -- --ignored
//! ```
//!
//! The scenario makes one group and two labels under unique names, and one
//! issue in `Fixture Project` that carries two of them (to see the
//! `label-groups-exclusive` rule refuse a move). When it ends, even on a
//! failure, it cancels the issue and deletes the labels (the CLI has no
//! label delete, so this is a raw `issueLabelDelete`, sandbox only).
//! Multi-select groups are not tried: the sandbox workspace does not have them.

mod common;

use common::*;
use serde_json::Value;

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
        // The rule under test, and the raw mutation the cleanup needs.
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\nrules = [\"label-groups-exclusive\"]\nallow_raw_mutation = true\n");
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
}

struct Cleanup<'a> {
    live: &'a Live,
    issues: Vec<String>,
    labels: Vec<String>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for id in &self.issues {
            let _ = self
                .live
                .run(&["issue", "update", id, "--state", "Canceled"]);
        }
        for id in &self.labels {
            let _ = self.live.run(&[
                "api",
                "--mutation",
                "mutation($id: String!) { issueLabelDelete(id: $id) { success } }",
                "--var",
                &format!("id={id}"),
            ]);
        }
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn label_writes_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let (group, a, b) = (
        format!("live group {stamp}"),
        format!("live a {stamp}"),
        format!("live b {stamp}"),
    );
    let renamed = format!("{b} renamed");
    let mut cleanup = Cleanup {
        live: &live,
        issues: Vec::new(),
        labels: Vec::new(),
    };

    // A group, then a label in it. The same request again returns it.
    let g = live.json(&[
        "label",
        "create",
        "--name",
        &group,
        "--is-group",
        "--group-type",
        "single-select",
    ]);
    cleanup.labels.push(g["id"].as_str().unwrap().to_owned());
    assert_eq!(g["existing"], false);
    assert_eq!(g["isGroup"], true);
    assert_eq!(g["groupType"], "singleSelect");
    assert!(g["team"].is_null(), "no --team: a workspace label");

    let made_a = live.json(&["label", "create", "--name", &a, "--group", &group]);
    cleanup
        .labels
        .push(made_a["id"].as_str().unwrap().to_owned());
    assert_eq!(made_a["existing"], false);
    assert_eq!(made_a["parent"]["name"], group.as_str());
    let again = live.json(&["label", "create", "--name", &a, "--group", &group]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], made_a["id"]);

    // A name used elsewhere is refused before Linear is asked.
    let o = live.run(&["label", "create", "--name", &a]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    // A label is not a group.
    let o = live.run(&["label", "create", "--name", "never made", "--group", &a]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("not a group"), "{}", stderr(&o));

    // A plain label with a color; update sends only what differs.
    let made_b = live.json(&["label", "create", "--name", &b, "--color", "#112233"]);
    let b_id = made_b["id"].as_str().unwrap().to_owned();
    cleanup.labels.push(b_id.clone());
    assert_eq!(made_b["color"], "#112233");
    let upd = live.json(&[
        "label",
        "update",
        &b,
        "--color",
        "#445566",
        "--description",
        "made by a live test",
    ]);
    assert_eq!(upd["changed"], true);
    assert_eq!(upd["color"], "#445566");
    assert_eq!(upd["description"], "made by a live test");
    let same = live.json(&["label", "update", &b, "--color", "#445566"]);
    assert_eq!(same["changed"], false);
    let cleared = live.json(&["label", "update", &b, "--description", ""]);
    assert_eq!(cleared["description"], "");
    let o = live.run(&["label", "update", &b, "--new-name", &a]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let ren = live.json(&["label", "update", &b, "--new-name", &renamed]);
    assert_eq!(ren["name"], renamed.as_str());
    assert_eq!(ren["id"], b_id.as_str());

    // An issue carrying both labels (b is in no group, so it is allowed). Moving b
    // into the group would put two labels of it on that issue: refused with exit 5.
    let issue = live.json(&[
        "issue",
        "create",
        "--title",
        &format!("live label check {stamp}"),
        "--project",
        "Fixture Project",
        "--label",
        &a,
        "--label",
        &renamed,
    ]);
    let identifier = issue["identifier"].as_str().unwrap().to_owned();
    cleanup.issues.push(identifier.clone());
    let o = live.run(&["label", "update", &renamed, "--group", &group]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains(&identifier), "{}", stderr(&o));
    let still = live.json(&["label", "view", &renamed]);
    assert!(still[0]["parent"].is_null(), "refused: nothing changed");

    // Without the issue's other label in the way, it moves; and out again.
    let o = live.run(&["issue", "update", &identifier, "--remove-labels", &a]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let moved = live.json(&["label", "update", &renamed, "--group", &group]);
    assert_eq!(moved["changed"], true);
    assert_eq!(moved["parent"]["name"], group.as_str());
    let out = live.json(&["label", "update", &renamed, "--no-group"]);
    assert!(out["parent"].is_null());

    // Read back through the read command.
    let listed = live.json(&["label", "view", &format!("{group}/{a}")]);
    assert_eq!(listed[0]["id"], made_a["id"]);
}
