//! `linear audit` against a mock Linear server.
//!
//! The fixtures describe issue EX-23 (In Progress, assigned to the viewer) in
//! the project "Fixture Project" (led by the viewer). EX-23 started after the
//! project's latest status update, so the project's status update is outdated
//! whatever day the tests run on: that is the finding the tests rely on.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

/// What one audit asks for, in order: who am I, the issues, the projects.
fn replies() -> Vec<Reply> {
    vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(&fixture("projects")),
    ]
}

fn json_run(sb: &Sandbox, mock: &Mock, args: &[&str]) -> (i32, Value, String) {
    let mut full = vec!["audit", "--json"];
    full.extend_from_slice(args);
    let o = linear(sb, mock, &full);
    assert_no_leak(&o);
    (
        code(&o),
        serde_json::from_str(&stdout(&o)).unwrap_or(Value::Null),
        stderr(&o),
    )
}

fn rules(report: &Value) -> Vec<(String, String)> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().unwrap().to_owned(),
                f["target"]["identifier"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn has(report: &Value, rule: &str, identifier: &str) -> bool {
    rules(report).contains(&(rule.to_owned(), identifier.to_owned()))
}

fn operations(mock: &Mock) -> Vec<String> {
    (0..mock.requests().len())
        .map(|n| {
            request(mock, n)["operationName"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn set_config(sb: &Sandbox, text: &str) {
    std::fs::write(sb.config_dir().join("workspaces.toml"), text).unwrap();
}

/// The `issue_list` fixture with EX-23 changed by `edit`.
fn issues_with(edit: impl FnOnce(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    edit(&mut v["data"]["issues"]["nodes"][0]);
    v.to_string()
}

// ------------------------------------------------------------------ fetching

#[test]
fn it_audits_what_it_fetches_from_the_issue_side() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let (status, report, err) = json_run(&sb, &mock, &[]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(report["failedWorkspaces"], json!([]));
    assert_eq!(report["unresolvedIssues"], json!([]));
    assert!(
        has(&report, "status-update-outdated", "aaaaaaaaaaaa"),
        "{report}"
    );

    assert_eq!(operations(&mock), ["Whoami", "IssueList", "Projects"]);
    // Open issues, and anything updated lately (completed work counts for
    // the status-update check), not "the first 100 issues of each project".
    let filter = &request(&mock, 1)["variables"]["filter"];
    assert_eq!(
        filter["or"][0],
        json!({"state": {"type": {"nin": ["completed", "canceled"]}}})
    );
    let since = filter["or"][1]["updatedAt"]["gte"].as_str().unwrap();
    let since: chrono::DateTime<chrono::Utc> = since.parse().unwrap();
    // Status updates go stale after 14 days; one more is enough margin.
    let days = (chrono::Utc::now() - since).num_days();
    assert!((14..=16).contains(&days), "{days}");
}

#[test]
fn every_page_of_issues_and_projects_is_fetched() {
    let sb = workspace();
    let mut first: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    first["data"]["issues"]["pageInfo"] = json!({"hasNextPage": true, "endCursor": "c1"});
    let mut second: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    let node = &mut second["data"]["issues"]["nodes"][0];
    node["id"] = json!("00000000-0000-4000-8000-0000000000aa");
    node["identifier"] = json!("EX-24");
    node["dueDate"] = json!("2020-01-01");
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&first.to_string()),
        ok(&second.to_string()),
        ok(&fixture("projects")),
    ]);
    let (status, report, err) = json_run(&sb, &mock, &[]);
    assert_eq!(status, 0, "{err}");
    assert!(
        has(&report, "overdue", "EX-24"),
        "the second page was read: {report}"
    );
    assert_eq!(request(&mock, 2)["variables"]["after"], "c1");
}

// -------------------------------------------------------------------- output

#[test]
fn the_human_output_says_what_to_do_about_each_finding() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["audit"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    assert!(text.contains("status-update-outdated"), "{text}");
    assert!(text.contains("(actionable)"), "{text}");
    assert!(
        text.contains("fix: linear project status-update aaaaaaaaaaaa"),
        "{text}"
    );
    assert!(
        text.contains("https://linear.app/example/project/"),
        "{text}"
    );
    assert!(text.contains(" -w example"), "{text}");
    assert!(text.contains("findings,"), "{text}");
}

#[test]
fn quiet_prints_one_line_per_finding() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let o = linear(&sb, &mock, &["audit", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o)
            .lines()
            .any(|l| l == "example status-update-outdated aaaaaaaaaaaa"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn nothing_found_is_said_so() {
    let sb = workspace();
    // A project that is fine: no status update is due and its issue is not in progress.
    let mut projects: Value = serde_json::from_str(&fixture("projects")).unwrap();
    projects["data"]["projects"]["nodes"] = json!([]);
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&issues_with(|i| i["state"]["type"] = json!("backlog"))),
        ok(&projects.to_string()),
    ]);
    let o = linear(&sb, &mock, &["audit"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "No findings.");
}

// ----------------------------------------------------------------- exit codes

#[test]
fn findings_do_not_fail_the_command_unless_asked() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let (status, report, _) = json_run(&sb, &mock, &[]);
    assert_eq!(status, 0);
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["actionable"] == true));
}

#[test]
fn fail_on_actionable_exits_with_6_after_printing_the_findings() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let (status, report, err) = json_run(&sb, &mock, &["--fail-on", "actionable"]);
    assert_eq!(status, 6, "{err}");
    assert!(has(&report, "status-update-outdated", "aaaaaaaaaaaa"));
    assert!(err.contains("\"audit_findings\""), "{err}");
}

#[test]
fn fail_on_actionable_ignores_findings_that_are_only_informational() {
    let sb = workspace();
    // Somebody else leads the project and owns the issue: nothing to fix by me.
    let mut projects: Value = serde_json::from_str(&fixture("projects")).unwrap();
    let other = json!({
        "id": "00000000-0000-4000-8000-0000000000bb", "name": "Bob Example",
        "displayName": "bob", "email": "bob@example.com", "active": true, "isMe": false
    });
    projects["data"]["projects"]["nodes"][0]["lead"] = other.clone();
    let issues = issues_with(|i| i["assignee"] = other.clone());
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&issues),
        ok(&projects.to_string()),
    ]);
    let (status, report, err) = json_run(&sb, &mock, &["--fail-on", "actionable"]);
    assert_eq!(status, 0, "{err}");
    let findings = report["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    assert!(
        findings.iter().all(|f| f["actionable"] == false),
        "{report}"
    );
}

// -------------------------------------------------------- configuration

#[test]
fn the_thresholds_come_from_the_workspace_settings() {
    let sb = workspace();
    // EX-23 has been In Progress and untouched since 2020.
    let issues = issues_with(|i| i["updatedAt"] = json!("2020-01-01T00:00:00Z"));
    let base = "[workspaces.example]\nurl_key = \"example\"\n";

    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&issues),
        ok(&fixture("projects")),
    ]);
    let (_, report, _) = json_run(&sb, &mock, &[]);
    assert!(has(&report, "stale-in-progress", "EX-23"), "{report}");

    set_config(
        &sb,
        &format!(
            "{base}[workspaces.example.audit]\nstale_days = 100000\nstatus_update_days = 100000\n"
        ),
    );
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&issues),
        ok(&fixture("projects")),
    ]);
    let (_, report, err) = json_run(&sb, &mock, &[]);
    assert!(!has(&report, "stale-in-progress", "EX-23"), "{report}");
    assert!(err.is_empty(), "{err}");
    // The wider window for the status update check is asked of Linear, too.
    let since = request(&mock, 1)["variables"]["filter"]["or"][1]["updatedAt"]["gte"]
        .as_str()
        .unwrap()
        .to_owned();
    let since: chrono::DateTime<chrono::Utc> = since.parse().unwrap();
    assert!((chrono::Utc::now() - since).num_days() > 90_000);
}

#[test]
fn enabled_rules_are_applied_to_existing_issues_and_templates_are_fetched_for_them() {
    let sb = workspace();
    set_config(
        &sb,
        "[workspaces.example]\nurl_key = \"example\"\nrules = [\"template-sections\", \"source-attachment\"]\n",
    );
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(&fixture("projects")),
        ok(&fixture("templates_sections")),
    ]);
    let (status, report, err) = json_run(&sb, &mock, &[]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(
        operations(&mock),
        ["Whoami", "IssueList", "Projects", "Templates"]
    );
    // "Body of the fixture issue." is none of the templates' sections.
    assert!(has(&report, "template-sections", "EX-23"), "{report}");
    // EX-23 carries an https source attachment.
    assert!(!has(&report, "source-attachment", "EX-23"), "{report}");
}

#[test]
fn templates_are_not_fetched_when_the_rule_is_off() {
    let sb = workspace();
    let mock = Mock::start(replies());
    let _ = json_run(&sb, &mock, &[]);
    assert!(!operations(&mock).contains(&"Templates".to_owned()));
}

// --------------------------------------------------------------- narrowing

#[test]
fn issues_narrows_the_audit_to_them_and_their_projects() {
    let sb = workspace();
    let mut list: Value = serde_json::from_str(&fixture("issue_list")).unwrap();
    let mut other = list["data"]["issues"]["nodes"][0].clone();
    other["id"] = json!("00000000-0000-4000-8000-0000000000aa");
    other["identifier"] = json!("EX-24");
    other["dueDate"] = json!("2020-01-01");
    other["project"] = Value::Null;
    other["projectMilestone"] = Value::Null;
    list["data"]["issues"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(other);
    let body = list.to_string();

    // Both are overdue-or-not; without --issues EX-24 is reported.
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&body),
        ok(&fixture("projects")),
    ]);
    let (_, all, _) = json_run(&sb, &mock, &[]);
    assert!(has(&all, "overdue", "EX-24"));

    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&body),
        ok(&fixture("projects")),
    ]);
    let (status, narrowed, err) = json_run(&sb, &mock, &["--issues", "ex-23"]);
    assert_eq!(status, 0, "{err}");
    assert!(!has(&narrowed, "overdue", "EX-24"), "{narrowed}");
    assert!(
        has(&narrowed, "status-update-outdated", "aaaaaaaaaaaa"),
        "{narrowed}"
    );
    assert_eq!(narrowed["unresolvedIssues"], json!([]));
    // The issue was in the fetched set, so it was not asked for again.
    assert_eq!(operations(&mock), ["Whoami", "IssueList", "Projects"]);
}

#[test]
fn an_issue_that_does_not_exist_is_unresolved_not_clean() {
    let sb = workspace();
    let not_found = r#"{"errors":[{"message":"Entity not found: Issue - Could not find referenced Issue.","extensions":{"type":"invalid input","code":"INPUT_ERROR"}}]}"#;
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(not_found),
        ok(&fixture("projects")),
    ]);
    let (status, report, err) = json_run(&sb, &mock, &["--issues", "EX-23,EX-999"]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(report["unresolvedIssues"], json!(["EX-999"]));
    // The issue outside the fetched set was asked for by identifier.
    assert_eq!(
        operations(&mock),
        ["Whoami", "IssueList", "IssueById", "Projects"]
    );
    assert_eq!(request(&mock, 2)["variables"]["id"], "EX-999");

    // The human output says so as well, on stderr.
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(&fixture("issue_list")),
        ok(not_found),
        ok(&fixture("projects")),
    ]);
    let o = linear(&sb, &mock, &["audit", "--issues", "EX-999"]);
    assert!(stderr(&o).contains("EX-999"), "{}", stderr(&o));
}

#[test]
fn a_closed_issue_that_is_named_is_fetched_and_checked() {
    let sb = workspace();
    // The issue is not among the open or recently updated ones; Linear has it.
    let mut closed: Value = serde_json::from_str(&fixture("issue")).unwrap();
    let issue = &mut closed["data"]["issue"];
    issue["id"] = json!("00000000-0000-4000-8000-0000000000cc");
    issue["identifier"] = json!("EX-5");
    issue["state"] = json!({"id": "x", "name": "Done", "type": "completed"});
    issue["updatedAt"] = json!("2020-01-01T00:00:00Z");
    let empty_list =
        r#"{"data":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![
        ok(&fixture("whoami")),
        ok(empty_list),
        ok(&closed.to_string()),
        ok(&fixture("projects")),
    ]);
    let (status, report, err) = json_run(
        &sb,
        &mock,
        &["--issues", "EX-5", "--since", "2026-01-01T00:00:00Z"],
    );
    assert_eq!(status, 0, "{err}");
    assert_eq!(report["unresolvedIssues"], json!([]));
    assert!(has(&report, "not-updated-since", "EX-5"), "{report}");
}

#[test]
fn since_without_issues_is_a_usage_error_before_anything_is_fetched() {
    let sb = workspace();
    let mock = Mock::start(vec![]);
    let o = linear(&sb, &mock, &["audit", "--since", "2026-10-06T00:00:00Z"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--issues"), "{}", stderr(&o));
    assert!(mock.requests().is_empty());

    // A time that is not RFC 3339 is a usage error as well.
    let o = linear(
        &sb,
        &mock,
        &["audit", "--issues", "EX-1", "--since", "yesterday"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}

#[test]
fn since_reports_each_named_issue_not_updated_since_then() {
    let sb = workspace();
    // EX-23 was updated 2026-10-06; asking about a later time finds it stale.
    let mock = Mock::start(replies());
    let (status, report, err) = json_run(
        &sb,
        &mock,
        &[
            "--issues",
            "EX-23",
            "--since",
            "2999-01-01T00:00:00Z",
            "--fail-on",
            "actionable",
        ],
    );
    assert_eq!(status, 6, "{err}");
    assert!(has(&report, "not-updated-since", "EX-23"), "{report}");

    let mock = Mock::start(replies());
    let (_, report, _) = json_run(
        &sb,
        &mock,
        &["--issues", "EX-23", "--since", "2000-01-01T00:00:00Z"],
    );
    assert!(!has(&report, "not-updated-since", "EX-23"), "{report}");
}

// ------------------------------------------------------------- workspaces

#[test]
fn a_workspace_that_cannot_be_audited_fails_the_command_but_not_the_others() {
    let sb = workspace();
    // No credentials for this one.
    let o = sb.run(
        &["workspace", "add", "other", "--url-key", "other"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    let mock = Mock::start(replies());
    let (status, report, err) = json_run(&sb, &mock, &[]);
    assert_eq!(
        status, 1,
        "an unreachable workspace is not a clean result: {err}"
    );
    assert!(has(&report, "status-update-outdated", "aaaaaaaaaaaa"));
    assert_eq!(report["failedWorkspaces"][0]["workspace"], "other");
    assert!(report["failedWorkspaces"][0]["message"]
        .as_str()
        .unwrap()
        .contains("no credentials"));
    assert!(err.contains("other"), "{err}");

    // With --fail-on, findings still win: the hook is told to stop.
    let mock = Mock::start(replies());
    let (status, _, err) = json_run(&sb, &mock, &["--fail-on", "actionable"]);
    assert_eq!(status, 6, "{err}");
    assert!(err.contains("other"), "{err}");

    // --workspace narrows it to one.
    let mock = Mock::start(replies());
    let (status, report, err) = json_run(&sb, &mock, &["-w", "example"]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(report["failedWorkspaces"], json!([]));
}

#[test]
fn a_key_for_another_workspace_is_refused() {
    let sb = workspace();
    let mut who: Value = serde_json::from_str(&fixture("whoami")).unwrap();
    who["data"]["organization"]["urlKey"] = json!("someone-else");
    let mock = Mock::start(vec![ok(&who.to_string())]);
    let (status, report, _) = json_run(&sb, &mock, &[]);
    assert_eq!(status, 1);
    assert_eq!(report["findings"], json!([]));
    assert!(report["failedWorkspaces"][0]["message"]
        .as_str()
        .unwrap()
        .contains("someone-else"));
}

// ----------------------------------------------------------------- --cached

#[test]
fn cached_reads_the_last_refresh_without_the_network() {
    let sb = workspace();
    // Unknown first: no entry is not "no findings".
    let o = sb.run(&["audit", "--cached", "--json"], None, &[]);
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    assert!(stderr(&o).contains("nothing cached"), "{}", stderr(&o));

    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);

    let o = sb.run(&["audit", "--cached", "--json"], None, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let cached: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(has(&cached, "status-update-outdated", "aaaaaaaaaaaa"));
    assert_eq!(cached["cached"][0]["workspace"], "example");
    assert!(cached["cached"][0]["ageSecs"].as_u64().unwrap() < 60);

    // Same findings as asking Linear now.
    let mock = Mock::start(replies());
    let (_, live, _) = json_run(&sb, &mock, &[]);
    assert_eq!(cached["findings"], live["findings"]);

    // --fail-on works on cached findings.
    let o = sb.run(&["audit", "--cached", "--fail-on", "actionable"], None, &[]);
    assert_eq!(code(&o), 6, "{}", stderr(&o));
}

#[test]
fn cached_refuses_an_entry_past_its_ttl() {
    let sb = workspace();
    let mock = Mock::start(replies());
    assert_eq!(code(&linear(&sb, &mock, &["cache", "refresh"])), 0);
    let file = sb.cache_dir().join("example.json");
    let mut entry: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    entry["fetchedAt"] = json!((chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339());
    std::fs::write(&file, entry.to_string()).unwrap();

    let o = sb.run(&["audit", "--cached"], None, &[]);
    assert_eq!(code(&o), 1, "unknown must not look clean");
    assert!(stderr(&o).contains("past the 300s TTL"), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("linear cache refresh"),
        "{}",
        stderr(&o)
    );
    assert!(stdout(&o).is_empty());

    let o = sb.run(&["audit", "--cached", "--ttl", "7200"], None, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn cached_cannot_be_narrowed() {
    let sb = workspace();
    let o = sb.run(&["audit", "--cached", "--issues", "EX-1"], None, &[]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}
