//! Staleness rules: stale-in-progress and status-update-outdated. `now` is
//! fixed (2026-10-20 12:00 UTC); the defaults are 7 and 14 days.

mod common;

use common::*;
use linear_core::audit::{AuditConfig, RuleId, Severity};

fn rule(s: &linear_core::audit::Snapshot, r: RuleId) -> Vec<(String, bool)> {
    of_rule(&audit(s, now()).findings, r)
}

// -------------------------------------------------------- stale-in-progress

#[test]
fn an_issue_idle_for_exactly_the_limit_is_stale() {
    let i = issue("KK-1")
        .state("started")
        .updated("2026-10-13T12:00:00Z")
        .build();
    assert_eq!(
        rule(&snapshot(vec![i], vec![]), RuleId::StaleInProgress),
        [("KK-1".to_owned(), false)]
    );
}

#[test]
fn an_issue_a_second_short_of_the_limit_is_not_stale() {
    let i = issue("KK-1")
        .state("started")
        .updated("2026-10-13T12:00:01Z")
        .build();
    assert!(rule(&snapshot(vec![i], vec![]), RuleId::StaleInProgress).is_empty());
}

#[test]
fn only_issues_in_progress_go_stale() {
    let issues: Vec<_> = [
        "triage",
        "backlog",
        "unstarted",
        "completed",
        "canceled",
        "duplicate",
    ]
    .iter()
    .map(|s| {
        issue(&format!("KK-{s}"))
            .state(s)
            .updated("2026-01-01T00:00:00Z")
            .build()
    })
    .collect();
    assert!(rule(&snapshot(issues, vec![]), RuleId::StaleInProgress).is_empty());
}

#[test]
fn staleness_is_decided_on_the_state_type_not_the_name() {
    // A team that renamed "In Progress" to "Doing" still has a started state.
    let mut i = issue("KK-1").updated("2026-10-01T00:00:00Z").build();
    i.state.type_ = "started".into();
    i.state.name = "Doing".into();
    let report = audit(&snapshot(vec![i], vec![]), now());
    assert_eq!(report.findings.len(), 1);
    assert!(
        report.findings[0]
            .message
            .contains("been Doing with no update for 19 days (limit 7)"),
        "{}",
        report.findings[0].message
    );
}

#[test]
fn a_stale_issue_is_actionable_only_for_its_assignee() {
    let issues = vec![
        issue("KK-1")
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .mine()
            .build(),
        issue("KK-2")
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .theirs()
            .build(),
        issue("KK-3")
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .build(),
    ];
    assert_eq!(
        rule(&snapshot(issues, vec![]), RuleId::StaleInProgress),
        [
            ("KK-1".to_owned(), true),
            ("KK-2".to_owned(), false),
            ("KK-3".to_owned(), false),
        ]
    );
}

#[test]
fn the_limit_is_configurable() {
    let i = issue("KK-1")
        .state("started")
        .updated("2026-10-16T00:00:00Z")
        .build();
    let s = snapshot(vec![i], vec![]);
    assert!(rule(&s, RuleId::StaleInProgress).is_empty());
    let tight = AuditConfig {
        stale_days: 4,
        ..AuditConfig::default()
    };
    let report = linear_core::audit::audit(&s, &tight, now());
    assert_eq!(report.findings.len(), 1);
    assert!(report.findings[0].message.contains("(limit 4)"));
}

#[test]
fn a_stale_issue_is_a_warning_with_a_fix() {
    let i = issue("KK-1")
        .state("started")
        .updated("2026-10-01T00:00:00Z")
        .build();
    let report = audit(&snapshot(vec![i], vec![]), now());
    let f = &report.findings[0];
    assert_eq!(f.severity, Severity::Warn);
    assert_eq!(f.fix, "linear issue update KK-1 --state <state> -w ken109");
}

// ---------------------------------------------------- status-update-outdated

/// A project that is in progress, led by me, whose update was posted at `at`.
fn updated_at(at: &str) -> linear_core::types::Project {
    project("p").lead_me().update_at(at).build()
}

#[test]
fn a_fresh_update_with_nothing_changed_after_it_is_fine() {
    let p = updated_at("2026-10-19T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("started")
        .started("2026-10-18T00:00:00Z")
        .build();
    assert!(rule(&snapshot(vec![i], vec![p]), RuleId::StatusUpdateOutdated).is_empty());
}

#[test]
fn an_update_as_old_as_the_limit_is_outdated() {
    let p = updated_at("2026-10-06T12:00:00Z");
    let report = audit(&snapshot(vec![], vec![p]), now());
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::StatusUpdateOutdated);
    assert!(f.actionable);
    assert!(
        f.message.contains("is 14 days old (limit 14)"),
        "{}",
        f.message
    );
    assert_eq!(
        f.fix,
        "linear project status-update p --body <text> -w ken109"
    );
}

#[test]
fn an_update_a_second_short_of_the_limit_is_current() {
    let p = updated_at("2026-10-06T12:00:01Z");
    assert!(rule(&snapshot(vec![], vec![p]), RuleId::StatusUpdateOutdated).is_empty());
}

#[test]
fn an_update_older_than_the_last_state_change_of_an_issue_is_outdated() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let done = issue("KK-7")
        .in_project("p")
        .state("completed")
        .started("2026-10-10T00:00:00Z")
        .completed("2026-10-17T00:00:00Z")
        .build();
    let report = audit(&snapshot(vec![done], vec![p]), now());
    let f = &report.findings[0];
    assert!(
        f.message.contains(
            "latest status update (2026-10-15) is older than the last issue state change (2026-10-17, KK-7)"
        ),
        "{}",
        f.message
    );
    assert!(!f.message.contains("days old"), "{}", f.message);
}

#[test]
fn the_latest_state_change_among_issues_counts() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let issues = vec![
        issue("KK-1")
            .in_project("p")
            .state("started")
            .started("2026-10-12T00:00:00Z")
            .build(),
        issue("KK-2")
            .in_project("p")
            .state("started")
            .started("2026-10-18T00:00:00Z")
            .build(),
        issue("KK-3")
            .in_project("p")
            .state("started")
            .started("2026-10-16T00:00:00Z")
            .build(),
    ];
    let report = audit(&snapshot(issues, vec![p]), now());
    assert!(
        report.findings[0].message.contains("KK-2"),
        "{}",
        report.findings[0].message
    );
}

#[test]
fn a_state_change_before_the_update_is_already_covered_by_it() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("completed")
        .started("2026-10-01T00:00:00Z")
        .completed("2026-10-14T23:59:59Z")
        .build();
    assert!(rule(&snapshot(vec![i], vec![p]), RuleId::StatusUpdateOutdated).is_empty());
}

#[test]
fn a_cancellation_after_the_update_counts() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("canceled")
        .canceled("2026-10-18T00:00:00Z")
        .updated("2026-10-18T00:00:00Z")
        .build();
    assert_eq!(
        rule(&snapshot(vec![i], vec![p]), RuleId::StatusUpdateOutdated).len(),
        1
    );
}

#[test]
fn a_cancellation_before_the_update_is_covered_even_if_the_issue_was_edited_since() {
    // updatedAt moves on every edit; canceledAt says when the state really changed.
    let p = updated_at("2026-10-15T00:00:00Z");
    for state in ["canceled", "duplicate"] {
        let i = issue("KK-1")
            .in_project("p")
            .state(state)
            .canceled("2026-10-10T00:00:00Z")
            .updated("2026-10-19T00:00:00Z")
            .build();
        assert!(
            rule(
                &snapshot(vec![i], vec![p.clone()]),
                RuleId::StatusUpdateOutdated
            )
            .is_empty(),
            "{state}"
        );
    }
}

#[test]
fn a_cancellation_without_a_timestamp_falls_back_to_updated_at() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("canceled")
        .updated("2026-10-18T00:00:00Z")
        .build();
    assert_eq!(
        rule(&snapshot(vec![i], vec![p]), RuleId::StatusUpdateOutdated).len(),
        1
    );
}

#[test]
fn an_edit_of_a_waiting_issue_is_not_a_state_change() {
    // Backlog issue touched after the update: no startedAt/completedAt, not canceled.
    let p = updated_at("2026-10-15T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("backlog")
        .updated("2026-10-19T00:00:00Z")
        .build();
    assert!(rule(&snapshot(vec![i], vec![p]), RuleId::StatusUpdateOutdated).is_empty());
}

#[test]
fn issues_of_other_projects_do_not_count() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let elsewhere = issue("KK-1")
        .in_project("other")
        .state("started")
        .started("2026-10-19T00:00:00Z")
        .build();
    let loose = issue("KK-2")
        .state("started")
        .started("2026-10-19T00:00:00Z")
        .build();
    assert!(rule(
        &snapshot(vec![elsewhere, loose], vec![p]),
        RuleId::StatusUpdateOutdated
    )
    .is_empty());
}

#[test]
fn both_reasons_are_reported_in_one_finding() {
    let p = updated_at("2026-10-01T00:00:00Z");
    let i = issue("KK-1")
        .in_project("p")
        .state("started")
        .started("2026-10-18T00:00:00Z")
        .build();
    let report = audit(&snapshot(vec![i], vec![p]), now());
    assert_eq!(report.findings.len(), 1);
    let m = &report.findings[0].message;
    assert!(
        m.contains("is older than the last issue state change") && m.contains("19 days old"),
        "{m}"
    );
}

#[test]
fn a_project_in_progress_with_no_update_at_all_is_outdated() {
    let p = project("p").lead_me().no_update().build();
    let report = audit(&snapshot(vec![], vec![p]), now());
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::StatusUpdateOutdated);
    assert!(
        f.message.contains("it has no status update"),
        "{}",
        f.message
    );
}

#[test]
fn only_projects_in_progress_need_status_updates() {
    let projects: Vec<_> = ["backlog", "planned", "paused", "completed", "canceled"]
        .iter()
        .map(|s| {
            project(s)
                .status(s)
                .update_at("2026-01-01T00:00:00Z")
                .build()
        })
        .collect();
    assert!(rule(&snapshot(vec![], projects), RuleId::StatusUpdateOutdated).is_empty());
}

#[test]
fn the_update_limit_is_configurable() {
    let p = updated_at("2026-10-15T00:00:00Z");
    let s = snapshot(vec![], vec![p]);
    assert!(rule(&s, RuleId::StatusUpdateOutdated).is_empty());
    let tight = AuditConfig {
        status_update_days: 5,
        ..AuditConfig::default()
    };
    let report = linear_core::audit::audit(&s, &tight, now());
    assert_eq!(report.findings.len(), 1);
    assert!(report.findings[0].message.contains("5 days old (limit 5)"));
}

#[test]
fn a_project_is_actionable_only_for_its_lead() {
    let mine = project("a")
        .lead_me()
        .update_at("2026-09-01T00:00:00Z")
        .build();
    let theirs = project("b")
        .lead_other()
        .update_at("2026-09-01T00:00:00Z")
        .build();
    let nobody = project("c").update_at("2026-09-01T00:00:00Z").build();
    assert_eq!(
        rule(
            &snapshot(vec![], vec![mine, theirs, nobody]),
            RuleId::StatusUpdateOutdated
        ),
        [
            ("a".to_owned(), true),
            ("b".to_owned(), false),
            ("c".to_owned(), false)
        ]
    );
}

// ----------------------------------------------- actionable vs informational

#[test]
fn the_report_splits_actionable_from_informational() {
    let issues = vec![
        issue("KK-1")
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .mine()
            .build(),
        issue("KK-2")
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .theirs()
            .build(),
    ];
    let report = audit(&snapshot(issues, vec![]), now());
    let actionable: Vec<_> = report
        .actionable()
        .map(|f| f.target.identifier.as_str())
        .collect();
    let info: Vec<_> = report
        .informational()
        .map(|f| f.target.identifier.as_str())
        .collect();
    assert_eq!(actionable, ["KK-1"]);
    assert_eq!(info, ["KK-2"]);
    assert!(report.has_actionable());
}

#[test]
fn informational_findings_alone_never_count_as_actionable() {
    let i = issue("KK-2")
        .state("started")
        .updated("2026-10-01T00:00:00Z")
        .theirs()
        .build();
    let report = audit(&snapshot(vec![i], vec![]), now());
    assert_eq!(report.findings.len(), 1);
    assert!(!report.has_actionable());
    assert!(!audit(&snapshot(vec![], vec![]), now()).has_actionable());
}

#[test]
fn thresholds_come_from_workspace_settings_with_defaults_for_what_is_absent() {
    let c: AuditConfig = serde_json::from_str(r#"{ "stale_days": 3 }"#).unwrap();
    assert_eq!(
        c,
        AuditConfig {
            stale_days: 3,
            status_update_days: 14,
            validators: vec![]
        }
    );
    assert_eq!(
        serde_json::from_str::<AuditConfig>("{}").unwrap(),
        AuditConfig::default()
    );
    assert_eq!(
        AuditConfig::default(),
        AuditConfig {
            stale_days: 7,
            status_update_days: 14,
            validators: vec![]
        }
    );
    assert!(serde_json::from_str::<AuditConfig>(r#"{ "stale": 3 }"#).is_err());
}

// ------------------------------------------- real (anonymized) responses

#[test]
fn the_sandbox_project_is_outdated_because_its_issue_started_after_the_update() {
    // In the captured data the issue was started seconds after the update was
    // posted: a real case of "an issue changed state after the last update".
    let s = sandbox_snapshot();
    let report = audit(&s, ts("2026-10-07T00:00:00Z"));
    let got: Vec<_> = report
        .findings
        .iter()
        .map(|f| (f.rule, f.target.identifier.as_str(), f.actionable))
        .collect();
    assert_eq!(got, [(RuleId::StatusUpdateOutdated, "aaaaaaaaaaaa", true)]);
}

#[test]
fn the_sandbox_issue_goes_stale_and_its_project_outdated_as_time_passes() {
    let s = sandbox_snapshot();
    let report = audit(&s, ts("2026-10-25T00:00:00Z"));
    let got: Vec<_> = report
        .findings
        .iter()
        .map(|f| (f.rule, f.target.identifier.as_str(), f.actionable))
        .collect();
    assert_eq!(
        got,
        [
            (RuleId::StaleInProgress, "EX-23", true),
            (RuleId::StatusUpdateOutdated, "aaaaaaaaaaaa", true),
        ]
    );
}
