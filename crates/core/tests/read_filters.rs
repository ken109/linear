//! Filters built from what a person types, as they appear on the wire.

use linear_core::filters::{InitiativeQuery, IssueQuery, ProjectQuery};
use linear_core::read::{self, InitiativeListVars, IssueListVars, ProjectListVars};
use linear_core::types::PageVars;
use linear_core::wire::build_request;
use serde_json::{json, Value};

fn page() -> PageVars {
    PageVars {
        first: 50,
        after: None,
    }
}

fn issue_filter_json(q: &IssueQuery) -> Value {
    let req = build_request(&read::issue_list(IssueListVars::new(page(), q.filter())));
    serde_json::from_str::<Value>(&req.to_json()).unwrap()["variables"]["filter"].clone()
}

#[test]
fn an_empty_query_sends_a_null_filter() {
    assert!(IssueQuery::default().filter().is_none());
    assert_eq!(issue_filter_json(&IssueQuery::default()), Value::Null);
    assert!(ProjectQuery::default().filter().is_none());
    assert!(InitiativeQuery::default().filter().is_none());
}

#[test]
fn me_none_email_and_name_become_different_user_filters() {
    let q = IssueQuery {
        assignee: Some("me".into()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"assignee": {"isMe": {"eq": true}}})
    );
    let q = IssueQuery {
        assignee: Some("none".into()),
        ..IssueQuery::default()
    };
    assert_eq!(issue_filter_json(&q), json!({"assignee": {"null": true}}));
    let q = IssueQuery {
        assignee: Some("alice@example.com".into()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"assignee": {"email": {"eq": "alice@example.com"}}})
    );
    let q = IssueQuery {
        assignee: Some("Alice".into()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"assignee": {"or": [
            {"name": {"eqIgnoreCase": "Alice"}},
            {"displayName": {"eqIgnoreCase": "Alice"}}
        ]}})
    );
}

#[test]
fn state_types_and_open_narrow_by_type() {
    let q = IssueQuery {
        state_types: vec!["started".into(), "unstarted".into()],
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"state": {"type": {"in": ["started", "unstarted"]}}})
    );

    let q = IssueQuery {
        open: true,
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"state": {"type": {"nin": ["completed", "canceled"]}}})
    );

    // An explicit type list wins over `open`.
    let q = IssueQuery {
        open: true,
        state_types: vec!["started".into()],
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"state": {"type": {"in": ["started"]}}})
    );
}

#[test]
fn state_names_are_alternatives_and_labels_all_have_to_match() {
    let q = IssueQuery {
        state_names: vec!["In Progress".into(), "Backlog".into()],
        labels: vec!["Bug".into(), "api".into()],
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({
            "labels": {"some": {"name": {"eqIgnoreCase": "Bug"}}},
            "and": [{"labels": {"some": {"name": {"eqIgnoreCase": "api"}}}}],
            "or": [
                {"state": {"name": {"eqIgnoreCase": "In Progress"}}},
                {"state": {"name": {"eqIgnoreCase": "Backlog"}}}
            ]
        })
    );
}

#[test]
fn project_milestone_team_and_source_url_filters() {
    let q = IssueQuery {
        team_key: Some("kk".into()),
        project_id: Some("p-1".into()),
        milestone: Some("M1".into()),
        source_url: Some("https://example.com/a".into()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({
            "team": {"key": {"eqIgnoreCase": "kk"}},
            "project": {"id": {"eq": "p-1"}},
            "projectMilestone": {"name": {"eqIgnoreCase": "M1"}},
            "attachments": {"some": {"url": {"eq": "https://example.com/a"}}}
        })
    );
}

#[test]
fn project_filters() {
    let q = ProjectQuery {
        lead: Some("me".into()),
        open: true,
        initiative: Some("Human Sim".into()),
        ..ProjectQuery::default()
    };
    let req = build_request(&read::project_list(ProjectListVars::new(
        page(),
        q.filter(),
    )));
    let v: Value = serde_json::from_str(&req.to_json()).unwrap();
    assert_eq!(
        v["variables"]["filter"],
        json!({
            "status": {"type": {"nin": ["completed", "canceled"]}},
            "lead": {"isMe": {"eq": true}},
            "initiatives": {"some": {"name": {"eqIgnoreCase": "Human Sim"}}}
        })
    );
    assert_eq!(v["operationName"], "ProjectList");
}

#[test]
fn initiative_filters() {
    let q = InitiativeQuery {
        statuses: vec!["Active".into()],
        owner: Some("me".into()),
    };
    let req = build_request(&read::initiative_list(InitiativeListVars::new(
        page(),
        q.filter(),
    )));
    let v: Value = serde_json::from_str(&req.to_json()).unwrap();
    assert_eq!(
        v["variables"]["filter"],
        json!({"status": {"in": ["Active"]}, "owner": {"isMe": {"eq": true}}})
    );
}
