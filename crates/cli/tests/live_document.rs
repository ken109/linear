//! `document` commands against the real sandbox workspace. Ignored by default;
//! they need the sandbox credentials in the environment and the seeded data
//! (see `tests/fixtures/README.md`; this uses `Fixture Project`, which the key's
//! owner leads, and `Finished Project`, which has no lead):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_document -- --ignored
//! ```
//!
//! The scenario makes a document template (a raw mutation: the CLI creates
//! issue and project templates only), documents under `Fixture Project` and under
//! an initiative of its own, and removes all of it when it ends, even on a
//! failure (raw `documentDelete`, `templateDelete` and `initiativeArchive`,
//! sandbox only; the workspace has to allow raw mutations for that).

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
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\nrules = [\"template-sections\"]\nallow_raw_mutation = true\n");
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

    /// A raw mutation (the workspace allows them for this test).
    fn raw(&self, query: &str, vars: &[&str]) -> Value {
        let mut args = vec!["api", "--mutation", query];
        for v in vars {
            args.push("--var");
            args.push(v);
        }
        args.push("--json");
        let o = self.run(&args);
        assert_eq!(code(&o), 0, "{query}: {}", stderr(&o));
        serde_json::from_str(&stdout(&o)).unwrap()
    }
}

struct Cleanup<'a> {
    live: &'a Live,
    documents: Vec<String>,
    templates: Vec<String>,
    initiatives: Vec<String>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let run = |query: &str, id: &str| {
            let _ = self
                .live
                .run(&["api", "--mutation", query, "--var", &format!("id={id}")]);
        };
        for id in &self.documents {
            run(
                "mutation($id: String!) { documentDelete(id: $id) { success } }",
                id,
            );
        }
        for id in &self.templates {
            run(
                "mutation($id: String!) { templateDelete(id: $id) { success } }",
                id,
            );
        }
        for id in &self.initiatives {
            run(
                "mutation($id: String!) { initiativeArchive(id: $id) { success } }",
                id,
            );
        }
    }
}

const FULL: &str = "## Goal\n\nWrite it down.\n\n## Notes\n\nA second section.\n";
const PARTIAL: &str = "## Goal\n\nWrite it down.\n";

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn document_writes_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut cleanup = Cleanup {
        live: &live,
        documents: Vec::new(),
        templates: Vec::new(),
        initiatives: Vec::new(),
    };
    let write = |name: &str, text: &str| {
        let path = live.sb.cwd().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    };
    let (full, partial) = (write("full.md", FULL), write("partial.md", PARTIAL));
    let project = "Fixture Project";

    // A document template, so the rule has something to hold a body to.
    let template = format!("live doc template {stamp}");
    let made = live.raw(
        "mutation($name: String!, $data: JSON!) { templateCreate(input: {type: \"document\", name: $name, templateData: $data}) { success template { id } } }",
        &[
            &format!("name={template}"),
            // The JSON scalar travels as a string of JSON.
            &format!(
                "data={}",
                serde_json::json!({"descriptionData": {"type": "doc", "content": [
                    {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Goal"}]},
                    {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Notes"}]},
                ]}})
            ),
        ],
    );
    cleanup.templates.push(
        made["templateCreate"]["template"]["id"]
            .as_str()
            .unwrap()
            .to_owned(),
    );

    // The rule: a body needs a template, and the template's sections.
    let title = format!("live document {stamp}");
    let base = [
        "document",
        "create",
        "--title",
        &title,
        "--project",
        project,
    ];
    let with = |extra: &[&str]| -> Vec<String> {
        base.iter()
            .chain(extra.iter())
            .map(|s| (*s).to_owned())
            .collect()
    };
    let run = |args: Vec<String>| live.run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let o = run(with(&["--body-file", &full]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let o = run(with(&["--body-file", &partial, "--template", &template]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("\"Notes\""), "{}", stderr(&o));

    // Created, with the body Linear stores; the same title returns it.
    let o = run(with(&[
        "--body-file",
        &full,
        "--template",
        &template,
        "--json",
    ]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let created: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(created["existing"], false);
    assert_eq!(created["project"]["name"], project);
    let id = created["id"].as_str().unwrap().to_owned();
    cleanup.documents.push(id.clone());
    let slug = created["slugId"].as_str().unwrap().to_owned();
    let again = live.json(&[
        "document",
        "create",
        "--title",
        &title,
        "--project",
        project,
        "--body-file",
        &full,
        "--template",
        &template,
    ]);
    assert_eq!(again["existing"], true);
    assert_eq!(again["id"], id.as_str());

    // Read back by slug and by URL, and found by the listing.
    let viewed = live.json(&["document", "view", &slug]);
    assert_eq!(viewed["title"], title.as_str());
    assert!(viewed["content"]
        .as_str()
        .unwrap()
        .contains("A second section."));
    let url = created["url"].as_str().unwrap();
    assert_eq!(live.json(&["document", "view", url])["id"], id.as_str());
    let listed = live.json(&["document", "list", "--project", project, "--title", &title]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());

    // An update sends only what differs; a replaced body is held to the template too.
    let renamed = format!("{title} renamed");
    let upd = live.json(&["document", "update", &slug, "--title", &renamed]);
    assert_eq!(upd["changed"], true);
    assert_eq!(upd["title"], renamed.as_str());
    let same = live.json(&["document", "update", &slug, "--title", &renamed]);
    assert_eq!(same["changed"], false);
    let o = live.run(&["document", "update", &slug, "--body-file", &full]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let newer = write("newer.md", "## Goal\n\nChanged.\n\n## Notes\n\nToo.\n");
    let upd = live.json(&[
        "document",
        "update",
        &slug,
        "--body-file",
        &newer,
        "--template",
        &template,
    ]);
    assert_eq!(upd["changed"], true);
    let viewed = live.json(&["document", "view", &slug]);
    assert!(viewed["content"].as_str().unwrap().contains("Changed."));

    // A project nobody leads is not the viewer's.
    let o = live.run(&[
        "document",
        "create",
        "--title",
        "never made",
        "--project",
        "Finished Project",
    ]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));

    // An initiative: nobody owns a new one, so it is refused until the viewer is its owner.
    let name = format!("live document initiative {stamp}");
    let initiative = live.json(&["initiative", "create", "--name", &name]);
    let initiative_id = initiative["id"].as_str().unwrap().to_owned();
    cleanup.initiatives.push(initiative_id.clone());
    let o = live.run(&[
        "document",
        "create",
        "--title",
        "never made",
        "--initiative",
        &name,
    ]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("no owner"), "{}", stderr(&o));

    let me = live.json(&["workspace", "whoami"]);
    let me = me["user"]["id"].as_str().unwrap().to_owned();
    live.raw(
        "mutation($id: String!, $owner: String!) { initiativeUpdate(id: $id, input: {ownerId: $owner}) { success } }",
        &[&format!("id={initiative_id}"), &format!("owner={me}")],
    );
    let ititle = format!("live initiative document {stamp}");
    let o = live.run(&[
        "document",
        "create",
        "--title",
        &ititle,
        "--initiative",
        &name,
        "--json",
        "--body-file",
        &full,
        "--template",
        &template,
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let idoc: Value = serde_json::from_str(&stdout(&o)).unwrap();
    cleanup
        .documents
        .push(idoc["id"].as_str().unwrap().to_owned());
    assert_eq!(idoc["initiative"]["id"], initiative_id.as_str());
    let listed = live.json(&["document", "list", "--initiative", &name]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let islug = idoc["slugId"].as_str().unwrap();
    let upd = live.json(&[
        "document",
        "update",
        islug,
        "--title",
        &format!("{ititle} renamed"),
    ]);
    assert_eq!(upd["changed"], true);
}
