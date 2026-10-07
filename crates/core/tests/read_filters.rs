//! Filters built from what a person types, as they appear on the wire.

use linear_core::filters::{
    closed_since, priority_number, InitiativeQuery, IssueQuery, Pick, ProjectQuery,
};
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

#[test]
fn the_audit_filter_is_open_issues_or_recently_updated_ones() {
    let since = "2026-10-06T00:00:00Z".parse().unwrap();
    let req = build_request(&read::issue_list(IssueListVars::new(
        page(),
        Some(linear_core::filters::audit_issues(since)),
    )));
    let v: Value = serde_json::from_str(&req.to_json()).unwrap();
    assert_eq!(
        v["variables"]["filter"],
        json!({"or": [
            {"state": {"type": {"nin": ["completed", "canceled"]}}},
            {"updatedAt": {"gte": "2026-10-06T00:00:00Z"}},
        ]})
    );
}

// ---------------------------------------------------------------- closed since

fn since() -> chrono::DateTime<chrono::Utc> {
    "2026-09-23T12:00:00Z".parse().unwrap()
}

fn utc(s: &str) -> chrono::DateTime<chrono::Utc> {
    s.parse().unwrap()
}

#[test]
fn closed_since_alone_is_completed_or_canceled_at_or_after_the_time() {
    let q = IssueQuery {
        closed_since: Some(since()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"and": [{"or": [
            {"completedAt": {"gte": "2026-09-23T12:00:00Z"}},
            {"canceledAt": {"gte": "2026-09-23T12:00:00Z"}},
        ]}]})
    );
}

#[test]
fn open_with_closed_since_is_the_open_issues_plus_the_recently_closed_ones() {
    let q = IssueQuery {
        open: true,
        closed_since: Some(since()),
        ..IssueQuery::default()
    };
    // One list of alternatives; the open clause is not also ANDed in as a plain state filter.
    assert_eq!(
        issue_filter_json(&q),
        json!({"and": [{"or": [
            {"state": {"type": {"nin": ["completed", "canceled"]}}},
            {"completedAt": {"gte": "2026-09-23T12:00:00Z"}},
            {"canceledAt": {"gte": "2026-09-23T12:00:00Z"}},
        ]}]})
    );
}

#[test]
fn closed_since_narrows_like_any_other_filter_and_keeps_label_and_state_alternatives() {
    let q = IssueQuery {
        state_types: vec!["completed".into()],
        labels: vec!["a".into(), "b".into()],
        state_names: vec!["Done".into()],
        closed_since: Some(since()),
        ..IssueQuery::default()
    };
    let f = issue_filter_json(&q);
    assert_eq!(f["state"], json!({"type": {"in": ["completed"]}}));
    assert_eq!(
        f["or"],
        json!([{"state": {"name": {"eqIgnoreCase": "Done"}}}])
    );
    // The second label and the closed-since alternatives are all ANDed.
    assert_eq!(f["and"].as_array().unwrap().len(), 2, "{f}");
    assert_eq!(f["and"][0]["labels"]["some"]["name"]["eqIgnoreCase"], "b");
    assert!(f["and"][1]["or"][0]["completedAt"].is_object(), "{f}");
}

#[test]
fn closed_since_reads_days_and_dates() {
    let now = utc("2026-10-07T03:30:00Z");
    assert_eq!(closed_since("14d", now), Ok(utc("2026-09-23T03:30:00Z")));
    assert_eq!(closed_since(" 1D ", now), Ok(utc("2026-10-06T03:30:00Z")));
    assert_eq!(
        closed_since("2026-10-01", now),
        Ok(utc("2026-10-01T00:00:00Z"))
    );
}

#[test]
fn closed_since_refuses_what_is_not_a_day_count_or_a_real_date() {
    let now = utc("2026-10-07T03:30:00Z");
    for bad in [
        "",
        "d",
        "0d",
        "-3d",
        "1.5d",
        "14",
        "14days",
        "99999999d",
        "2026-02-30",
        "2026-13-01",
        "yesterday",
        "2026-10-01T00:00:00Z",
    ] {
        let e = closed_since(bad, now).expect_err(bad);
        assert!(e.contains("expected"), "{bad:?}: {e}");
    }
}

#[test]
fn priorities_are_alternatives_by_number() {
    let q = IssueQuery {
        priorities: vec![1, 2],
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"priority": {"in": [1.0, 2.0]}})
    );
}

#[test]
fn priority_names_and_numbers_both_read() {
    for (spec, number) in [
        ("0", 0),
        ("none", 0),
        ("No-Priority", 0),
        ("1", 1),
        ("Urgent", 1),
        ("high", 2),
        ("2", 2),
        (" medium ", 3),
        ("low", 4),
        ("4", 4),
    ] {
        assert_eq!(priority_number(spec), Ok(number), "{spec}");
    }
    for bad in ["5", "-1", "", "critical", "1.5"] {
        let e = priority_number(bad).unwrap_err();
        assert!(e.contains("expected 0-4"), "{bad}: {e}");
    }
}

#[test]
fn a_parent_is_one_issue_or_none() {
    let q = IssueQuery {
        parent: Some(Pick::Is("issue-id".into())),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"parent": {"id": {"eq": "issue-id"}}})
    );
    let q = IssueQuery {
        parent: Some(Pick::Nothing),
        ..IssueQuery::default()
    };
    assert_eq!(issue_filter_json(&q), json!({"parent": {"null": true}}));
}

#[test]
fn a_cycle_number_is_matched_within_the_team_and_none_means_no_cycle() {
    let q = IssueQuery {
        cycle: Some(Pick::Is(42)),
        team_key: Some("EX".into()),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({
            "team": {"key": {"eqIgnoreCase": "EX"}},
            "cycle": {"number": {"eq": 42.0}, "team": {"key": {"eqIgnoreCase": "EX"}}},
        })
    );
    let q = IssueQuery {
        cycle: Some(Pick::Nothing),
        ..IssueQuery::default()
    };
    assert_eq!(issue_filter_json(&q), json!({"cycle": {"null": true}}));
}

#[test]
fn updated_after_is_a_lower_bound_on_the_update_time() {
    let q = IssueQuery {
        updated_after: Some(utc("2026-10-01T00:00:00Z")),
        ..IssueQuery::default()
    };
    assert_eq!(
        issue_filter_json(&q),
        json!({"updatedAt": {"gte": "2026-10-01T00:00:00Z"}})
    );
}

#[test]
fn the_new_filters_narrow_together_with_the_old_ones() {
    let q = IssueQuery {
        assignee: Some("me".into()),
        priorities: vec![2],
        parent: Some(Pick::Nothing),
        open: true,
        ..IssueQuery::default()
    };
    let f = issue_filter_json(&q);
    assert_eq!(f["assignee"], json!({"isMe": {"eq": true}}));
    assert_eq!(f["priority"], json!({"in": [2.0]}));
    assert_eq!(f["parent"], json!({"null": true}));
    assert_eq!(
        f["state"],
        json!({"type": {"nin": ["completed", "canceled"]}})
    );
}

#[test]
fn a_search_sends_the_term_and_the_filter_and_asks_for_comments_only_on_request() {
    let q = IssueQuery {
        team_key: Some("EX".into()),
        ..IssueQuery::default()
    };
    let req = build_request(&read::issue_search(read::IssueSearchVars::new(
        page(),
        "duplicate check",
        q.filter(),
        false,
    )));
    assert!(req.query.contains("searchIssues"), "{}", req.query);
    let v = serde_json::from_str::<Value>(&req.to_json()).unwrap()["variables"].clone();
    assert_eq!(v["term"], "duplicate check");
    assert_eq!(
        v["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
    assert_eq!(v["includeComments"], Value::Null);

    let req = build_request(&read::issue_search(read::IssueSearchVars::new(
        page(),
        "x",
        None,
        true,
    )));
    assert_eq!(req.variables["includeComments"], true);
    assert_eq!(req.variables["filter"], Value::Null);
}
