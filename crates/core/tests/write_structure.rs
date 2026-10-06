//! The pieces the milestone, initiative and template writes are built from:
//! inputs, mutations and the markdown-to-ProseMirror conversion.

use chrono::NaiveDate;
use linear_core::inputs::*;
use linear_core::template::{headings_of, markdown_to_doc, skeleton_of};
use linear_core::wire::build_request;
use serde_json::json;

fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

// ------------------------------------------------------------------ milestones

#[test]
fn a_milestone_create_input_always_carries_its_target_date() {
    let minimal = serde_json::to_value(MilestoneCreateInput {
        project_id: "p".into(),
        name: "M1".into(),
        target_date: date("2026-11-01"),
        description: None,
    })
    .unwrap();
    assert_eq!(
        minimal,
        json!({ "projectId": "p", "name": "M1", "targetDate": "2026-11-01" })
    );

    let full = serde_json::to_value(MilestoneCreateInput {
        project_id: "p".into(),
        name: "M1".into(),
        target_date: date("2026-11-01"),
        description: Some("Why".into()),
    })
    .unwrap();
    assert_eq!(full["description"], "Why");
}

#[test]
fn a_milestone_update_input_omits_what_is_not_changed() {
    assert_eq!(
        serde_json::to_value(MilestoneUpdateInput::default()).unwrap(),
        json!({}),
        "an untouched field is omitted, never null"
    );
    assert!(MilestoneUpdateInput::default().is_empty());

    let changed = MilestoneUpdateInput {
        name: Some("New".into()),
        target_date: Some(date("2026-12-01")),
        description: None,
    };
    assert!(!changed.is_empty());
    assert_eq!(
        serde_json::to_value(changed).unwrap(),
        json!({ "name": "New", "targetDate": "2026-12-01" })
    );
}

#[test]
fn the_milestone_mutations_name_the_right_fields() {
    let create = build_request(&milestone_create(MilestoneCreateInput {
        project_id: "p".into(),
        name: "M1".into(),
        target_date: date("2026-11-01"),
        description: None,
    }));
    assert!(
        create.query.contains("projectMilestoneCreate"),
        "{}",
        create.query
    );

    let update = build_request(&milestone_update("m-1", MilestoneUpdateInput::default()));
    assert!(update.query.contains("projectMilestoneUpdate"));
    assert_eq!(update.variables["id"], "m-1");

    let delete = build_request(&milestone_delete("m-1"));
    assert!(delete.query.contains("projectMilestoneDelete"));
    assert_eq!(delete.variables["id"], "m-1");
}

// ------------------------------------------------------------------ initiative, template

#[test]
fn an_initiative_input_omits_a_missing_description() {
    let input = InitiativeCreateInput {
        name: "human-sim".into(),
        description: None,
    };
    assert_eq!(
        serde_json::to_value(&input).unwrap(),
        json!({ "name": "human-sim" })
    );
    let request = build_request(&initiative_create(input));
    assert!(request.query.contains("initiativeCreate"));
}

#[test]
fn a_template_input_sends_the_type_and_the_document() {
    let doc = markdown_to_doc("## A\n");
    let input = TemplateCreateInput {
        kind: "issue".into(),
        name: "Bug".into(),
        team_id: Some("t".into()),
        description: None,
        template_data: json!({ "title": "", "descriptionData": doc }),
    };
    let request = build_request(&template_create(input));
    assert!(request.query.contains("templateCreate"));
    assert_eq!(request.variables["input"]["type"], "issue");
    assert_eq!(request.variables["input"]["teamId"], "t");
    assert!(request.variables["input"].get("description").is_none());
    assert_eq!(
        request.variables["input"]["templateData"]["descriptionData"]["type"],
        "doc"
    );
}

// ------------------------------------------------------------------ markdown

#[test]
fn headings_become_the_sections_of_the_document() {
    let doc = markdown_to_doc("## Background\n\nWhy.\n\n## Acceptance criteria\n\n- one\n- two\n");
    assert_eq!(headings_of(&doc), ["Background", "Acceptance criteria"]);
    assert_eq!(
        skeleton_of(&doc),
        "## Background\n\n## Acceptance criteria\n"
    );
    assert_eq!(doc["content"][0]["type"], "heading");
    assert_eq!(doc["content"][0]["attrs"]["level"], 2);
}

#[test]
fn lists_group_consecutive_items_and_keep_their_kind() {
    let doc = markdown_to_doc("- a\n* b\n1. c\n2. d\ntext\n- e\n");
    let kinds: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["bullet_list", "ordered_list", "paragraph", "bullet_list"]
    );
    assert_eq!(doc["content"][0]["content"].as_array().unwrap().len(), 2);
    assert_eq!(doc["content"][1]["attrs"], json!({ "order": 1 }));
    assert_eq!(doc["content"][1]["content"].as_array().unwrap().len(), 2);
}

#[test]
fn bold_text_gets_a_strong_mark_and_other_asterisks_stay_text() {
    let doc = markdown_to_doc("before **bold** after\n2 * 3 ** 4\n");
    let first = doc["content"][0]["content"].as_array().unwrap();
    assert_eq!(first.len(), 3);
    assert_eq!(first[1]["text"], "bold");
    assert_eq!(first[1]["marks"], json!([{ "type": "strong" }]));
    // A lone `**` has nothing to close; it is plain text.
    assert_eq!(doc["content"][1]["content"][0]["text"], "2 * 3 ** 4");
}

#[test]
fn a_hash_without_a_space_is_not_a_heading_and_blank_lines_vanish() {
    let doc = markdown_to_doc("#tag\n\n\n####### seven\n");
    let kinds: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["paragraph", "paragraph"]);
    assert!(headings_of(&doc).is_empty());
}
