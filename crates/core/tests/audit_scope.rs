//! Narrowing the audit to named issues (`--issues`, `--since`) and applying
//! validator rules to existing issues. `now` is fixed (2026-10-20 12:00 UTC).

mod common;

use common::*;
use linear_core::audit::{audit_scoped, AuditConfig, AuditOptions, RuleId, TargetKind};
use linear_core::config::Rule;
use linear_core::ErrorCode;

fn only(ids: &[&str]) -> AuditOptions {
    AuditOptions {
        issues: Some(ids.iter().map(|s| (*s).to_owned()).collect()),
        since: None,
    }
}

fn since(ids: &[&str], at: &str) -> AuditOptions {
    AuditOptions {
        since: Some(ts(at)),
        ..only(ids)
    }
}

fn run(s: &linear_core::audit::Snapshot, o: &AuditOptions) -> linear_core::audit::AuditReport {
    audit_scoped(s, &AuditConfig::default(), o, now()).unwrap()
}

fn late_issue(id: &str) -> linear_core::types::Issue {
    issue(id).mine().due("2026-10-01").build()
}

// ------------------------------------------------------------------ scoping

#[test]
fn only_the_named_issues_are_audited() {
    let s = snapshot(vec![late_issue("KK-1"), late_issue("KK-2")], vec![]);
    let report = run(&s, &only(&["KK-2"]));
    let ids: Vec<_> = report
        .findings
        .iter()
        .map(|f| f.target.identifier.as_str())
        .collect();
    assert_eq!(ids, ["KK-2"]);
    assert!(report.unresolved_issues.is_empty());
}

#[test]
fn identifiers_match_without_regard_to_case_or_padding() {
    let s = snapshot(vec![late_issue("KK-1")], vec![]);
    let report = run(&s, &only(&[" kk-1 "]));
    assert_eq!(report.findings.len(), 1);
    assert!(report.unresolved_issues.is_empty());
}

#[test]
fn the_projects_of_the_named_issues_are_audited_too() {
    // A project without a lead, an overdue milestone and an outdated project:
    // all findings about the project KK-1 sits in. Another project is left alone.
    let mine = project("mine")
        .milestone("M1", Some("2026-10-01"), "overdue")
        .update_at("2026-09-01T00:00:00Z")
        .build();
    let other = project("other").target("2026-10-01").build();
    let i = issue("KK-1")
        .in_project("mine")
        .in_milestone("mine", "M1")
        .build();
    let s = snapshot(vec![i], vec![mine, other]);
    let report = run(&s, &only(&["KK-1"]));
    let got: Vec<_> = report
        .findings
        .iter()
        .map(|f| (f.rule, f.target.kind, f.target.identifier.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (RuleId::Overdue, TargetKind::Milestone, "M1"),
            (RuleId::ProjectWithoutLead, TargetKind::Project, "mine"),
            (RuleId::StatusUpdateOutdated, TargetKind::Project, "mine"),
        ]
    );
}

#[test]
fn a_narrowed_audit_is_the_full_audit_cut_down_to_the_scope() {
    let p = project("p")
        .lead_me()
        .target("2026-10-01")
        .milestone("M1", Some("2026-10-01"), "overdue")
        .update_at("2026-09-01T00:00:00Z")
        .build();
    let q = project("q").target("2026-10-01").build();
    let issues = vec![
        late_issue("KK-1"),
        issue("KK-2")
            .in_project("p")
            .mine()
            .state("started")
            .updated("2026-10-01T00:00:00Z")
            .build(),
        issue("KK-3").in_project("q").due("2026-10-02").build(),
        issue("KK-4").in_project("p").build(),
    ];
    let s = snapshot(issues, vec![p, q]);
    let full = audit(&s, now());
    let scoped = run(&s, &only(&["KK-2"]));
    let expected: Vec<_> = full
        .findings
        .iter()
        .filter(|f| match f.target.kind {
            TargetKind::Issue => f.target.identifier == "KK-2",
            TargetKind::Project => f.target.identifier == "p",
            TargetKind::Milestone => true, // the only milestone belongs to p
        })
        .cloned()
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(scoped.findings, expected);
}

#[test]
fn state_changes_of_unnamed_issues_still_count_for_the_named_projects() {
    let p = project("p")
        .lead_me()
        .update_at("2026-10-10T00:00:00Z")
        .build();
    let named = issue("KK-1").in_project("p").build();
    let other = issue("KK-2")
        .in_project("p")
        .state("completed")
        .completed("2026-10-15T00:00:00Z")
        .build();
    let s = snapshot(vec![named, other], vec![p]);
    let report = run(&s, &only(&["KK-1"]));
    assert_eq!(
        of_rule(&report.findings, RuleId::StatusUpdateOutdated).len(),
        1
    );
}

#[test]
fn named_issues_missing_from_the_snapshot_are_reported_not_dropped() {
    let s = snapshot(vec![issue("KK-1").build()], vec![]);
    let report = run(&s, &only(&["KK-1", "KK-9", "kk-9", "KK-10"]));
    assert_eq!(report.unresolved_issues, ["KK-9", "KK-10"]);
}

#[test]
fn naming_no_issues_audits_nothing() {
    let s = snapshot(vec![late_issue("KK-1")], vec![project("p").build()]);
    let report = run(&s, &only(&[]));
    assert!(report.findings.is_empty());
    assert!(report.unresolved_issues.is_empty());
}

#[test]
fn without_options_the_scoped_audit_is_the_full_audit() {
    let s = snapshot(vec![late_issue("KK-1")], vec![project("p").build()]);
    let report =
        audit_scoped(&s, &AuditConfig::default(), &AuditOptions::default(), now()).unwrap();
    assert_eq!(report, audit(&s, now()));
}

#[test]
fn since_without_issues_is_a_usage_error() {
    let s = snapshot(vec![], vec![]);
    let o = AuditOptions {
        since: Some(ts("2026-10-20T00:00:00Z")),
        issues: None,
    };
    let err = audit_scoped(&s, &AuditConfig::default(), &o, now()).unwrap_err();
    assert_eq!(err.code(), ErrorCode::Usage);
    assert!(err.to_string().contains("--since needs --issues"), "{err}");
}

#[test]
fn options_read_from_json() {
    let o: AuditOptions =
        serde_json::from_str(r#"{ "issues": ["KK-1"], "since": "2026-10-20T00:00:00Z" }"#).unwrap();
    assert_eq!(o, since(&["KK-1"], "2026-10-20T00:00:00Z"));
    assert_eq!(
        serde_json::from_str::<AuditOptions>("{}").unwrap(),
        AuditOptions::default()
    );
}

// -------------------------------------------------------------------- since

#[test]
fn an_issue_not_updated_since_the_time_is_reported() {
    let s = snapshot(
        vec![issue("KK-1").updated("2026-10-19T00:00:00Z").build()],
        vec![],
    );
    let report = run(&s, &since(&["KK-1"], "2026-10-19T00:00:01Z"));
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::NotUpdatedSince);
    assert_eq!(
        f.message,
        "KK-1 was last updated 2026-10-19T00:00:00Z, before 2026-10-19T00:00:01Z"
    );
    assert_eq!(f.fix, "linear issue update KK-1 --state <state> -w ken109");
}

#[test]
fn an_issue_updated_exactly_at_the_time_counts_as_updated() {
    let s = snapshot(
        vec![issue("KK-1").updated("2026-10-19T00:00:00Z").build()],
        vec![],
    );
    assert!(run(&s, &since(&["KK-1"], "2026-10-19T00:00:00Z"))
        .findings
        .is_empty());
    assert!(run(&s, &since(&["KK-1"], "2026-10-18T00:00:00Z"))
        .findings
        .is_empty());
}

#[test]
fn not_updated_since_is_always_actionable_whoever_the_assignee_is() {
    let issues = vec![
        issue("KK-1").mine().updated("2026-10-01T00:00:00Z").build(),
        issue("KK-2")
            .theirs()
            .updated("2026-10-01T00:00:00Z")
            .build(),
        issue("KK-3").updated("2026-10-01T00:00:00Z").build(),
    ];
    let s = snapshot(issues, vec![]);
    let report = run(
        &s,
        &since(&["KK-1", "KK-2", "KK-3"], "2026-10-10T00:00:00Z"),
    );
    assert_eq!(
        of_rule(&report.findings, RuleId::NotUpdatedSince),
        [
            ("KK-1".to_owned(), true),
            ("KK-2".to_owned(), true),
            ("KK-3".to_owned(), true)
        ]
    );
    assert!(report.has_actionable());
}

#[test]
fn only_the_named_issues_are_held_to_the_time() {
    let issues = vec![
        issue("KK-1").updated("2026-10-01T00:00:00Z").build(),
        issue("KK-2").updated("2026-10-01T00:00:00Z").build(),
    ];
    let report = run(
        &snapshot(issues, vec![]),
        &since(&["KK-1"], "2026-10-10T00:00:00Z"),
    );
    assert_eq!(of_rule(&report.findings, RuleId::NotUpdatedSince).len(), 1);
}

#[test]
fn without_since_nothing_is_checked_for_staleness_of_touch() {
    let s = snapshot(
        vec![issue("KK-1").updated("2026-10-01T00:00:00Z").build()],
        vec![],
    );
    assert!(of_rule(&run(&s, &only(&["KK-1"])).findings, RuleId::NotUpdatedSince).is_empty());
}

// --------------------------------------------------------------- validators

fn with_validators(rules: &[Rule]) -> AuditConfig {
    AuditConfig {
        validators: rules.to_vec(),
        ..AuditConfig::default()
    }
}

fn run_validators(s: &linear_core::audit::Snapshot, rules: &[Rule]) -> Vec<(RuleId, String, bool)> {
    linear_core::audit::audit(s, &with_validators(rules), now())
        .findings
        .into_iter()
        .filter(|f| {
            matches!(
                f.rule,
                RuleId::SourceAttachment | RuleId::LabelGroupsExclusive
            )
        })
        .map(|f| (f.rule, f.target.identifier, f.actionable))
        .collect()
}

#[test]
fn validators_run_only_when_enabled() {
    let s = snapshot(vec![issue("KK-1").build()], vec![]);
    assert!(run_validators(&s, &[]).is_empty());
    assert_eq!(
        run_validators(&s, &[Rule::SourceAttachment]),
        [(RuleId::SourceAttachment, "KK-1".to_owned(), false)]
    );
}

#[test]
fn source_attachment_applies_to_open_issues_that_lack_one() {
    let issues = vec![
        issue("KK-1").mine().build(),
        issue("KK-2").source("https://example.com/a").build(),
        issue("KK-3").source("ftp://example.com/a").build(),
        issue("KK-4").state("completed").build(),
    ];
    let got = run_validators(&snapshot(issues, vec![]), &[Rule::SourceAttachment]);
    assert_eq!(
        got,
        [
            (RuleId::SourceAttachment, "KK-1".to_owned(), true),
            (RuleId::SourceAttachment, "KK-3".to_owned(), false),
        ]
    );
}

#[test]
fn label_groups_exclusive_flags_two_labels_of_one_group() {
    let issues = vec![
        issue("KK-1")
            .labels(&[("api", "area"), ("web", "area")])
            .mine()
            .build(),
        issue("KK-2")
            .labels(&[("api", "area"), ("p1", "priority")])
            .build(),
    ];
    let report = linear_core::audit::audit(
        &snapshot(issues, vec![]),
        &with_validators(&[Rule::LabelGroupsExclusive]),
        now(),
    );
    assert_eq!(report.findings.len(), 1);
    let f = &report.findings[0];
    assert_eq!(f.rule, RuleId::LabelGroupsExclusive);
    assert!(f.actionable);
    assert!(
        f.message.contains("area") && f.message.contains("api, web"),
        "{}",
        f.message
    );
}

#[test]
fn several_violations_of_one_rule_on_one_issue_make_one_finding() {
    let i = issue("KK-1")
        .labels(&[
            ("api", "area"),
            ("web", "area"),
            ("p1", "priority"),
            ("p2", "priority"),
        ])
        .build();
    let got = run_validators(&snapshot(vec![i], vec![]), &[Rule::LabelGroupsExclusive]);
    assert_eq!(got.len(), 1);
}

#[test]
fn a_named_issue_is_validated_even_when_it_just_closed() {
    let issues = vec![
        issue("KK-1").state("completed").build(),
        issue("KK-2").state("completed").build(),
    ];
    let s = snapshot(issues, vec![]);
    let config = with_validators(&[Rule::SourceAttachment]);
    let report = audit_scoped(&s, &config, &only(&["KK-1"]), now()).unwrap();
    assert_eq!(
        of_rule(&report.findings, RuleId::SourceAttachment),
        [("KK-1".to_owned(), false)]
    );
}

#[test]
fn template_sections_without_templates_has_nothing_to_hold_an_issue_to() {
    let s = snapshot(vec![issue("KK-1").build()], vec![]);
    assert!(run_validators(&s, &[Rule::TemplateSections]).is_empty());
}

#[test]
fn validator_settings_read_from_json_with_kebab_case_names() {
    let c: AuditConfig = serde_json::from_str(
        r#"{ "validators": ["source-attachment", "label-groups-exclusive"] }"#,
    )
    .unwrap();
    assert_eq!(
        c.validators,
        [Rule::SourceAttachment, Rule::LabelGroupsExclusive]
    );
    assert_eq!(c.stale_days, 7);
}
