//! Consistency rules: project state vs issues, overdue, issues without a
//! milestone, projects without a lead. `now` is fixed (2026-10-20).

mod common;

use common::*;
use linear_core::audit::{Finding, RuleId, Severity, TargetKind};

/// The findings of the consistency rules only (the staleness rules have their
/// own tests).
fn of_consistency(findings: &[Finding]) -> Vec<&Finding> {
    findings
        .iter()
        .filter(|f| {
            !matches!(
                f.rule,
                RuleId::StaleInProgress | RuleId::StatusUpdateOutdated
            )
        })
        .collect()
}

fn rule(s: &linear_core::audit::Snapshot, r: RuleId) -> Vec<(String, bool)> {
    of_rule(&audit(s, now()).findings, r)
}

// ------------------------------------------------- project-state-vs-issues

#[test]
fn completed_project_with_open_issues_is_flagged() {
    for open in ["triage", "backlog", "unstarted", "started"] {
        let p = project("done")
            .status("completed")
            .lead_me()
            .issue_states(&["completed", open])
            .build();
        let got = rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues);
        assert_eq!(got, [("done".to_owned(), true)], "open issue type {open}");
    }
}

#[test]
fn canceled_project_with_open_issues_is_flagged() {
    let p = project("gone")
        .status("canceled")
        .issue_states(&["started"])
        .build();
    assert_eq!(
        rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues),
        [("gone".to_owned(), false)]
    );
}

#[test]
fn completed_project_with_only_closed_issues_is_fine() {
    // Duplicate counts as closed.
    let p = project("done")
        .status("completed")
        .issue_states(&["completed", "canceled", "duplicate"])
        .build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).is_empty());
}

#[test]
fn unstarted_project_with_work_in_progress_is_flagged() {
    for status in ["backlog", "planned"] {
        let p = project("early")
            .status(status)
            .issue_states(&["unstarted", "started"])
            .build();
        assert_eq!(
            rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).len(),
            1,
            "{status}"
        );
    }
}

#[test]
fn unstarted_project_with_unstarted_issues_is_fine() {
    let p = project("early")
        .status("planned")
        .issue_states(&["unstarted", "backlog"])
        .build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).is_empty());
}

#[test]
fn started_project_with_every_issue_closed_is_flagged() {
    let p = project("finished")
        .issue_states(&["completed", "canceled"])
        .build();
    let report = audit(&snapshot(vec![], vec![p]), now());
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::ProjectStateVsIssues);
    assert_eq!(f.severity, Severity::Warn);
    assert!(
        f.message.contains("all 2 issue(s) are closed"),
        "{}",
        f.message
    );
    assert_eq!(
        f.fix,
        "linear project update finished --status completed -w ken109"
    );
}

#[test]
fn started_project_without_issues_is_fine() {
    let p = project("empty").build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).is_empty());
}

#[test]
fn a_truncated_issue_window_never_proves_that_everything_is_closed() {
    let p = project("big")
        .issue_states(&["completed", "completed"])
        .issues_truncated()
        .build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).is_empty());
}

#[test]
fn a_truncated_window_still_proves_a_mismatch_it_can_see() {
    let p = project("big")
        .status("completed")
        .issue_states(&["started"])
        .issues_truncated()
        .build();
    assert_eq!(
        rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).len(),
        1
    );
}

#[test]
fn a_paused_project_is_never_compared_with_its_issues() {
    let p = project("paused")
        .status("paused")
        .issue_states(&["started", "completed"])
        .build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectStateVsIssues).is_empty());
}

// ------------------------------------------------------------------ overdue

#[test]
fn overdue_covers_projects_milestones_and_issues() {
    let p = project("late")
        .lead_me()
        .target("2026-10-10")
        .milestone("M1", Some("2026-10-15"), "overdue")
        .build();
    let i = issue("KK-1")
        .mine()
        .state("started")
        .due("2026-10-19")
        .build();
    let report = audit(&snapshot(vec![i], vec![p]), now());
    let got: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == RuleId::Overdue)
        .map(|f| (f.target.kind, f.target.identifier.as_str(), f.actionable))
        .collect();
    assert_eq!(
        got,
        [
            (TargetKind::Project, "late", true),
            (TargetKind::Milestone, "M1", true),
            (TargetKind::Issue, "KK-1", true),
        ]
    );
}

#[test]
fn due_today_is_not_overdue() {
    let p = project("today").target("2026-10-20").build();
    let i = issue("KK-1").due("2026-10-20").build();
    assert!(rule(&snapshot(vec![i], vec![p]), RuleId::Overdue).is_empty());
}

#[test]
fn due_yesterday_is_one_day_overdue() {
    let i = issue("KK-1").due("2026-10-19").build();
    let report = audit(&snapshot(vec![i], vec![]), now());
    assert!(
        report.findings[0].message.contains("(1 day ago)"),
        "{}",
        report.findings[0].message
    );
}

#[test]
fn closed_work_is_never_overdue() {
    let p = project("done")
        .status("completed")
        .target("2026-01-01")
        .milestone("M1", Some("2026-01-01"), "overdue")
        .build();
    let c = project("cancelled")
        .status("canceled")
        .target("2026-01-01")
        .build();
    let mut issues = Vec::new();
    for s in ["completed", "canceled", "duplicate"] {
        issues.push(issue(&format!("KK-{s}")).state(s).due("2026-01-01").build());
    }
    assert!(rule(&snapshot(issues, vec![p, c]), RuleId::Overdue).is_empty());
}

#[test]
fn a_finished_milestone_is_not_overdue_but_an_open_backlog_issue_is() {
    let p = project("p")
        .milestone("Shipped", Some("2026-01-01"), "done")
        .build();
    let i = issue("KK-1").state("backlog").due("2026-10-01").build();
    let got = rule(&snapshot(vec![i], vec![p]), RuleId::Overdue);
    assert_eq!(got, [("KK-1".to_owned(), false)]);
}

#[test]
fn overdue_ownership_follows_the_owner_of_each_target() {
    let p = project("p").lead_other().target("2026-10-01").build();
    let mine = issue("KK-1").mine().due("2026-10-01").build();
    let theirs = issue("KK-2").theirs().due("2026-10-01").build();
    let nobody = issue("KK-3").due("2026-10-01").build();
    let got = rule(
        &snapshot(vec![mine, theirs, nobody], vec![p]),
        RuleId::Overdue,
    );
    assert_eq!(
        got,
        [
            ("p".to_owned(), false),
            ("KK-1".to_owned(), true),
            ("KK-2".to_owned(), false),
            ("KK-3".to_owned(), false),
        ]
    );
}

#[test]
fn an_overdue_milestone_belongs_to_the_project_lead() {
    let p = project("p")
        .lead_me()
        .milestone("M1", Some("2026-10-01"), "overdue")
        .build();
    let report = audit(&snapshot(vec![], vec![p]), now());
    let f = &report.findings[0];
    assert_eq!(f.target.kind, TargetKind::Milestone);
    assert!(f.actionable);
    assert_eq!(f.target.url, "https://linear.app/x/project/p");
}

// -------------------------------------------------- issue-without-milestone

#[test]
fn open_issue_outside_the_milestones_of_its_project_is_flagged() {
    let p = project("p").milestone("M1", None, "next").build();
    let loose = issue("KK-1").mine().in_project("p").build();
    let placed = issue("KK-2")
        .in_project("p")
        .in_milestone("p", "M1")
        .build();
    let report = audit(&snapshot(vec![loose, placed], vec![p]), now());
    let got = of_rule(&report.findings, RuleId::IssueWithoutMilestone);
    assert_eq!(got, [("KK-1".to_owned(), true)]);
    assert_eq!(report.findings[0].severity, Severity::Info);
}

#[test]
fn issues_without_a_milestone_are_fine_when_nothing_asks_for_one() {
    // No milestones in the project, no project, project unknown to the
    // snapshot, closed project, closed issue.
    let bare = project("bare").build();
    let done = project("done")
        .status("completed")
        .milestone("M1", None, "done")
        .build();
    let issues = vec![
        issue("KK-1").in_project("bare").build(),
        issue("KK-2").build(),
        issue("KK-3").in_project("elsewhere").build(),
        issue("KK-4").in_project("done").build(),
        issue("KK-5").in_project("done").state("completed").build(),
    ];
    assert!(rule(
        &snapshot(issues, vec![bare, done]),
        RuleId::IssueWithoutMilestone
    )
    .is_empty());
}

#[test]
fn a_closed_issue_needs_no_milestone() {
    let p = project("p").milestone("M1", None, "next").build();
    let i = issue("KK-1").state("canceled").in_project("p").build();
    assert!(rule(&snapshot(vec![i], vec![p]), RuleId::IssueWithoutMilestone).is_empty());
}

// ------------------------------------------------------ project-without-lead

#[test]
fn project_without_a_lead_is_informational_and_never_actionable() {
    let leaderless = project("a").build();
    let led = project("b").lead_other().build();
    let report = audit(&snapshot(vec![], vec![leaderless, led]), now());
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::ProjectWithoutLead);
    assert_eq!(f.severity, Severity::Info);
    assert!(!f.actionable);
    assert_eq!(f.fix, "linear project update a --lead <user> -w ken109");
    assert_eq!(report.findings.len(), 1);
}

#[test]
fn a_closed_project_may_have_no_lead() {
    let p = project("old").status("completed").build();
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::ProjectWithoutLead).is_empty());
}

// ------------------------------------------------------------------- shape

#[test]
fn findings_carry_workspace_target_and_a_fix_for_that_workspace() {
    let i = issue("KK-9").mine().due("2026-10-01").build();
    let report = audit(&snapshot(vec![i], vec![]), now());
    let f = &report.findings[0];
    assert_eq!(f.workspace, "ken109");
    assert_eq!(f.target.kind, TargetKind::Issue);
    assert_eq!(f.target.id, "i-KK-9");
    assert_eq!(f.target.url, "https://linear.app/x/issue/KK-9");
    assert_eq!(f.fix, "linear issue update KK-9 --due <date> -w ken109");
}

#[test]
fn findings_are_ordered_by_rule_then_kind_then_identifier() {
    let p = project("p").target("2026-10-01").build();
    let issues = vec![
        issue("KK-2").due("2026-10-01").build(),
        issue("KK-1").due("2026-10-01").build(),
    ];
    let report = audit(&snapshot(issues, vec![p]), now());
    let order: Vec<_> = report
        .findings
        .iter()
        .map(|f| (f.rule, f.target.identifier.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (RuleId::Overdue, "p"),
            (RuleId::Overdue, "KK-1"),
            (RuleId::Overdue, "KK-2"),
            (RuleId::ProjectWithoutLead, "p"),
        ]
    );
}

#[test]
fn a_report_round_trips_through_json_with_stable_rule_names() {
    let i = issue("KK-1").mine().due("2026-10-01").build();
    let report = audit(&snapshot(vec![i], vec![]), now());
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["findings"][0]["rule"], "overdue");
    assert_eq!(json["findings"][0]["severity"], "warn");
    assert_eq!(json["findings"][0]["target"]["kind"], "issue");
    let back: linear_core::audit::AuditReport = serde_json::from_value(json).unwrap();
    assert_eq!(back, report);
}

#[test]
fn a_snapshot_round_trips_through_json() {
    let p = project("p")
        .lead_me()
        .milestone("M1", Some("2026-10-01"), "overdue")
        .build();
    let i = issue("KK-1")
        .in_project("p")
        .in_milestone("p", "M1")
        .build();
    let s = snapshot(vec![i], vec![p]);
    let back: linear_core::audit::Snapshot =
        serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

// ------------------------------------------- real (anonymized) responses

#[test]
fn the_sandbox_data_is_clean_when_nothing_is_late() {
    // The fixture was captured on 2026-10-06; its dates are all later.
    let s = sandbox_snapshot();
    let report = audit(
        &s,
        chrono::Utc.with_ymd_and_hms(2026, 10, 7, 0, 0, 0).unwrap(),
    );
    let consistency = of_consistency(&report.findings);
    assert!(consistency.is_empty(), "{consistency:#?}");
}

#[test]
fn the_sandbox_data_goes_overdue_when_the_dates_pass() {
    let s = sandbox_snapshot();
    let late = chrono::Utc.with_ymd_and_hms(2027, 1, 5, 0, 0, 0).unwrap();
    let report = audit(&s, late);
    let got: Vec<_> = of_consistency(&report.findings)
        .into_iter()
        .map(|f| {
            (
                f.rule,
                f.target.kind,
                f.target.identifier.as_str(),
                f.actionable,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (RuleId::Overdue, TargetKind::Project, "aaaaaaaaaaaa", true),
            (RuleId::Overdue, TargetKind::Milestone, "Milestone 1", true),
            (RuleId::Overdue, TargetKind::Issue, "EX-23", true),
        ]
    );
}

use chrono::TimeZone;
