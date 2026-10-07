//! Linear documents: the fragments, the filter and inputs they send, who may
//! write one, and the `template-sections` rule applied to a document body.

use cynic::GraphQlResponse;
use linear_core::config::Rule;
use linear_core::docs::*;
use linear_core::guard::{check, DenyReason, Viewer, Write};
use linear_core::rules::{Draft, Fetched, Operation, Outcome, RuleSet, TemplateKind};
use linear_core::types::{PageVars, Template};
use linear_core::wire::build_request;
use serde_json::json;

const PROJECT: &str = "00000000-0000-4000-8000-000000000006";
const INITIATIVE: &str = "00000000-0000-4000-8000-000000000101";
const ALICE: &str = "00000000-0000-4000-8000-000000000001";

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let r: GraphQlResponse<T> = serde_json::from_str(&text).unwrap();
    r.data.expect("data")
}

fn page() -> PageVars {
    PageVars {
        first: 50,
        after: None,
    }
}

// ------------------------------------------------------------------ reading

#[test]
fn a_listing_parses_and_names_what_each_document_hangs_off() {
    let data: DocList = fixture("documents");
    let docs = &data.documents.nodes;
    assert_eq!(docs.len(), 3);
    assert_eq!(docs[0].parent_label(), "project: Fixture Project");
    assert_eq!(docs[1].parent_label(), "initiative: Example Initiative");
    assert_eq!(docs[2].parent_label(), "issue: EX-23");
    assert_eq!(docs[0].slug_id, "1a2b3c4d5e6f");
    assert_eq!(docs[0].creator.as_ref().unwrap().name, "Alice Example");
}

#[test]
fn a_view_carries_the_body_next_to_the_document() {
    let data: DocView = fixture("document_view");
    assert_eq!(data.document.title, "Design notes");
    assert!(data
        .detail
        .content
        .as_deref()
        .unwrap()
        .starts_with("## Goal"));
    // Serialized, it reads back the same.
    let v = serde_json::to_value(&data).unwrap();
    assert_eq!(v["document"]["slugId"], "1a2b3c4d5e6f");
    assert_eq!(v["detail"]["content"], data.detail.content.clone().unwrap());

    let req = build_request(&doc_view("title-1a2b3c4d5e6f"));
    assert_eq!(req.operation_name.as_deref(), Some("DocView"));
    assert_eq!(req.variables, json!({"id": "title-1a2b3c4d5e6f"}));
}

#[test]
fn the_listing_filter_names_the_parent_by_id_and_the_title_ignoring_case() {
    assert_eq!(DocQuery::default().filter(), None);

    let filter = |q: DocQuery| {
        build_request(&doc_list(DocListVars::new(page(), q.filter()))).variables["filter"].clone()
    };
    assert_eq!(
        filter(DocQuery {
            project_id: Some(PROJECT.into()),
            ..DocQuery::default()
        }),
        json!({"project": {"id": {"eq": PROJECT}}})
    );
    assert_eq!(
        filter(DocQuery {
            initiative_id: Some(INITIATIVE.into()),
            title: Some("Roadmap".into()),
            ..DocQuery::default()
        }),
        json!({"initiative": {"id": {"eq": INITIATIVE}}, "title": {"eqIgnoreCase": "Roadmap"}})
    );
}

// ------------------------------------------------------------------ writing

#[test]
fn a_create_sends_the_parent_and_only_what_was_given() {
    let op = doc_create(DocCreateInput {
        title: "Plan".into(),
        content: None,
        project_id: Some(PROJECT.into()),
        initiative_id: None,
    });
    let req = build_request(&op);
    assert_eq!(req.operation_name.as_deref(), Some("DocCreate"));
    assert_eq!(
        req.variables["input"],
        json!({"title": "Plan", "projectId": PROJECT})
    );

    let op = doc_create(DocCreateInput {
        title: "Plan".into(),
        content: Some("## A\n\nx".into()),
        project_id: None,
        initiative_id: Some(INITIATIVE.into()),
    });
    assert_eq!(
        build_request(&op).variables["input"],
        json!({"title": "Plan", "content": "## A\n\nx", "initiativeId": INITIATIVE})
    );
}

#[test]
fn an_update_leaves_alone_what_it_does_not_name() {
    assert!(DocUpdateInput::default().is_empty());
    let input = DocUpdateInput {
        title: Some("New".into()),
        content: None,
    };
    assert!(!input.is_empty());
    let req = build_request(&doc_update("doc-id", input));
    assert_eq!(req.operation_name.as_deref(), Some("DocUpdate"));
    assert_eq!(req.variables["id"], "doc-id");
    assert_eq!(req.variables["input"], json!({"title": "New"}));
    assert_eq!(
        build_request(&doc_update("doc-id", DocUpdateInput::default())).variables["input"],
        json!({})
    );
}

// ------------------------------------------------------------------ ownership

#[test]
fn only_the_owner_of_an_initiative_may_write_under_it() {
    let me = Viewer::new("w", ALICE);
    assert!(check(&me, &Write::InitiativeUpdate { owner: Some(ALICE) }, false).is_ok());

    let other = check(
        &me,
        &Write::InitiativeUpdate {
            owner: Some("someone-else"),
        },
        false,
    )
    .unwrap_err();
    assert_eq!(other.reason, DenyReason::InitiativeNotOwned);
    assert!(
        other.message.contains("owned by someone else"),
        "{}",
        other.message
    );

    let nobody = check(&me, &Write::InitiativeUpdate { owner: None }, false).unwrap_err();
    assert!(nobody.message.contains("no owner"), "{}", nobody.message);
    // `allow_foreign` covers creating an issue and nothing here.
    assert!(check(&me, &Write::InitiativeUpdate { owner: None }, true).is_err());
}

// ------------------------------------------------------------------ the validator

fn document_template() -> Template {
    let heading = |t: &str| json!({"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": t}]});
    let data = json!({
        "descriptionData": {"type": "doc", "content": [heading("Goal"), heading("Notes")]}
    })
    .to_string();
    serde_json::from_value(json!({
        "id": "tpl-doc", "name": "Plan", "description": null, "type": "document",
        "team": null, "templateData": data, "updatedAt": "2026-10-06T00:00:00Z",
    }))
    .unwrap()
}

fn fetched() -> Fetched {
    Fetched {
        templates: vec![document_template()],
        ..Fetched::default()
    }
}

#[test]
fn template_sections_holds_a_document_body_to_a_document_template() {
    let rules = RuleSet::new(&[Rule::TemplateSections]);
    assert_eq!(
        Operation::DocumentCreate.template_kind(),
        TemplateKind::Document
    );
    assert_eq!(TemplateKind::Document.linear_type(), "document");
    assert!(Operation::DocumentCreate.is_create() && !Operation::DocumentUpdate.is_create());
    assert!(rules.applies(Rule::TemplateSections, Operation::DocumentCreate));
    assert!(rules.applies(Rule::TemplateSections, Operation::DocumentUpdate));
    // The other rules are about issues.
    let all = RuleSet::new(&[Rule::SourceAttachment, Rule::LabelGroupsExclusive]);
    assert!(!all.applies(Rule::SourceAttachment, Operation::DocumentCreate));
    assert!(!all.applies(Rule::LabelGroupsExclusive, Operation::DocumentUpdate));

    // The template to read is a document one.
    let draft = Draft::new(Operation::DocumentCreate)
        .template("Plan")
        .body("x");
    let needs = rules.needs(&draft);
    assert_eq!(needs.templates[0].kind, TemplateKind::Document);

    let ok = Draft::new(Operation::DocumentCreate)
        .template("Plan")
        .body("## Goal\n\nWrite.\n\n## Notes\n\nMore.\n");
    assert_eq!(rules.check(&ok, &fetched()), Ok(Outcome::Proceed));

    let partial = Draft::new(Operation::DocumentUpdate)
        .template("Plan")
        .body("## Goal\n\nWrite.\n");
    let err = rules.check(&partial, &fetched()).unwrap_err();
    assert!(err.to_string().contains("\"Notes\""), "{err}");

    // A body without a template, on create and on update: refused. A title-only update: not checked.
    let err = rules
        .check(&Draft::new(Operation::DocumentCreate).body("x"), &fetched())
        .unwrap_err();
    assert!(err.to_string().contains("a template is required"), "{err}");
    let err = rules
        .check(&Draft::new(Operation::DocumentUpdate).body("x"), &fetched())
        .unwrap_err();
    assert!(err.to_string().contains("to replace the body"), "{err}");
    assert_eq!(
        rules.check(&Draft::new(Operation::DocumentUpdate), &fetched()),
        Ok(Outcome::Proceed)
    );

    // An issue template of the same name is not a document template.
    let mut issue_only = fetched();
    issue_only.templates[0].type_ = "issue".into();
    let err = rules.check(&ok, &issue_only).unwrap_err();
    assert!(
        err.to_string()
            .contains("no document template named \"Plan\""),
        "{err}"
    );
}
