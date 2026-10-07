//! `attachmentLinkGitHubPR` and the check for the GitHub integration it needs.

use linear_core::inputs::attachment_link_github_pr;
use linear_core::queries::{self, Integrations};
use serde_json::{json, Value};

fn integrations(nodes: Value) -> Integrations {
    let body = json!({"data": {"integrations": {"nodes": nodes}}});
    let response: cynic::GraphQlResponse<Integrations> = serde_json::from_value(body).unwrap();
    response.data.unwrap()
}

#[test]
fn the_mutation_names_the_issue_and_the_pull_request() {
    let op = attachment_link_github_pr("KK-12", "https://github.com/o/r/pull/3");
    let body: Value = serde_json::to_value(&op).unwrap();
    let query = body["query"].as_str().unwrap();
    assert!(
        query.contains("attachmentLinkGitHubPR(issueId: $issueId, url: $url)"),
        "{query}"
    );
    assert!(
        query.contains("sourceType") && query.contains("metadata"),
        "{query}"
    );
    assert_eq!(
        body["variables"],
        json!({"issueId": "KK-12", "url": "https://github.com/o/r/pull/3"})
    );
}

#[test]
fn the_integrations_query_selects_what_it_needs() {
    let body = serde_json::to_value(queries::integrations()).unwrap();
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("integrations(first: 100)"), "{query}");
    assert!(
        query.contains("service") && query.contains("archivedAt"),
        "{query}"
    );
}

#[test]
fn the_real_integrations_of_a_workspace_with_github_are_recognised() {
    let text = include_str!("fixtures/integrations.json");
    let response: cynic::GraphQlResponse<Integrations> = serde_json::from_str(text).unwrap();
    assert!(response.data.unwrap().has_github());
}

#[test]
fn github_must_be_the_installed_app_and_not_archived() {
    assert!(!integrations(json!([])).has_github());
    // A personal connection and a commit-link connection do not make a pull request attachment.
    assert!(!integrations(json!([
        {"service": "githubPersonal", "archivedAt": null},
        {"service": "githubCommit", "archivedAt": null},
        {"service": "slack", "archivedAt": null},
    ]))
    .has_github());
    assert!(!integrations(json!([
        {"service": "github", "archivedAt": "2026-01-01T00:00:00.000Z"},
    ]))
    .has_github());
    assert!(integrations(json!([
        {"service": "github", "archivedAt": "2026-01-01T00:00:00.000Z"},
        {"service": "github", "archivedAt": null},
    ]))
    .has_github());
    assert!(integrations(json!([
        {"service": "githubEnterpriseServer", "archivedAt": null},
    ]))
    .has_github());
}
