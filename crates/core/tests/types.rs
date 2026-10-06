//! Fragment parsing against anonymized real responses (see `tests/fixtures`).

use chrono::NaiveDate;
use cynic::GraphQlResponse;
use linear_core::inputs::*;
use linear_core::queries::*;
use linear_core::types::*;
use linear_core::InWorkspace;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let r: GraphQlResponse<T> = serde_json::from_str(&fixture(name)).unwrap();
    assert!(r.errors.is_none(), "{:?}", r.errors);
    r.data.expect("data")
}

#[test]
fn whoami_parses() {
    let d: Whoami = parse("whoami");
    assert_eq!(d.viewer.name, "Alice Example");
    assert!(d.viewer.is_me);
    assert_eq!(d.organization.url_key, "example");
}

#[test]
fn issue_parses_with_all_relations() {
    let d: IssueById = parse("issue");
    let i = d.issue;
    assert_eq!(i.identifier, "EX-23");
    assert_eq!(i.team.key, "EX");
    assert_eq!(i.state.name, "In Progress");
    assert_eq!(i.state.state_type(), StateType::Started);
    assert_eq!(i.assignee.as_ref().unwrap().display_name, "alice");
    assert_eq!(i.project.as_ref().unwrap().name, "Fixture Project");
    assert_eq!(i.project_milestone.as_ref().unwrap().name, "Milestone 1");
    assert_eq!(i.due_date, NaiveDate::from_ymd_opt(2026, 11, 1));
    assert_eq!(i.estimate, Some(3.0));
    assert!(i.started_at.is_some());
    assert!(i.completed_at.is_none());
    assert_eq!(i.source_url(), Some("https://example.com/source/1"));

    // Labels come through grouped: "api" belongs to the "area" group.
    let api = i.labels.iter().find(|l| l.name == "api").unwrap();
    assert_eq!(api.parent.as_ref().unwrap().name, "area");
    let bug = i.labels.iter().find(|l| l.name == "Bug").unwrap();
    assert!(bug.parent.is_none());
}

#[test]
fn assigned_issues_page_parses() {
    let d: AssignedStartedIssues = parse("assigned_issues");
    let page = d.viewer.assigned_issues;
    assert!(!page.nodes.is_empty());
    assert!(!page.page_info.has_next_page);
    assert!(page
        .nodes
        .iter()
        .all(|i| i.state.state_type() == StateType::Started));
}

#[test]
fn projects_parse_with_status_update_and_counts() {
    let d: Projects = parse("projects");
    let p = &d.projects.nodes[0];
    assert_eq!(p.status.type_, ProjectStatusType::Started);
    assert_eq!(p.lead.as_ref().unwrap().name, "Alice Example");
    assert_eq!(p.project_milestones.len(), 1);
    assert_eq!(p.project_milestones[0].status, ProjectMilestoneStatus::Next);
    let u = p.last_update.as_ref().unwrap();
    assert_eq!(u.health, ProjectUpdateHealthType::OnTrack);
    assert_eq!(u.body, "On track.");

    let c = p.issue_counts();
    assert!(c.complete);
    assert_eq!(c.total(), 3 - 1 /* the child issue has no project */);
    assert_eq!(c.started, 1);
    assert_eq!(c.completed, 1);
}

#[test]
fn comments_parse() {
    let d: IssueCommentsQuery = parse("issue_comments");
    let c = &d.issue.comments[0];
    assert_eq!(c.body, "A fixture comment.");
    assert_eq!(c.issue.as_ref().unwrap().identifier, "EX-23");
}

#[test]
fn template_data_is_decoded_from_its_string_form() {
    let d: Templates = parse("templates");
    let t = &d.templates[0];
    assert_eq!(t.type_, "issue");
    // Linear sends the JSON scalar as an encoded string.
    assert!(t.template_data.is_string());
    let data = t.data().unwrap();
    assert_eq!(data["descriptionData"]["type"], "doc");
}

#[test]
fn initiative_status_falls_back_for_unknown_values() {
    let d: Initiatives = parse("initiatives");
    let n = &d.initiatives.nodes;
    assert_eq!(n[0].status, InitiativeStatus::Active);
    assert_eq!(n[1].status, InitiativeStatus::Other("Paused".into()));
    // The fallback value survives serialization.
    let v = serde_json::to_value(&n[1]).unwrap();
    assert_eq!(v["status"], "Paused");
}

#[test]
fn unknown_state_type_is_not_an_error() {
    assert_eq!(StateType::parse("duplicate"), StateType::Duplicate);
    assert_eq!(
        StateType::parse("whatever"),
        StateType::Other("whatever".into())
    );
    assert!(StateType::Duplicate.is_closed());
    assert!(!StateType::Started.is_closed());
}

#[test]
fn domain_types_serialize_as_camel_case() {
    let d: IssueById = parse("issue");
    let v = serde_json::to_value(&d.issue).unwrap();
    assert!(v.get("dueDate").is_some());
    assert!(v.get("projectMilestone").is_some());
    assert_eq!(v["state"]["type"], "started");
    assert_eq!(v["labels"]["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn domain_types_round_trip_through_json() {
    // The cache stores what `--json` prints, so a serialized value must parse back.
    let d: IssueById = parse("issue");
    let json = serde_json::to_string(&d.issue).unwrap();
    let back: Issue = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d.issue);

    let d: Projects = parse("projects");
    let json = serde_json::to_string(&d.projects.nodes[0]).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d.projects.nodes[0]);
}

#[test]
fn in_workspace_flattens_and_round_trips() {
    let d: IssueById = parse("issue");
    let tagged = InWorkspace::new("ken109", d.issue.clone());
    let v = serde_json::to_value(&tagged).unwrap();
    assert_eq!(v["workspace"], "ken109");
    assert_eq!(v["identifier"], "EX-23");

    let back: InWorkspace<Issue> = serde_json::from_value(v).unwrap();
    assert_eq!(back, tagged);
}

#[test]
fn empty_update_input_serializes_as_an_empty_object() {
    // A `None` must be omitted, never sent as `null` (which Linear reads as "clear").
    let v = serde_json::to_value(IssueUpdateInput::default()).unwrap();
    assert_eq!(v, serde_json::json!({}));

    let op = issue_update("EX-1", IssueUpdateInput::default());
    let body = serde_json::to_value(&op).unwrap();
    assert_eq!(body["variables"]["input"], serde_json::json!({}));

    let v = serde_json::to_value(IssueUpdateInput {
        state_id: Some("s".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(v, serde_json::json!({ "stateId": "s" }));
}

#[test]
fn issue_update_response_parses() {
    let d: IssueUpdate = parse("issue_update");
    assert!(d.issue_update.success);
    assert_eq!(d.issue_update.issue.unwrap().identifier, "EX-23");
}
