//! What crosses the WebAssembly boundary: query results serialize in a form
//! that parses back to the same value, and the JSON Schema of the enums that
//! carry a fallback lists values that really are known.

use cynic::GraphQlResponse;
use linear_core::queries::*;
use linear_core::read::*;
use linear_core::types::*;
use schemars::{schema_for, JsonSchema};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt::Debug;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Parse a fixture, serialize it, parse that again: nothing may be lost or renamed.
fn round_trips<T: DeserializeOwned + Serialize + PartialEq + Debug>(name: &str) {
    let r: GraphQlResponse<T> = serde_json::from_str(&fixture(name)).unwrap();
    let first = r.data.expect("data");
    let json = serde_json::to_value(&first).unwrap();
    let second: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|e| panic!("{name}: the serialized form does not parse: {e}\n{json:#}"));
    assert_eq!(first, second, "{name}");
}

#[test]
fn every_query_result_round_trips() {
    round_trips::<Whoami>("whoami");
    round_trips::<IssueById>("issue");
    round_trips::<AssignedStartedIssues>("assigned_issues");
    round_trips::<Projects>("projects");
    round_trips::<IssueCommentsQuery>("issue_comments");
    round_trips::<Templates>("templates");
    round_trips::<Templates>("templates_sections");
    round_trips::<Initiatives>("initiatives");
    round_trips::<IssueView>("issue_view");
    round_trips::<ProjectView>("project_view");
    round_trips::<MilestonesOfProject>("milestones");
    round_trips::<MilestoneView>("milestone_view");
    round_trips::<InitiativeView>("initiative_view");
    round_trips::<Labels>("labels");
    round_trips::<Teams>("teams");
    round_trips::<Users>("users");
}

#[test]
fn a_query_result_serializes_in_the_shape_of_the_response() {
    let r: GraphQlResponse<AssignedStartedIssues> =
        serde_json::from_str(&fixture("assigned_issues")).unwrap();
    let out = serde_json::to_value(r.data.unwrap()).unwrap();
    let issue = &out["viewer"]["assignedIssues"]["nodes"][0];
    assert!(issue["projectMilestone"].is_object() || issue["projectMilestone"].is_null());
    assert!(issue["createdAt"].is_string());
    assert!(out["viewer"]["assignedIssues"]["pageInfo"]["hasNextPage"].is_boolean());
}

/// The values a schema lists as known, if it is an open enum.
fn known_values<T: JsonSchema>() -> Vec<String> {
    let schema = serde_json::to_value(schema_for!(T)).unwrap();
    schema["x-known-values"]
        .as_array()
        .unwrap_or_else(|| panic!("{}: no x-known-values in {schema}", T::schema_name()))
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

/// Every listed value parses to a named variant, not the fallback; and the
/// schema says plainly that any other string is allowed.
fn known_values_are_known<T: JsonSchema + DeserializeOwned + Serialize + Debug + PartialEq>(
    is_fallback: impl Fn(&T) -> bool,
) {
    let values = known_values::<T>();
    assert!(!values.is_empty());
    for v in &values {
        let parsed: T = serde_json::from_value(json!(v)).unwrap();
        assert!(
            !is_fallback(&parsed),
            "{}: {v:?} parses as the fallback",
            T::schema_name()
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), json!(v));
    }
    let unknown: T = serde_json::from_value(json!("somethingLinearAddsLater")).unwrap();
    assert!(is_fallback(&unknown));
    let schema = serde_json::to_value(schema_for!(T)).unwrap();
    assert_eq!(schema["type"], "string", "{}", T::schema_name());
}

#[test]
fn open_enum_schemas_list_the_values_the_enums_know() {
    known_values_are_known::<ProjectStatusType>(|v| matches!(v, ProjectStatusType::Other(_)));
    known_values_are_known::<ProjectUpdateHealthType>(|v| {
        matches!(v, ProjectUpdateHealthType::Other(_))
    });
    known_values_are_known::<ProjectMilestoneStatus>(|v| {
        matches!(v, ProjectMilestoneStatus::Other(_))
    });
    known_values_are_known::<InitiativeStatus>(|v| matches!(v, InitiativeStatus::Other(_)));
    known_values_are_known::<LabelGroupType>(|v| matches!(v, LabelGroupType::Other(_)));
}

#[test]
fn the_state_type_schema_lists_exactly_what_state_type_parses() {
    let values = known_values::<StateType>();
    for v in &values {
        assert!(
            !matches!(StateType::parse(v), StateType::Other(_)),
            "{v:?} is not a known state type"
        );
    }
    // And nothing the parser knows is missing from the list.
    for known in [
        "triage",
        "backlog",
        "unstarted",
        "started",
        "completed",
        "canceled",
        "duplicate",
    ] {
        assert!(values.iter().any(|v| v == known), "{known} missing");
    }
    // A workflow state's `type` is documented with those values.
    let schema: Value = serde_json::to_value(schema_for!(WorkflowState)).unwrap();
    assert_eq!(
        schema["properties"]["type"]["$ref"], "#/$defs/StateType",
        "{schema}"
    );
}
