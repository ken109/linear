//! The pieces project writes are built from: inputs, mutations, the queries a
//! write reads first, and name matching for project statuses.

use chrono::NaiveDate;
use linear_core::inputs::Patch;
use linear_core::project_write::*;
use linear_core::types::{ProjectStatus, ProjectStatusType, ProjectUpdateHealthType};
use linear_core::wire::build_request;
use serde_json::json;

fn status(id: &str, name: &str, type_: ProjectStatusType) -> ProjectStatus {
    ProjectStatus {
        id: cynic::Id::new(id),
        name: name.to_owned(),
        type_,
    }
}

// ------------------------------------------------------------------ inputs

#[test]
fn an_update_that_names_nothing_serializes_as_an_empty_object() {
    let none = ProjectUpdateInput::default();
    assert!(none.is_empty());
    assert_eq!(serde_json::to_value(&none).unwrap(), json!({}));
}

#[test]
fn an_update_patch_can_keep_clear_or_set() {
    let v = serde_json::to_value(ProjectUpdateInput {
        content: Patch::Clear,
        target_date: Patch::Set(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()),
        lead_id: Patch::Set("u-1".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        v,
        json!({ "content": null, "targetDate": "2026-12-31", "leadId": "u-1" })
    );

    let cleared = serde_json::to_value(ProjectUpdateInput {
        target_date: Patch::Clear,
        lead_id: Patch::Clear,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(cleared, json!({ "targetDate": null, "leadId": null }));
}

#[test]
fn an_update_knows_whether_it_changes_anything() {
    for input in [
        ProjectUpdateInput {
            name: Some("n".into()),
            ..Default::default()
        },
        ProjectUpdateInput {
            content: Patch::Clear,
            ..Default::default()
        },
        ProjectUpdateInput {
            status_id: Some("s".into()),
            ..Default::default()
        },
        ProjectUpdateInput {
            lead_id: Patch::Clear,
            ..Default::default()
        },
        ProjectUpdateInput {
            sort_order: Some(1.5),
            ..Default::default()
        },
        ProjectUpdateInput {
            priority_sort_order: Some(1.5),
            ..Default::default()
        },
    ] {
        assert!(!input.is_empty());
    }
}

#[test]
fn a_create_input_omits_what_was_not_given() {
    let only_required = serde_json::to_value(ProjectCreateInput {
        name: "P".into(),
        team_ids: vec!["t".into()],
        description: None,
        content: None,
        lead_id: None,
        target_date: None,
    })
    .unwrap();
    assert_eq!(only_required, json!({ "name": "P", "teamIds": ["t"] }));

    let full = serde_json::to_value(ProjectCreateInput {
        name: "P".into(),
        team_ids: vec!["t".into()],
        description: Some("Summary".into()),
        content: Some("Body".into()),
        lead_id: Some("u".into()),
        target_date: NaiveDate::from_ymd_opt(2026, 12, 1),
    })
    .unwrap();
    assert_eq!(
        full,
        json!({
            "name": "P", "teamIds": ["t"], "description": "Summary", "content": "Body",
            "leadId": "u", "targetDate": "2026-12-01",
        })
    );
}

// ------------------------------------------------------------------ mutations

#[test]
fn the_mutations_name_the_right_fields() {
    let create = build_request(&project_create(ProjectCreateInput {
        name: "P".into(),
        team_ids: vec!["t".into()],
        description: None,
        content: None,
        lead_id: None,
        target_date: None,
    }));
    assert!(create.query.contains("projectCreate"), "{}", create.query);
    assert_eq!(create.operation_name.as_deref(), Some("ProjectCreate"));

    let update = build_request(&project_update(
        "p-1",
        ProjectUpdateInput {
            status_id: Some("s".into()),
            ..Default::default()
        },
    ));
    assert!(update.query.contains("projectUpdate"), "{}", update.query);
    assert_eq!(update.operation_name.as_deref(), Some("ProjectUpdate"));
    assert_eq!(update.variables["id"], "p-1");
    assert_eq!(update.variables["input"], json!({ "statusId": "s" }));

    let delete = build_request(&project_delete("p-1"));
    assert!(delete.query.contains("projectDelete"), "{}", delete.query);
    assert_eq!(delete.variables["id"], "p-1");

    let status_update = build_request(&project_update_create(StatusUpdateCreateInput {
        project_id: "p-1".into(),
        health: ProjectUpdateHealthType::AtRisk,
        body: "Where it stands.".into(),
    }));
    assert!(
        status_update.query.contains("projectUpdateCreate"),
        "{}",
        status_update.query
    );
    assert_eq!(
        status_update.variables["input"],
        json!({ "projectId": "p-1", "health": "atRisk", "body": "Where it stands." })
    );

    let link = build_request(&initiative_to_project_create("i-1", "p-1"));
    assert!(link.query.contains("initiativeToProjectCreate"));
    assert_eq!(
        link.variables["input"],
        json!({ "initiativeId": "i-1", "projectId": "p-1" })
    );
}

// ------------------------------------------------------------------ reads

#[test]
fn the_twin_check_asks_for_unfinished_projects_by_exact_name() {
    let req = build_request(&unfinished_named("Same Name"));
    assert_eq!(
        req.variables["filter"],
        json!({
            "name": { "eq": "Same Name" },
            "status": { "type": { "nin": ["completed", "canceled"] } },
        })
    );
    assert!(req.query.contains("projects"), "{}", req.query);
}

#[test]
fn the_order_query_asks_for_both_ordering_values_and_the_lead() {
    let req = build_request(&project_order("p-1"));
    for field in ["sortOrder", "prioritySortOrder", "lead", "slugId"] {
        assert!(req.query.contains(field), "{field} missing: {}", req.query);
    }
    assert_eq!(req.variables["id"], "p-1");
}

// ------------------------------------------------------------------ matching

#[test]
fn a_project_status_is_matched_by_name_ignoring_case_or_by_id() {
    let rows = vec![
        status("s-1", "Backlog", ProjectStatusType::Backlog),
        status("s-2", "In Progress", ProjectStatusType::Started),
        status("s-3", "Completed", ProjectStatusType::Completed),
    ];
    assert_eq!(
        match_project_status(&rows, "in progress")
            .unwrap()
            .id
            .inner(),
        "s-2"
    );
    assert_eq!(
        match_project_status(&rows, "Completed").unwrap().id.inner(),
        "s-3"
    );
    assert_eq!(match_project_status(&rows, "s-1").unwrap().name, "Backlog");

    let err = match_project_status(&rows, "Done").unwrap_err().to_string();
    assert!(err.contains("no project status"), "{err}");
    assert!(err.contains("Backlog, In Progress, Completed"), "{err}");
}

#[test]
fn an_exact_status_name_wins_over_a_differently_cased_twin() {
    let rows = vec![
        status("s-1", "done", ProjectStatusType::Completed),
        status("s-2", "Done", ProjectStatusType::Completed),
    ];
    assert_eq!(
        match_project_status(&rows, "Done").unwrap().id.inner(),
        "s-2"
    );
    let err = match_project_status(&rows, "DONE").unwrap_err().to_string();
    assert!(err.contains("ambiguous"), "{err}");
}
