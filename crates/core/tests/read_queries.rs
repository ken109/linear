//! The read queries against anonymized responses.

use chrono::{TimeZone, Utc};
use linear_core::read::{self, IssueList, IssueView};
use linear_core::wire::{build_request, parse_response, ResponseMeta};
use serde_json::Value;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    parse_response(&meta, &fixture(name), now).unwrap()
}

#[test]
fn an_issue_list_decodes_into_pages_of_issues() {
    let list: IssueList = parse("issue_list");
    assert_eq!(list.issues.nodes.len(), 1);
    assert_eq!(list.issues.nodes[0].identifier, "EX-23");
    assert!(!list.issues.page_info.has_next_page);
}

#[test]
fn an_issue_view_selects_the_issue_twice_under_two_names() {
    let req = build_request(&read::issue_view("EX-23"));
    let q = &req.query;
    assert!(q.contains("detail: issue(id: $id)"), "{q}");
    assert!(q.contains("issue(id: $id)"), "{q}");
    assert_eq!(req.variables["id"], "EX-23");

    let view: IssueView = parse("issue_view");
    assert_eq!(view.issue.identifier, "EX-23");
    assert_eq!(
        view.issue.description.as_deref(),
        Some("Body of the fixture issue.")
    );
    assert_eq!(view.detail.priority_label, "High");
    assert_eq!(view.detail.comments.len(), 1);
    assert_eq!(view.detail.comments[0].body, "A fixture comment.");
    // Both ends of the relations, as Linear holds them.
    assert_eq!(view.detail.relations.len(), 1);
    assert_eq!(view.detail.relations[0].type_, "blocks");
    assert_eq!(view.detail.relations[0].related_issue.identifier, "EX-24");
    assert_eq!(view.detail.inverse_relations.len(), 1);
    assert_eq!(view.detail.inverse_relations[0].issue.identifier, "EX-22");
}

#[test]
fn the_issue_list_request_selects_the_page_and_the_filter() {
    let req = build_request(&read::issue_list(read::IssueListVars::new(
        linear_core::types::PageVars {
            first: 50,
            after: Some("c1".into()),
        },
        None,
    )));
    let v: Value = serde_json::from_str(&req.to_json()).unwrap();
    assert_eq!(v["operationName"], "IssueList");
    assert_eq!(v["variables"]["first"], 50);
    assert_eq!(v["variables"]["after"], "c1");
    assert_eq!(v["variables"]["filter"], Value::Null);
}
