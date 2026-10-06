//! `template-sections` applied to existing issues. `now` is fixed (2026-10-20 12:00 UTC).
//!
//! An issue does not record its template, so the audit takes the one whose
//! sections the body shares most.

mod common;

use common::*;
use linear_core::audit::{audit, audit_scoped, AuditConfig, AuditOptions, RuleId, Snapshot};
use linear_core::config::Rule;
use linear_core::types::Template;
use serde_json::{json, Value};

fn heading(text: &str) -> Value {
    json!({ "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": text }] })
}

/// An issue template as Linear returns it: `templateData` is a JSON document
/// encoded in a string.
fn template(name: &str, kind: &str, team: Option<&str>, sections: &[&str]) -> Template {
    let content: Vec<Value> = sections.iter().map(|s| heading(s)).collect();
    let data = json!({
        "title": "",
        "descriptionData": { "type": "doc", "content": content },
    })
    .to_string();
    serde_json::from_value(json!({
        "id": format!("tpl-{name}"),
        "name": name,
        "description": null,
        "type": kind,
        "team": team.map(|id| json!({ "id": id, "key": "T", "name": "Team" })),
        "templateData": data,
        "updatedAt": "2026-10-06T00:00:00Z",
    }))
    .unwrap()
}

fn task() -> Template {
    template("Task", "issue", None, &["Goal", "Done when"])
}

fn bug() -> Template {
    template("Bug", "issue", None, &["Steps", "Expected"])
}

fn config() -> AuditConfig {
    AuditConfig {
        validators: vec![Rule::TemplateSections],
        ..AuditConfig::default()
    }
}

fn with(templates: Vec<Template>, issues: Vec<linear_core::types::Issue>) -> Snapshot {
    Snapshot {
        templates,
        ..snapshot(issues, vec![])
    }
}

/// `(identifier, actionable, message)` of the template-sections findings.
fn findings(s: &Snapshot) -> Vec<(String, bool, String)> {
    audit(s, &config(), now())
        .findings
        .into_iter()
        .filter(|f| f.rule == RuleId::TemplateSections)
        .map(|f| (f.target.identifier, f.actionable, f.message))
        .collect()
}

#[test]
fn a_filled_body_passes() {
    let body = "## Goal\nShip it.\n\n## Done when\nIt is shipped.\n";
    let s = with(vec![task()], vec![issue("KK-1").description(body).build()]);
    assert!(findings(&s).is_empty());
}

#[test]
fn a_missing_and_an_empty_section_are_both_reported() {
    let body = "## Goal\n\n## Notes\nSomething.\n";
    let s = with(
        vec![task()],
        vec![issue("KK-1").mine().description(body).build()],
    );
    assert_eq!(
        findings(&s),
        [(
            "KK-1".to_owned(),
            true,
            "KK-1 does not fill the template \"Task\": section \"Goal\" is empty; \
             section \"Done when\" is missing"
                .to_owned()
        )]
    );
}

#[test]
fn the_template_the_body_is_closest_to_is_used() {
    let bug_body = "## Steps\nRun it.\n\n## Expected\nNo crash.\n";
    let half_bug = "## Steps\nRun it.\n";
    let s = with(
        vec![task(), bug()],
        vec![
            issue("KK-1").description(bug_body).build(),
            issue("KK-2").description(half_bug).build(),
        ],
    );
    assert_eq!(
        findings(&s),
        [(
            "KK-2".to_owned(),
            false,
            "KK-2 does not fill the template \"Bug\": section \"Expected\" is missing".to_owned()
        )]
    );
}

#[test]
fn a_body_with_no_template_section_follows_no_template() {
    let s = with(
        vec![task(), bug()],
        vec![
            issue("KK-1").description("Just prose.").build(),
            issue("KK-2").build(),
        ],
    );
    let got = findings(&s);
    assert_eq!(got.len(), 2);
    assert_eq!(
        got[0].2,
        "KK-1 follows none of the issue templates (Bug, Task)"
    );
    assert_eq!(got[1].0, "KK-2");
}

#[test]
fn nothing_is_judged_without_issue_templates() {
    let s = with(
        vec![template("Kickoff", "project", None, &["Goal"])],
        vec![issue("KK-1").build()],
    );
    assert!(findings(&s).is_empty());
    assert!(findings(&with(vec![], vec![issue("KK-1").build()])).is_empty());
}

#[test]
fn a_template_of_another_team_does_not_apply() {
    let theirs = template("Theirs", "issue", Some("t-other"), &["Secret"]);
    let s = with(vec![theirs], vec![issue("KK-1").build()]);
    assert!(findings(&s).is_empty());

    let ours = template("Ours", "issue", Some("t-1"), &["Goal"]);
    let s = with(vec![ours], vec![issue("KK-1").build()]);
    assert_eq!(findings(&s).len(), 1);
}

#[test]
fn only_open_issues_are_checked_unless_named() {
    let issues = vec![
        issue("KK-1").state("completed").build(),
        issue("KK-2").state("completed").build(),
    ];
    let s = with(vec![task()], issues);
    assert!(findings(&s).is_empty());

    let options = AuditOptions {
        issues: Some(vec!["KK-2".to_owned()]),
        since: None,
    };
    let report = audit_scoped(&s, &config(), &options, now()).unwrap();
    let ids: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == RuleId::TemplateSections)
        .map(|f| f.target.identifier.as_str())
        .collect();
    assert_eq!(ids, ["KK-2"]);
}

#[test]
fn the_rule_runs_only_when_enabled() {
    let s = with(vec![task()], vec![issue("KK-1").build()]);
    let report = audit(&s, &AuditConfig::default(), now());
    assert!(report.findings.is_empty());
}

#[test]
fn the_finding_names_the_rule_and_a_fix_for_the_workspace() {
    let s = with(vec![task()], vec![issue("KK-1").build()]);
    let report = audit(&s, &config(), now());
    let f = &report.findings[0];
    assert_eq!(f.rule.as_str(), "template-sections");
    assert_eq!(
        f.fix,
        "linear issue update KK-1 --body-file <file> -w ken109"
    );
}

#[test]
fn a_snapshot_without_templates_still_reads_from_json() {
    let s = snapshot(vec![issue("KK-1").build()], vec![]);
    let mut v = serde_json::to_value(&s).unwrap();
    v.as_object_mut().unwrap().remove("templates");
    let back: Snapshot = serde_json::from_value(v).unwrap();
    assert!(back.templates.is_empty());
}
