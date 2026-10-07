//! The pieces initiative updates, project links and status updates are built from.

use chrono::NaiveDate;
use linear_core::initiative_write::*;
use linear_core::types::{InitiativeStatus, InitiativeUpdateHealthType};
use linear_core::wire::build_request;
use serde_json::json;

#[test]
fn an_update_that_names_nothing_serializes_as_an_empty_object() {
    let none = InitiativeUpdateInput::default();
    assert!(none.is_empty());
    assert_eq!(serde_json::to_value(&none).unwrap(), json!({}));
}

#[test]
fn an_update_sends_only_what_it_names() {
    let input = InitiativeUpdateInput {
        name: Some("Renamed".into()),
        status: Some(InitiativeStatus::Completed),
        target_date: NaiveDate::from_ymd_opt(2026, 12, 31),
        owner_id: Some("u-1".into()),
        ..Default::default()
    };
    assert!(!input.is_empty());
    assert_eq!(
        serde_json::to_value(&input).unwrap(),
        json!({
            "name": "Renamed", "status": "Completed",
            "targetDate": "2026-12-31", "ownerId": "u-1"
        })
    );
    let request = build_request(&initiative_update("i-1", input));
    assert!(request.query.contains("initiativeUpdate"));
    assert_eq!(request.variables["id"], "i-1");
}

#[test]
fn the_link_mutations_name_the_link_and_the_project() {
    let request = build_request(&initiative_to_project_delete("link-1"));
    assert!(request.query.contains("initiativeToProjectDelete"));
    assert_eq!(request.variables, json!({ "id": "link-1" }));

    let request = build_request(&project_links("p-1"));
    assert!(request.query.contains("initiativeToProjects"));
    assert_eq!(request.variables, json!({ "id": "p-1" }));
}

#[test]
fn a_status_update_carries_the_initiative_health_and_body() {
    let request = build_request(&initiative_update_create(
        InitiativeStatusUpdateCreateInput {
            initiative_id: "i-1".into(),
            health: InitiativeUpdateHealthType::AtRisk,
            body: "Waiting.".into(),
        },
    ));
    assert!(request.query.contains("initiativeUpdateCreate"));
    assert_eq!(
        request.variables,
        json!({ "input": { "initiativeId": "i-1", "health": "atRisk", "body": "Waiting." } })
    );
}

#[test]
fn the_status_updates_query_asks_for_one_initiative() {
    let request = build_request(&initiative_status_updates("i-1"));
    assert!(request.query.contains("initiativeUpdates"));
    assert_eq!(request.variables, json!({ "id": "i-1" }));
}
