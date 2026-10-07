//! The GitHub integration: `issue link-pr`, and what `issue view` shows of the
//! branch name and the pull requests. Against a mock Linear: the sandbox has no
//! GitHub integration, so none of this can run live there.
//!
//! The attachments are those of `core/tests/fixtures/attachments_github.json`
//! (0 a synced GitHub issue, 1 open #41, 2 draft #42, 3 merged #43, ...).

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const PR: &str = "https://github.com/example/app/pull/41";

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn with_attachments(mut v: View, nodes: Vec<Value>) -> View {
    v.0["issue"]["attachments"] = json!({ "nodes": nodes });
    v
}

fn routes(view: View, extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes = vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("Integrations", vec![integrations_of(&["github", "slack"])]),
        (
            "AttachmentLinkGitHubPr",
            vec![link_ok(github_attachment(1))],
        ),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

// ------------------------------------------------------------------ link-pr

#[test]
fn link_pr_links_the_pull_request_and_reports_its_state() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));

    // The URL is sent in its canonical form, whatever the page the person copied.
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "link-pr",
            "EX-23",
            "https://github.com/example/app/pull/41/files?w=1",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(v["url"], PR);
    assert_eq!(v["alreadyLinked"], false);
    assert_eq!(v["attachment"]["sourceType"], "github");
    assert_eq!(v["pullRequest"]["number"], 41);
    assert_eq!(v["pullRequest"]["status"], "open");
    assert_eq!(
        mock.of("AttachmentLinkGitHubPr"),
        vec![json!({ "issueId": "id-EX-23", "url": PR })]
    );
    // Asked about the integration first.
    let ops = mock.ops();
    let at = |name: &str| ops.iter().position(|o| o == name).unwrap();
    assert!(at("Integrations") < at("AttachmentLinkGitHubPr"), "{ops:?}");
}

#[test]
fn link_pr_prints_one_line_and_quiet_prints_the_url() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o).trim(),
        format!("EX-23  #41  {PR}  (linked, open)")
    );

    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR, "--quiet"]);
    assert_eq!(stdout(&o).trim(), PR);
}

#[test]
fn a_workspace_without_the_github_integration_is_an_error_and_nothing_is_sent() {
    let sb = workspace_with_rules(&[]);
    // A personal connection is not the integration the link needs.
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "Integrations",
            vec![integrations_of(&["githubPersonal", "slack"])],
        )],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("no GitHub integration"),
        "{}",
        stderr(&o)
    );
    assert!(mock.of("AttachmentLinkGitHubPr").is_empty());
    mock.assert_read_only();
}

#[test]
fn an_integration_that_cannot_be_listed_leaves_the_decision_to_linear() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        vec![("Integrations", vec![graphql_error("not allowed")])],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentLinkGitHubPr").len(), 1);
}

#[test]
fn linear_refusing_the_link_is_an_error() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentLinkGitHubPr",
            vec![graphql_error("GitHub integration not found")],
        )],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("GitHub integration not found"),
        "{}",
        stderr(&o)
    );

    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentLinkGitHubPr",
            vec![data(json!({ "attachmentLinkGitHubPR": {
                "success": false, "attachment": github_attachment(1)
            }}))],
        )],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("could not link"), "{}", stderr(&o));
}

#[test]
fn an_attachment_that_is_not_a_pull_request_one_is_warned_about() {
    let sb = workspace_with_rules(&[]);
    // sourceType oauthClient: a plain link, which will not follow GitHub.
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentLinkGitHubPr",
            vec![link_ok(github_attachment(6))],
        )],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("not a GitHub pull request"),
        "{}",
        stderr(&o)
    );
    assert_eq!(stdout_json(&o)["pullRequest"], Value::Null);
}

#[test]
fn a_pull_request_the_issue_already_has_is_not_linked_again() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        with_attachments(mine(), vec![github_attachment(0), github_attachment(1)]),
        vec![],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["alreadyLinked"], true);
    assert_eq!(v["pullRequest"]["status"], "open");
    assert!(mock.of("AttachmentLinkGitHubPr").is_empty());
    assert!(mock.of("Integrations").is_empty());

    // Another pull request on the same issue still goes through.
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "link-pr",
            "EX-23",
            "https://github.com/example/app/pull/99",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentLinkGitHubPr").len(), 1);
}

#[test]
fn something_that_is_not_a_pull_request_url_is_a_usage_error_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    for bad in [
        "https://github.com/example/app/issues/41",
        "https://github.com/example/app",
        "example/app#41",
        "ftp://github.com/example/app/pull/41",
    ] {
        let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", bad]);
        assert_eq!(code(&o), 2, "{bad}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty());
}

#[test]
fn link_pr_follows_ownership_like_any_change_to_the_issue() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
        vec![],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ view

fn view_reply(attachments: Vec<Value>) -> String {
    let mut v: Value = serde_json::from_str(&fixture("issue_view")).unwrap();
    v["data"]["issue"]["attachments"] = json!({ "nodes": attachments });
    v.to_string()
}

#[test]
fn view_json_has_the_branch_name_and_the_pull_requests() {
    let sb = workspace();
    let attachments = vec![
        github_attachment(0),
        github_attachment(1),
        github_attachment(3),
        github_attachment(6),
    ];
    let mock = Mock::start(vec![ok(&view_reply(attachments))]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-23", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["branchName"], "branch-ex-23");
    // The raw integration data is there ...
    assert_eq!(v["attachments"]["nodes"][1]["sourceType"], "github");
    assert_eq!(v["attachments"]["nodes"][1]["metadata"]["status"], "open");
    // ... and so is the reading of it: the synced issue and the plain link are not pull requests.
    let prs = v["pullRequests"].as_array().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0]["number"], 41);
    assert_eq!(prs[0]["status"], "open");
    assert_eq!(prs[0]["openedAt"], "2026-10-02T09:00:00Z");
    assert_eq!(prs[0]["mergedAt"], Value::Null);
    assert_eq!(prs[1]["number"], 43);
    assert_eq!(prs[1]["status"], "merged");
    assert_eq!(prs[1]["mergedAt"], "2026-10-05T09:00:00Z");
}

#[test]
fn view_json_has_an_empty_list_without_pull_requests() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_view"))]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-23", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["pullRequests"], json!([]));
}

#[test]
fn view_text_shows_the_branch_and_lists_the_pull_requests() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&view_reply(vec![github_attachment(3)]))]);
    let o = linear(&sb, &mock, &["issue", "view", "EX-23"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    assert!(
        text.contains("Branch") && text.contains("branch-ex-23"),
        "{text}"
    );
    assert!(text.contains("Pull requests"), "{text}");
    assert!(text.contains("#43  merged  Fix the widget"), "{text}");
    assert!(
        text.contains("https://github.com/example/app/pull/43"),
        "{text}"
    );
}

#[test]
fn a_dry_run_of_link_pr_plans_the_link_and_still_asks_for_the_integration() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &["issue", "link-pr", "EX-23", PR, "--dry-run", "--json"],
        ),
        &mock,
    );
    assert_eq!(v["command"], "issue link-pr");
    assert_eq!(planned(&v), ["AttachmentLinkGitHubPr"]);
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "issueId": "id-EX-23", "url": PR })
    );

    // No GitHub integration: the same error as the real run (exit 1), nothing planned.
    let mock = Routed::start(routes(
        mine(),
        vec![("Integrations", vec![integrations_of(&["slack"])])],
    ));
    let o = run(&sb, &mock, &["issue", "link-pr", "EX-23", PR, "--dry-run"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    mock.assert_read_only();

    // Already linked: nothing to send.
    let mock = Routed::start(routes(
        with_attachments(mine(), vec![github_attachment(0), github_attachment(1)]),
        vec![],
    ));
    let v = plan(
        &run(
            &sb,
            &mock,
            &["issue", "link-pr", "EX-23", PR, "--dry-run", "--json"],
        ),
        &mock,
    );
    assert_eq!(v["mutations"], json!([]));
}
