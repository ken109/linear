//! Reading the sections of an issue template.

use linear_core::queries::Templates;
use linear_core::template::{description_doc, headings_of, is_issue_template, skeleton_of};
use linear_core::wire::{parse_response, ResponseMeta};
use serde_json::json;

fn templates(name: &str) -> Vec<linear_core::types::Template> {
    let body = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    let t: Templates = parse_response(&meta, &body, chrono::Utc::now()).unwrap();
    t.templates
}

#[test]
fn the_sections_are_the_headings_of_the_body_in_order() {
    let all = templates("templates_sections");
    let t = all.iter().find(|t| t.name == "Sectioned Template").unwrap();
    let doc = description_doc(t).unwrap();
    assert_eq!(
        headings_of(&doc),
        ["Background", "Acceptance criteria", "Out of scope"]
    );
    assert_eq!(
        skeleton_of(&doc),
        "## Background\n\n## Acceptance criteria\n\n## Out of scope\n"
    );
}

#[test]
fn a_body_that_is_a_string_inside_the_data_is_accepted_too() {
    let mut all = templates("templates_sections");
    let t = all
        .iter_mut()
        .find(|t| t.name == "Sectioned Template")
        .unwrap();
    let data = t.data().unwrap();
    let encoded = data["descriptionData"].to_string();
    t.template_data = json!({"title": "", "descriptionData": encoded});
    let doc = description_doc(t).unwrap();
    assert_eq!(headings_of(&doc).len(), 3);
}

#[test]
fn headings_inside_lists_and_formatted_text_are_found() {
    let doc = json!({"type": "doc", "content": [
        {"type": "bullet_list", "content": [{"type": "list_item", "content": [
            {"type": "heading", "attrs": {"level": 3}, "content": [
                {"type": "text", "text": "Deep "},
                {"type": "text", "text": "heading", "marks": [{"type": "strong"}]}
            ]}
        ]}]}
    ]});
    assert_eq!(headings_of(&doc), ["Deep heading"]);
}

#[test]
fn a_template_without_headings_or_a_body_has_no_sections() {
    let all = templates("templates");
    // The original fixture's body is one paragraph that merely looks like a heading.
    let doc = description_doc(&all[0]).unwrap();
    assert!(headings_of(&doc).is_empty());
    assert_eq!(skeleton_of(&doc), "");

    let all = templates("templates_sections");
    let project = all.iter().find(|t| !is_issue_template(t)).unwrap();
    let err = description_doc(project).unwrap_err().to_string();
    assert!(err.contains("has no body"), "{err}");
}
