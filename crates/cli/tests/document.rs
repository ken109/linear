//! `linear document list|view|create|update` against a mock Linear that
//! answers by operation name: reading, the ownership of the parent, the
//! `template-sections` rule on a body, the idempotent return and the mutations.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const OWNER_INITIATIVE: &str = "00000000-0000-4000-8000-000000000101";
const ORPHAN_INITIATIVE: &str = "00000000-0000-4000-8000-000000000102";
const DOC_SLUG: &str = "1a2b3c4d5e6f";
const NEW_DOC: &str = "00000000-0000-4000-8000-0000000004b1";

const MUTATIONS: [&str; 2] = ["DocCreate", "DocUpdate"];

fn assert_nothing_written(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter().all(|o| !MUTATIONS.contains(&o.as_str())),
        "a mutation was sent: {ops:?}"
    );
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_str::<Value>(&fixture(name)).unwrap()["data"].clone()
}

fn documents() -> Value {
    fixture_json("documents")
}

/// The documents of the fixture, narrowed to the ones that match `keep`.
fn only(keep: impl Fn(&Value) -> bool) -> Reply {
    let mut v = documents();
    v["documents"]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|d| keep(d));
    data(v)
}

fn no_documents() -> Reply {
    only(|_| false)
}

fn initiatives() -> Reply {
    ok(&fixture("initiatives"))
}

/// A `DocView` answer: the first document, parented as asked.
fn view_of(parent: &str) -> Reply {
    let mut v = fixture_json("document_view");
    let d = &mut v["document"];
    match parent {
        "project" => {}
        "initiative" => {
            d["project"] = Value::Null;
            d["initiative"] = documents()["documents"]["nodes"][1]["initiative"].clone();
        }
        "issue" => {
            d["project"] = Value::Null;
            d["issue"] = documents()["documents"]["nodes"][2]["issue"].clone();
        }
        other => panic!("{other}"),
    }
    data(v)
}

fn doc_payload(field: &str, changes: Value) -> Reply {
    let mut d = documents()["documents"]["nodes"][0].clone();
    for (k, v) in changes.as_object().unwrap() {
        d[k] = v.clone();
    }
    data(json!({ field: { "success": true, "document": d } }))
}

fn project_routes(lead: Option<&str>) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        ("ProjectOwnershipQuery", vec![ownership(PROJECT, lead)]),
    ]
}

fn with(
    mut routes: Vec<(&'static str, Vec<Reply>)>,
    extra: Vec<(&'static str, Vec<Reply>)>,
) -> Vec<(&'static str, Vec<Reply>)> {
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

// ------------------------------------------------------------------ list

#[test]
fn list_shows_each_document_with_what_it_hangs_off() {
    let sb = workspace();
    let mock = Routed::start(vec![("DocList", vec![data(documents())])]);
    let o = run(&sb, &mock, &["document", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let out = stdout(&o);
    for want in [
        "SLUG",
        "TITLE",
        DOC_SLUG,
        "Design notes",
        "project: Fixture Project",
        "initiative: Example Initiative",
        "issue: EX-23",
        "2026-10-06",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    // No filter without a flag; a read writes nothing.
    assert_eq!(mock.of("DocList")[0]["filter"], Value::Null);
    assert_eq!(mock.ops(), ["DocList"]);

    let o = run(&sb, &mock, &["document", "list", "--quiet"]);
    assert_eq!(stdout(&o).lines().next(), Some(DOC_SLUG));
}

#[test]
fn list_json_is_tagged_with_the_workspace() {
    let sb = workspace();
    let mock = Routed::start(vec![("DocList", vec![data(documents())])]);
    let o = run(&sb, &mock, &["document", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["workspace"], "example");
    assert_eq!(rows[0]["slugId"], DOC_SLUG);
    assert_eq!(rows[0]["project"]["name"], "Fixture Project");
    assert_eq!(rows[1]["initiative"]["id"], OWNER_INITIATIVE);
    assert!(
        rows[0].get("content").is_none(),
        "a listing does not carry bodies"
    );
}

#[test]
fn list_narrows_by_project_initiative_and_title() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("ProjectRefs", vec![project_refs()]),
        ("InitiativeList", vec![initiatives()]),
        ("DocList", vec![data(documents())]),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "list",
            "--project",
            "Fixture Project",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("DocList")[0]["filter"],
        json!({"project": {"id": {"eq": PROJECT}}})
    );

    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "list",
            "--initiative",
            "example initiative",
            "--title",
            "Roadmap",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("DocList")[1]["filter"],
        json!({"initiative": {"id": {"eq": OWNER_INITIATIVE}}, "title": {"eqIgnoreCase": "Roadmap"}})
    );

    // A project and an initiative together: the flags exclude each other.
    let o = run(
        &sb,
        &mock,
        &["document", "list", "--project", "x", "--initiative", "y"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn list_limit_says_when_it_cut_the_list() {
    let sb = workspace();
    let mut v = documents();
    v["documents"]["pageInfo"] = json!({"hasNextPage": true, "endCursor": "c1"});
    let mock = Routed::start(vec![("DocList", vec![data(v)])]);
    let o = run(&sb, &mock, &["document", "list", "--limit", "2", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).lines().count(), 2);
    assert!(
        stderr(&o).contains("showing the first 2 results"),
        "{}",
        stderr(&o)
    );

    let empty = Routed::start(vec![("DocList", vec![no_documents()])]);
    let o = run(&sb, &empty, &["document", "list"]);
    assert_eq!(stdout(&o).trim(), "No documents found.");
}

// ------------------------------------------------------------------ view

#[test]
fn view_prints_the_document_and_its_body() {
    let sb = workspace();
    let mock = Routed::start(vec![("DocView", vec![view_of("project")])]);
    let o = run(&sb, &mock, &["document", "view", DOC_SLUG]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Design notes",
        "In:",
        "project: Fixture Project",
        "Creator:",
        "Alice Example",
        "https://linear.app/example/document/design-notes-1a2b3c4d5e6f",
        "## Goal",
        "Write it down.",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    assert_eq!(mock.of("DocView")[0]["id"], DOC_SLUG);

    let o = run(&sb, &mock, &["document", "view", DOC_SLUG, "--quiet"]);
    assert_eq!(stdout(&o).trim(), DOC_SLUG);
}

#[test]
fn view_json_has_the_body_and_a_url_is_reduced_to_its_slug() {
    let sb = workspace();
    let mock = Routed::start(vec![("DocView", vec![view_of("project")])]);
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "view",
            "https://linear.app/example/document/design-notes-1a2b3c4d5e6f?x=1",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["title"], "Design notes");
    assert!(v["content"].as_str().unwrap().contains("See the roadmap."));
    assert_eq!(mock.of("DocView")[0]["id"], "design-notes-1a2b3c4d5e6f");
}

#[test]
fn view_of_a_document_linear_does_not_have_fails() {
    let sb = workspace();
    let mock = Routed::start(vec![(
        "DocView",
        vec![graphql_error("Entity not found: Document")],
    )]);
    let o = run(&sb, &mock, &["document", "view", "nope"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("Entity not found"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ create

#[test]
fn a_document_is_created_in_a_project_the_viewer_leads() {
    let sb = workspace();
    let body = write_file(&sb, "body.md", "## Goal\n\nWrite it.\n\n");
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![
            ("DocList", vec![no_documents()]),
            (
                "DocCreate",
                vec![doc_payload(
                    "documentCreate",
                    json!({"id": NEW_DOC, "title": "Plan"}),
                )],
            ),
        ],
    ));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            " Plan ",
            "--project",
            "Fixture Project",
            "--body-file",
            &body,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["existing"], false);
    assert_eq!(v["id"], NEW_DOC);

    // Reads, the look for a document with that title, then one mutation.
    assert_eq!(
        mock.ops(),
        [
            "Whoami",
            "ProjectRefs",
            "ProjectOwnershipQuery",
            "DocList",
            "DocCreate"
        ]
    );
    assert_eq!(
        mock.of("DocList")[0]["filter"],
        json!({"project": {"id": {"eq": PROJECT}}, "title": {"eqIgnoreCase": "Plan"}})
    );
    assert_eq!(
        mock.of("DocCreate")[0]["input"],
        json!({"title": "Plan", "content": "## Goal\n\nWrite it.", "projectId": PROJECT})
    );
}

#[test]
fn a_document_is_created_in_an_initiative_the_viewer_owns() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives()]),
        ("DocList", vec![no_documents()]),
        (
            "DocCreate",
            vec![doc_payload(
                "documentCreate",
                json!({"id": NEW_DOC, "title": "Plan"}),
            )],
        ),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--initiative",
            "Example Initiative",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), DOC_SLUG);
    assert_eq!(
        mock.of("DocCreate")[0]["input"],
        json!({"title": "Plan", "initiativeId": OWNER_INITIATIVE})
    );
}

#[test]
fn a_parent_the_viewer_does_not_own_refuses_the_write() {
    let sb = workspace();
    // A project led by somebody else.
    let mock = Routed::start(with(
        project_routes(Some(BOT)),
        vec![("DocList", vec![no_documents()])],
    ));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert!(stderr(&o).contains("not the lead"), "{}", stderr(&o));
    assert_nothing_written(&mock);

    // An initiative with no owner.
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives()]),
        ("DocList", vec![no_documents()]),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--initiative",
            ORPHAN_INITIATIVE,
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("no owner"), "{}", stderr(&o));
    assert_nothing_written(&mock);
    assert!(
        !mock.ops().contains(&"DocList".to_owned()),
        "refused before looking further"
    );

    // Lenient ownership relaxes issues only, not this.
    let lenient = lenient_workspace_with_rules(&[]);
    let mock = Routed::start(project_routes(Some(BOT)));
    let o = run(
        &lenient,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
}

#[test]
fn a_title_that_the_parent_already_has_is_returned_and_nothing_is_created() {
    let sb = workspace();
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![("DocList", vec![only(|d| d["title"] == "Design notes")])],
    ));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Design notes",
            "--project",
            "Fixture Project",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["slugId"], DOC_SLUG);
    assert_nothing_written(&mock);

    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Design notes",
            "--project",
            "Fixture Project",
        ],
    );
    assert!(
        stdout(&o).contains("already exists, nothing created"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn a_create_without_a_parent_or_a_title_is_refused_before_any_request() {
    let sb = workspace();
    let mock = Routed::start(vec![]);
    for args in [
        vec!["document", "create", "--title", "Plan"],
        vec![
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "a",
            "--initiative",
            "b",
        ],
        vec![
            "document",
            "create",
            "--title",
            "  ",
            "--project",
            "Fixture Project",
        ],
        vec!["document", "create", "--project", "Fixture Project"],
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

// ------------------------------------------------------------------ the validator on a body

/// What `Templates` answers: the issue and project templates of the fixture and a document template.
fn templates_with_document() -> Reply {
    let heading = |t: &str| json!({"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": t}]});
    let tpl = json!({
        "id": "tpl-doc", "name": "Plan", "description": null, "type": "document",
        "team": null, "updatedAt": "2026-10-06T00:00:00Z",
        "templateData": json!({"descriptionData": {"type": "doc", "content": [heading("Goal"), heading("Notes")]}}).to_string(),
    });
    let mut v = fixture_json("templates_sections");
    v["templates"].as_array_mut().unwrap().push(tpl);
    data(v)
}

fn rule_routes(extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut added = vec![
        ("Templates", vec![templates_with_document()]),
        ("DocList", vec![no_documents()]),
    ];
    added.extend(extra);
    with(project_routes(Some(ALICE)), added)
}

fn args_with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = base.to_vec();
    a.extend_from_slice(extra);
    a
}

#[test]
fn template_sections_holds_a_new_documents_body_to_a_document_template() {
    let sb = workspace_with_rules(&["template-sections"]);
    let full = write_file(
        &sb,
        "full.md",
        "## Goal\n\nWrite it.\n\n## Notes\n\nMore.\n",
    );
    let partial = write_file(&sb, "partial.md", "## Goal\n\nWrite it.\n");
    let mock = Routed::start(rule_routes(vec![(
        "DocCreate",
        vec![doc_payload("documentCreate", json!({"id": NEW_DOC}))],
    )]));
    let base = [
        "document",
        "create",
        "--title",
        "Plan",
        "--project",
        "Fixture Project",
    ];

    // No template: refused (exit 5), nothing written.
    let o = run(&sb, &mock, &args_with(&base, &["--body-file", &full]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("a template is required"),
        "{}",
        stderr(&o)
    );
    // A section missing.
    let o = run(
        &sb,
        &mock,
        &args_with(&base, &["--body-file", &partial, "--template", "Plan"]),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("\"Notes\""), "{}", stderr(&o));
    // An issue template is not a document template.
    let o = run(
        &sb,
        &mock,
        &args_with(
            &base,
            &["--body-file", &full, "--template", "Sectioned Template"],
        ),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("no document template named"),
        "{}",
        stderr(&o)
    );
    assert_nothing_written(&mock);

    // Filled in: written.
    let o = run(
        &sb,
        &mock,
        &args_with(&base, &["--body-file", &full, "--template", "Plan"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("DocCreate").len(), 1);
}

#[test]
fn without_the_rule_a_template_is_ignored_with_a_note() {
    let sb = workspace();
    let body = write_file(&sb, "body.md", "just text\n");
    let mock = Routed::start(rule_routes(vec![(
        "DocCreate",
        vec![doc_payload("documentCreate", json!({"id": NEW_DOC}))],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
            "--body-file",
            &body,
            "--template",
            "Plan",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--template is ignored"),
        "{}",
        stderr(&o)
    );
    assert!(!mock.ops().contains(&"Templates".to_owned()));
}

#[test]
fn the_rule_can_be_narrowed_away_from_documents() {
    let sb = workspace_with_setting(
        &["template-sections"],
        "rule_operations = { \"template-sections\" = [\"issue_create\"] }",
    );
    let mock = Routed::start(rule_routes(vec![(
        "DocCreate",
        vec![doc_payload("documentCreate", json!({"id": NEW_DOC}))],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

// ------------------------------------------------------------------ update

fn update_routes(parent: &str, lead: Option<&str>) -> Vec<(&'static str, Vec<Reply>)> {
    with(
        project_routes(lead),
        vec![
            ("DocView", vec![view_of(parent)]),
            ("InitiativeList", vec![initiatives()]),
            (
                "DocUpdate",
                vec![doc_payload("documentUpdate", json!({"title": "Renamed"}))],
            ),
        ],
    )
}

#[test]
fn a_document_of_a_project_the_viewer_leads_is_renamed_and_only_what_differs_is_sent() {
    let sb = workspace();
    let body = write_file(
        &sb,
        "same.md",
        "## Goal\n\nWrite it down.\n\n## Notes\n\nSee the roadmap.\n",
    );
    let mock = Routed::start(update_routes("project", Some(ALICE)));
    // The body is the one it has: not sent. The title differs: sent.
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--title",
            "Renamed",
            "--body-file",
            &body,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["changed"], true);
    assert_eq!(v["title"], "Renamed");
    assert_eq!(
        mock.ops(),
        [
            "Whoami",
            "DocView",
            "ProjectRefs",
            "ProjectOwnershipQuery",
            "DocUpdate"
        ]
    );
    let call = &mock.of("DocUpdate")[0];
    assert_eq!(call["id"], "00000000-0000-4000-8000-000000000401");
    assert_eq!(call["input"], json!({"title": "Renamed"}));
}

#[test]
fn a_new_body_replaces_the_old_one() {
    let sb = workspace();
    let body = write_file(&sb, "new.md", "## Goal\n\nChanged.\n");
    let mock = Routed::start(update_routes("project", Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--body-file",
            &body,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("DocUpdate")[0]["input"],
        json!({"content": "## Goal\n\nChanged."})
    );
}

#[test]
fn an_update_that_changes_nothing_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(update_routes("project", Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--title",
            "Design notes",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], false);
    assert_nothing_written(&mock);

    let o = run(&sb, &mock, &["document", "update", DOC_SLUG]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("nothing to change"), "{}", stderr(&o));

    let empty = write_file(&sb, "empty.md", "  \n");
    let o = run(
        &sb,
        &mock,
        &["document", "update", DOC_SLUG, "--body-file", &empty],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--body-file is empty"),
        "{}",
        stderr(&o)
    );
    assert_nothing_written(&mock);
}

#[test]
fn only_the_lead_or_the_owner_may_change_a_document() {
    let sb = workspace();
    // The project is led by somebody else.
    let mock = Routed::start(update_routes("project", Some(BOT)));
    let o = run(
        &sb,
        &mock,
        &["document", "update", DOC_SLUG, "--title", "Renamed"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_nothing_written(&mock);

    // A document of an initiative the viewer owns: allowed.
    let mock = Routed::start(update_routes("initiative", Some(BOT)));
    let o = run(
        &sb,
        &mock,
        &[
            "document", "update", DOC_SLUG, "--title", "Renamed", "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("DocUpdate").len(), 1);
}

#[test]
fn a_document_of_an_issue_is_read_but_not_written() {
    let sb = workspace();
    let mock = Routed::start(update_routes("issue", Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &["document", "update", DOC_SLUG, "--title", "Renamed"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("issue: EX-23")
            && stderr(&o).contains("neither a project nor an initiative"),
        "{}",
        stderr(&o)
    );
    assert_nothing_written(&mock);
}

#[test]
fn template_sections_holds_a_replaced_body_but_not_a_rename() {
    let sb = workspace_with_rules(&["template-sections"]);
    let partial = write_file(&sb, "partial.md", "## Goal\n\nChanged.\n");
    let full = write_file(&sb, "full.md", "## Goal\n\nChanged.\n\n## Notes\n\nToo.\n");
    let routes = || {
        with(
            update_routes("project", Some(ALICE)),
            vec![("Templates", vec![templates_with_document()])],
        )
    };

    let mock = Routed::start(routes());
    // A rename leaves the body alone: nothing to check.
    let o = run(
        &sb,
        &mock,
        &["document", "update", DOC_SLUG, "--title", "Renamed"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    // A body without a template, and a body with a section missing.
    let mock = Routed::start(routes());
    let o = run(
        &sb,
        &mock,
        &["document", "update", DOC_SLUG, "--body-file", &full],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("to replace the body"), "{}", stderr(&o));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--body-file",
            &partial,
            "--template",
            "Plan",
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_nothing_written(&mock);

    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--body-file",
            &full,
            "--template",
            "Plan",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("DocUpdate").len(), 1);
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_of_create_plans_the_document() {
    let sb = workspace();
    let body = write_file(&sb, "body.md", "## Goal\n\nWrite it.\n\n");
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![("DocList", vec![no_documents()])],
    ));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "document",
                "create",
                "--title",
                " Plan ",
                "--project",
                "Fixture Project",
                "--body-file",
                &body,
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "document create");
    assert_eq!(planned(&v), ["DocCreate"]);
    assert_eq!(
        v["mutations"][0]["variables"]["input"],
        json!({"title": "Plan", "content": "## Goal\n\nWrite it.", "projectId": PROJECT})
    );

    // Somebody else's project: exit 4. The template rule: exit 5.
    let mock = Routed::start(with(
        project_routes(Some(BOT)),
        vec![("DocList", vec![no_documents()])],
    ));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();

    let ruled = workspace_with_rules(&["template-sections"]);
    let mock = Routed::start(rule_routes(vec![]));
    let o = run(
        &ruled,
        &mock,
        &[
            "document",
            "create",
            "--title",
            "Plan",
            "--project",
            "Fixture Project",
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_dry_run_of_update_lists_what_differs() {
    let sb = workspace();
    let mock = Routed::start(update_routes("project", Some(ALICE)));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "document",
                "update",
                DOC_SLUG,
                "--title",
                "Renamed",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "document update");
    assert_eq!(v["changed"], json!(["title"]));
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "id": "00000000-0000-4000-8000-000000000401", "input": { "title": "Renamed" } })
    );

    let mock = Routed::start(update_routes("project", Some(BOT)));
    let o = run(
        &sb,
        &mock,
        &[
            "document",
            "update",
            DOC_SLUG,
            "--title",
            "Renamed",
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}
