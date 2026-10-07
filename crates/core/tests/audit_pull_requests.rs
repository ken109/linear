//! The pull-request rules: pr-merged-issue-open and pr-open-too-long. `now` is
//! fixed (2026-10-20 12:00 UTC); a pull request may stay open 14 days.
//!
//! The attachments come from `fixtures/attachments_github.json`, in this order:
//! 0 a synced GitHub issue (not a pull request), 1 #41 open since 10-02,
//! 2 #42 draft since 09-01, 3 #43 merged, 4 #44 closed, 5 #45 with no readable
//! state, 6 a plain link to a pull request URL (not the integration's).

mod common;

use common::*;
use linear_core::audit::{audit, AuditConfig, RuleId, Severity};
use serde_json::Value;

fn attachment(n: usize) -> Value {
    let all: Value =
        serde_json::from_str(include_str!("fixtures/attachments_github.json")).unwrap();
    all["nodes"][n].clone()
}

fn with(identifier: &str, state: &str, attachments: &[usize]) -> linear_core::types::Issue {
    issue(identifier)
        .state(state)
        .mine()
        .attachments(attachments.iter().map(|n| attachment(*n)).collect())
        .build()
}

fn rule(issues: Vec<linear_core::types::Issue>, r: RuleId) -> Vec<(String, bool)> {
    of_rule(&audit_report(issues).findings, r)
}

fn audit_report(issues: Vec<linear_core::types::Issue>) -> linear_core::audit::AuditReport {
    audit(&snapshot(issues, vec![]), &AuditConfig::default(), now())
}

// ------------------------------------------------------ pr-merged-issue-open

#[test]
fn a_merged_pull_request_on_an_issue_still_in_progress_is_flagged() {
    let findings = audit_report(vec![with("KK-1", "started", &[3])]).findings;
    let merged: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == RuleId::PrMergedIssueOpen)
        .collect();
    assert_eq!(merged.len(), 1);
    let f = merged[0];
    assert_eq!(f.severity, Severity::Warn);
    assert!(f.actionable);
    assert_eq!(f.target.identifier, "KK-1");
    assert!(f.message.contains("#43"), "{}", f.message);
    assert_eq!(f.fix, "linear issue update KK-1 --state <state> -w ken109");
}

#[test]
fn it_only_warns() {
    // The finding names a fix for a person to run; the audit itself never writes.
    let f = audit_report(vec![with("KK-1", "started", &[3])]);
    assert!(f.findings.iter().all(|f| f.severity == Severity::Warn));
}

#[test]
fn an_issue_that_is_not_in_progress_is_not_flagged_for_a_merged_pull_request() {
    for state in ["triage", "backlog", "unstarted", "completed", "canceled"] {
        assert!(
            rule(vec![with("KK-1", state, &[3])], RuleId::PrMergedIssueOpen).is_empty(),
            "{state}"
        );
    }
}

#[test]
fn a_pull_request_still_waiting_keeps_the_issue_in_progress_legitimately() {
    // Merged #43 next to open #41, or next to draft #42.
    assert!(rule(
        vec![with("KK-1", "started", &[3, 1])],
        RuleId::PrMergedIssueOpen
    )
    .is_empty());
    assert!(rule(
        vec![with("KK-1", "started", &[3, 2])],
        RuleId::PrMergedIssueOpen
    )
    .is_empty());
    // A closed one beside it does not.
    assert_eq!(
        rule(
            vec![with("KK-1", "started", &[3, 4])],
            RuleId::PrMergedIssueOpen
        )
        .len(),
        1
    );
}

#[test]
fn only_a_merged_pull_request_counts() {
    for attachments in [&[1][..], &[2], &[4], &[5], &[0], &[6], &[]] {
        assert!(
            rule(
                vec![with("KK-1", "started", attachments)],
                RuleId::PrMergedIssueOpen
            )
            .is_empty(),
            "{attachments:?}"
        );
    }
}

#[test]
fn somebody_elses_issue_is_reported_but_not_actionable() {
    let other = issue("KK-2")
        .state("started")
        .theirs()
        .attachments(vec![attachment(3)])
        .build();
    assert_eq!(
        rule(vec![other], RuleId::PrMergedIssueOpen),
        [("KK-2".to_owned(), false)]
    );
}

// ------------------------------------------------------- pr-open-too-long

#[test]
fn a_pull_request_open_for_more_than_the_limit_is_flagged() {
    // #41 was opened on 10-02: 18 days before now.
    let findings = audit_report(vec![with("KK-1", "started", &[1])]).findings;
    let long: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == RuleId::PrOpenTooLong)
        .collect();
    assert_eq!(long.len(), 1);
    assert_eq!(long[0].severity, Severity::Warn);
    assert!(long[0].message.contains("18 days"), "{}", long[0].message);
    assert!(long[0].message.contains("#41"));
    assert_eq!(long[0].fix, "linear issue view KK-1 -w ken109");
}

#[test]
fn the_limit_is_in_whole_days_and_comes_from_the_config() {
    let issues = || vec![with("KK-1", "started", &[1])];
    let at = |days: u32| {
        let config = AuditConfig {
            pr_open_days: days,
            ..AuditConfig::default()
        };
        of_rule(
            &audit(&snapshot(issues(), vec![]), &config, now()).findings,
            RuleId::PrOpenTooLong,
        )
        .len()
    };
    assert_eq!(at(18), 1);
    assert_eq!(at(19), 0);
    assert_eq!(at(1), 1);
}

#[test]
fn a_draft_a_closed_and_a_merged_pull_request_are_not_open_too_long() {
    for attachments in [&[2][..], &[3], &[4], &[5], &[0], &[6], &[]] {
        assert!(
            rule(
                vec![with("KK-1", "started", attachments)],
                RuleId::PrOpenTooLong
            )
            .is_empty(),
            "{attachments:?}"
        );
    }
}

#[test]
fn the_oldest_pull_request_is_the_one_named() {
    let mut old = attachment(1);
    old["url"] = "https://github.com/example/app/pull/50".into();
    old["metadata"]["number"] = 50.into();
    old["metadata"]["createdAt"] = "2026-09-20T09:00:00.000Z".into();
    let i = issue("KK-1")
        .state("started")
        .mine()
        .attachments(vec![attachment(1), old])
        .build();
    let report = audit_report(vec![i]);
    let f = report
        .findings
        .iter()
        .find(|f| f.rule == RuleId::PrOpenTooLong)
        .unwrap();
    assert!(f.message.contains("#50"), "{}", f.message);
}

// ------------------------------------------------- workspaces without the integration

#[test]
fn issues_without_pull_requests_give_nothing_to_say() {
    // No attachments, a plain source attachment, a synced GitHub issue, a plain link.
    let issues = vec![
        issue("KK-1").state("started").mine().build(),
        issue("KK-2")
            .state("started")
            .mine()
            .source("https://github.com/example/app/pull/46")
            .build(),
        with("KK-3", "started", &[0]),
        with("KK-4", "started", &[6]),
    ];
    let report = audit_report(issues);
    assert!(
        report
            .findings
            .iter()
            .all(|f| !matches!(f.rule, RuleId::PrMergedIssueOpen | RuleId::PrOpenTooLong)),
        "{:?}",
        report.findings
    );
}

#[test]
fn the_rules_follow_the_scope_of_a_narrowed_audit() {
    use linear_core::audit::{audit_scoped, AuditOptions};
    let snapshot = snapshot(
        vec![with("KK-1", "started", &[3]), with("KK-2", "started", &[3])],
        vec![],
    );
    let options = AuditOptions {
        issues: Some(vec!["KK-2".to_owned()]),
        since: None,
    };
    let report = audit_scoped(&snapshot, &AuditConfig::default(), &options, now()).unwrap();
    assert_eq!(
        of_rule(&report.findings, RuleId::PrMergedIssueOpen),
        [("KK-2".to_owned(), true)]
    );
}
