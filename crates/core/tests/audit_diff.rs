//! `diff`: which findings are new since the previous audit (so a notifier
//! reports each problem once).

mod common;

use common::*;
use linear_core::audit::{diff, Finding, RuleId};

/// The findings of an audit at `now()` (2026-10-20) of issues due on the given dates.
fn overdue(ids: &[&str]) -> Vec<Finding> {
    let issues = ids
        .iter()
        .map(|id| issue(id).due("2026-10-01").build())
        .collect();
    audit(&snapshot(issues, vec![]), now()).findings
}

fn ids(findings: &[Finding]) -> Vec<&str> {
    findings
        .iter()
        .map(|f| f.target.identifier.as_str())
        .collect()
}

#[test]
fn only_findings_absent_before_are_new() {
    let new = diff(
        &overdue(&["KK-1", "KK-2"]),
        &overdue(&["KK-1", "KK-2", "KK-3"]),
    );
    assert_eq!(ids(&new), ["KK-3"]);
}

#[test]
fn everything_is_new_without_an_earlier_audit() {
    let now_findings = overdue(&["KK-1", "KK-2"]);
    assert_eq!(diff(&[], &now_findings), now_findings);
}

#[test]
fn nothing_is_new_when_nothing_changed() {
    let f = overdue(&["KK-1", "KK-2"]);
    assert!(diff(&f, &f).is_empty());
    assert!(diff(&f, &[]).is_empty());
}

#[test]
fn a_finding_whose_message_changed_is_not_new() {
    // The same overdue issue the next day says "20 days ago" instead of "19".
    let yesterday = audit(
        &snapshot(vec![issue("KK-1").due("2026-10-01").build()], vec![]),
        ts("2026-10-20T12:00:00Z"),
    )
    .findings;
    let today = audit(
        &snapshot(vec![issue("KK-1").due("2026-10-01").build()], vec![]),
        ts("2026-10-21T12:00:00Z"),
    )
    .findings;
    assert_ne!(yesterday[0].message, today[0].message);
    assert!(diff(&yesterday, &today).is_empty());
}

#[test]
fn a_finding_that_went_away_and_came_back_is_new_again() {
    let first = overdue(&["KK-1"]);
    let fixed = overdue(&[]);
    let back = overdue(&["KK-1"]);
    assert!(diff(&first, &fixed).is_empty());
    assert_eq!(ids(&diff(&fixed, &back)), ["KK-1"]);
}

#[test]
fn the_same_target_under_another_rule_is_a_new_finding() {
    let i = issue("KK-1")
        .mine()
        .state("started")
        .due("2026-10-01")
        .updated("2026-10-01T00:00:00Z");
    let overdue_only = audit(
        &snapshot(
            vec![issue("KK-1")
                .mine()
                .state("started")
                .due("2026-10-01")
                .build()],
            vec![],
        ),
        now(),
    )
    .findings;
    let both = audit(&snapshot(vec![i.build()], vec![]), now()).findings;
    let new = diff(&overdue_only, &both);
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].rule, RuleId::StaleInProgress);
}

#[test]
fn the_same_target_id_in_another_workspace_is_a_new_finding() {
    let a = overdue(&["KK-1"]);
    let mut b = a.clone();
    b[0].workspace = "lt-three".to_owned();
    assert_eq!(diff(&a, &b), b);
}

#[test]
fn order_follows_the_current_audit_and_repeats_are_dropped() {
    let mut current = overdue(&["KK-2", "KK-1"]);
    current.push(current[0].clone());
    let new = diff(&[], &current);
    assert_eq!(ids(&new), ["KK-1", "KK-2"]);
}
