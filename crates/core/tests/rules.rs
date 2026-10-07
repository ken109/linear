//! The validator engine: needs, checks, configuration and `audit` reuse.

use cynic::GraphQlResponse;
use linear_core::config::{Config, Rule};
use linear_core::metadata::AttachmentMetadata;
use linear_core::queries::{IssueById, Templates};
use linear_core::rules::template_sections::{self, SectionProblem};
use linear_core::rules::*;
use linear_core::types::{IssueRef, Label, Template};
use linear_core::ErrorCode;
use serde_json::json;
use std::collections::BTreeMap;

// ------------------------------------------------------------------ helpers

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let r: GraphQlResponse<T> = serde_json::from_str(&text).unwrap();
    r.data.expect("data")
}

fn doc(content: serde_json::Value) -> serde_json::Value {
    json!({ "type": "doc", "content": content })
}

fn heading(text: &str) -> serde_json::Value {
    json!({ "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": text }] })
}

fn paragraph(text: &str) -> serde_json::Value {
    json!({ "type": "paragraph", "content": [{ "type": "text", "text": text }] })
}

/// A template as Linear returns it: `templateData` is a JSON document encoded
/// in a string.
fn template(name: &str, kind: &str, description_data: serde_json::Value) -> Template {
    let data = json!({ "title": "", "descriptionData": description_data }).to_string();
    serde_json::from_value(json!({
        "id": format!("tpl-{name}"),
        "name": name,
        "description": null,
        "type": kind,
        "team": null,
        "templateData": data,
        "updatedAt": "2026-10-06T00:00:00Z",
    }))
    .unwrap()
}

fn research() -> Template {
    template(
        "Research",
        "issue",
        doc(json!([
            heading("Question"),
            paragraph("..."),
            heading("Done when"),
            paragraph("...")
        ])),
    )
}

fn project_template() -> Template {
    template(
        "Project",
        "project",
        doc(json!([heading("Definition of done")])),
    )
}

fn label(id: &str, name: &str, group: Option<(&str, &str, Option<&str>)>) -> Label {
    let parent = group.map(|(gid, gname, ty)| json!({ "id": gid, "name": gname, "groupType": ty }));
    serde_json::from_value(json!({
        "id": id, "name": name, "color": "#000000", "isGroup": false, "parent": parent,
    }))
    .unwrap()
}

fn issue_ref(identifier: &str) -> IssueRef {
    serde_json::from_value(json!({
        "id": format!("id-{identifier}"),
        "identifier": identifier,
        "url": format!("https://linear.app/example/issue/{identifier}"),
    }))
    .unwrap()
}

const GOOD_BODY: &str = "## Question\nWhy?\n\n## Done when\nWe know.\n";
const SOURCE: &str = "https://github.com/ken109/life/blob/main/decisions/x.md";

fn all_rules() -> RuleSet {
    RuleSet::new(&[
        Rule::TemplateSections,
        Rule::SourceAttachment,
        Rule::LabelGroupsExclusive,
    ])
}

fn fetched() -> Fetched {
    Fetched {
        templates: vec![research(), project_template()],
        existing_by_source: BTreeMap::new(),
    }
}

fn good_create() -> Draft {
    Draft::new(Operation::IssueCreate)
        .template("Research")
        .body(GOOD_BODY)
        .source(SOURCE)
}

fn kinds(r: &Rejected) -> Vec<(Rule, ViolationKind, Option<&str>)> {
    r.violations()
        .iter()
        .map(|v| (v.rule, v.kind, v.subject.as_deref()))
        .collect()
}

// ------------------------------------------------------- section matching

fn headings(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_filled_body_satisfies_every_section() {
    let h = headings(&["Question", "Done when"]);
    assert!(template_sections::missing_sections(GOOD_BODY, &h).is_empty());
}

#[test]
fn missing_and_empty_sections_are_told_apart() {
    let body = "## Question\n\n   \n## Done when\n";
    let h = headings(&["Question", "Done when", "Background"]);
    assert_eq!(
        template_sections::missing_sections(body, &h),
        vec![
            SectionProblem {
                section: "Question".into(),
                empty: true
            },
            SectionProblem {
                section: "Done when".into(),
                empty: true
            },
            SectionProblem {
                section: "Background".into(),
                empty: false
            },
        ]
    );
}

#[test]
fn a_section_runs_until_the_next_heading_of_any_level() {
    // "Question" has nothing of its own: the text belongs to the subsection.
    let body = "## Question\n### Detail\nsome text\n## Done when\nx\n";
    let h = headings(&["Question", "Done when"]);
    assert_eq!(
        template_sections::missing_sections(body, &h),
        vec![SectionProblem {
            section: "Question".into(),
            empty: true
        }]
    );
}

#[test]
fn heading_text_is_matched_exactly_and_must_be_a_heading_line() {
    // Same words in prose, or a different heading, do not count.
    let body = "Question\nsome text\n## Questions\ntext\n";
    let h = headings(&["Question"]);
    assert_eq!(
        template_sections::missing_sections(body, &h),
        vec![SectionProblem {
            section: "Question".into(),
            empty: false
        }]
    );
}

#[test]
fn headings_come_from_prosemirror_nodes_and_markdown_paragraphs() {
    let nodes = research();
    assert_eq!(
        template_sections::headings(&nodes).unwrap(),
        vec!["Question", "Done when"]
    );

    // The sandbox fixture was created through the API as markdown text.
    let d: Templates = fixture("templates");
    assert_eq!(
        template_sections::headings(&d.templates[0]).unwrap(),
        vec!["Goal"]
    );

    // A markdown string body (not a document at all).
    let md = template("Md", "issue", json!("## A\n\n## B\n"));
    assert_eq!(template_sections::headings(&md).unwrap(), vec!["A", "B"]);

    // No body at all.
    let mut none = research();
    none.template_data = json!({ "title": "" });
    assert!(template_sections::headings(&none).is_none());
}

// ------------------------------------------------------------------ phase 1

#[test]
fn needs_declares_the_template_and_the_source_lookup() {
    let needs = all_rules().needs(&good_create());
    assert_eq!(
        needs,
        Needs {
            templates: vec![TemplateNeed {
                name: "Research".into(),
                kind: TemplateKind::Issue
            }],
            source_urls: vec![SOURCE.into()],
        }
    );
}

#[test]
fn needs_is_empty_when_nothing_is_enabled_or_nothing_to_look_at() {
    assert!(RuleSet::default().needs(&good_create()).is_empty());
    // Only source-attachment: no template to read.
    let only_source = RuleSet::new(&[Rule::SourceAttachment]);
    assert!(only_source.needs(&good_create()).templates.is_empty());
    // An invalid source is not looked up (it is a violation).
    let bad = Draft::new(Operation::IssueCreate).source("ftp://x");
    assert!(only_source.needs(&bad).source_urls.is_empty());
}

#[test]
fn an_update_reads_a_template_only_when_it_replaces_the_body_under_one() {
    let rules = all_rules();
    let plain = Draft::new(Operation::IssueUpdate);
    assert!(rules.needs(&plain).is_empty());

    let template_only = Draft::new(Operation::IssueUpdate).template("Research");
    assert!(rules.needs(&template_only).is_empty());

    let both = Draft::new(Operation::IssueUpdate)
        .template("Research")
        .body(GOOD_BODY);
    assert_eq!(rules.needs(&both).templates.len(), 1);
    // Updates never look sources up.
    assert!(rules.needs(&both).source_urls.is_empty());
}

#[test]
fn project_operations_read_project_templates() {
    let draft = Draft::new(Operation::ProjectCreate)
        .template("Project")
        .body("## Definition of done\nx\n");
    let rules = all_rules();
    assert_eq!(
        rules.needs(&draft).templates,
        vec![TemplateNeed {
            name: "Project".into(),
            kind: TemplateKind::Project
        }]
    );
    assert_eq!(rules.check(&draft, &fetched()), Ok(Outcome::Proceed));

    // An issue template of the same name does not satisfy a project write.
    let only_issue = Fetched {
        templates: vec![template("Project", "issue", doc(json!([heading("X")])))],
        ..Fetched::default()
    };
    let err = rules.check(&draft, &only_issue).unwrap_err();
    assert_eq!(err.violations()[0].kind, ViolationKind::TemplateNotFound);
}

// ------------------------------------------------------------------ phase 2

#[test]
fn a_good_create_proceeds() {
    assert_eq!(
        all_rules().check(&good_create(), &fetched()),
        Ok(Outcome::Proceed)
    );
}

#[test]
fn every_violation_is_reported_together_in_rule_order() {
    let draft = Draft::new(Operation::IssueCreate)
        .template("Research")
        .body("## Question\n")
        .source("not a url")
        .labels(vec![
            label("l1", "api", Some(("g1", "area", Some("singleSelect")))),
            label("l2", "web", Some(("g1", "area", Some("singleSelect")))),
        ]);
    let err = all_rules().check(&draft, &fetched()).unwrap_err();
    assert_eq!(
        kinds(&err),
        vec![
            (
                Rule::TemplateSections,
                ViolationKind::SectionEmpty,
                Some("Question")
            ),
            (
                Rule::TemplateSections,
                ViolationKind::SectionMissing,
                Some("Done when")
            ),
            (
                Rule::SourceAttachment,
                ViolationKind::SourceInvalid,
                Some("not a url")
            ),
            (
                Rule::LabelGroupsExclusive,
                ViolationKind::LabelGroupConflict,
                Some("area")
            ),
        ]
    );
}

#[test]
fn a_rejection_maps_to_exit_code_5_and_lists_each_violation() {
    let err = all_rules()
        .check(&Draft::new(Operation::IssueCreate), &fetched())
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Validation);
    assert_eq!(err.code().exit_code(), 5);
    let text = err.to_string();
    assert!(text.starts_with("2 validation rules failed:"), "{text}");
    assert!(
        text.contains("[template-sections] a template is required"),
        "{text}"
    );
    assert!(
        text.contains("[source-attachment] a source URL is required"),
        "{text}"
    );
}

#[test]
fn a_create_without_a_template_or_body_is_rejected() {
    let rules = RuleSet::new(&[Rule::TemplateSections]);
    let err = rules
        .check(
            &Draft::new(Operation::IssueCreate).source(SOURCE),
            &fetched(),
        )
        .unwrap_err();
    assert_eq!(
        kinds(&err),
        vec![(
            Rule::TemplateSections,
            ViolationKind::TemplateRequired,
            None
        )]
    );

    // A template but no body: every section is missing.
    let err = rules
        .check(
            &Draft::new(Operation::IssueCreate).template("Research"),
            &fetched(),
        )
        .unwrap_err();
    assert_eq!(err.violations().len(), 2);
    assert!(err
        .violations()
        .iter()
        .all(|v| v.kind == ViolationKind::SectionMissing));
}

#[test]
fn an_unknown_template_lists_what_exists() {
    let rules = RuleSet::new(&[Rule::TemplateSections]);
    let draft = good_create().template("Nope");
    let err = rules.check(&draft, &fetched()).unwrap_err();
    let v = &err.violations()[0];
    assert_eq!(v.kind, ViolationKind::TemplateNotFound);
    assert!(v.message.contains("available: Research"), "{}", v.message);
}

#[test]
fn an_update_that_replaces_the_body_must_name_a_template() {
    let rules = all_rules();
    // The body is replaced and no template is named: refused, naming the flag.
    let plain = Draft::new(Operation::IssueUpdate).body("anything");
    let err = rules.check(&plain, &fetched()).unwrap_err();
    assert_eq!(
        kinds(&err),
        vec![(
            Rule::TemplateSections,
            ViolationKind::TemplateRequired,
            None
        )]
    );
    assert!(err.violations()[0].message.contains("--template"));
    assert_eq!(err.code(), ErrorCode::Validation);

    // The same for a project body.
    let project = Draft::new(Operation::ProjectUpdate).body("anything");
    assert!(rules.check(&project, &fetched()).is_err());

    // A blank template name is no template.
    let blank = Draft::new(Operation::IssueUpdate)
        .template("  ")
        .body("anything");
    assert!(rules.check(&blank, &fetched()).is_err());

    // An update that leaves the body alone has nothing to hold to a template.
    let no_body = Draft::new(Operation::IssueUpdate);
    assert_eq!(rules.check(&no_body, &fetched()), Ok(Outcome::Proceed));

    // The sections are judged against the named template.
    let bad = Draft::new(Operation::IssueUpdate)
        .template("Research")
        .body("x");
    assert!(rules.check(&bad, &fetched()).is_err());
    let good = Draft::new(Operation::IssueUpdate)
        .template("Research")
        .body(GOOD_BODY);
    assert_eq!(rules.check(&good, &fetched()), Ok(Outcome::Proceed));
}

#[test]
fn an_update_is_not_checked_when_rule_operations_leave_it_out() {
    let cfg = Config::parse(
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"template-sections\"]\n\
         [workspaces.w.rule_operations]\ntemplate-sections = [\"issue_create\"]\n",
    )
    .unwrap();
    let rules = RuleSet::from_workspace(cfg.get("w").unwrap());
    let issue = Draft::new(Operation::IssueUpdate).body("anything");
    let project = Draft::new(Operation::ProjectUpdate).body("anything");
    assert_eq!(rules.check(&issue, &fetched()), Ok(Outcome::Proceed));
    assert_eq!(rules.check(&project, &fetched()), Ok(Outcome::Proceed));
    // Creation is still held to a template.
    assert!(rules
        .check(&Draft::new(Operation::IssueCreate), &fetched())
        .is_err());
}

#[test]
fn an_existing_source_short_circuits_to_already_exists() {
    let mut f = fetched();
    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-7"));
    // Even a draft that would otherwise be rejected: nothing will be written.
    let draft = Draft::new(Operation::IssueCreate).source(SOURCE);
    match all_rules().check(&draft, &f) {
        Ok(Outcome::AlreadyExists(existing)) => assert_eq!(existing.identifier, "KK-7"),
        other => panic!("unexpected: {other:?}"),
    }
    // The URL is matched after trimming.
    let padded = Draft::new(Operation::IssueCreate).source(format!("  {SOURCE}\n"));
    assert!(matches!(
        all_rules().check(&padded, &f),
        Ok(Outcome::AlreadyExists(_))
    ));
}

#[test]
fn source_idempotence_does_nothing_when_the_rule_is_off() {
    let mut f = fetched();
    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-7"));
    let rules = RuleSet::new(&[Rule::TemplateSections]);
    assert_eq!(rules.check(&good_create(), &f), Ok(Outcome::Proceed));
}

#[test]
fn source_urls_must_be_http_or_https() {
    use linear_core::rules::source_attachment::is_http_url;
    assert!(is_http_url("https://example.com/a?b#c"));
    assert!(is_http_url("HTTP://example.com"));
    for bad in [
        "",
        "example.com",
        "ftp://example.com",
        "https://",
        "https:///x",
        "https://a b",
        "file:///etc/passwd",
    ] {
        assert!(!is_http_url(bad), "{bad}");
    }
}

#[test]
fn label_groups_are_exclusive_except_multi_select() {
    let rules = RuleSet::new(&[Rule::LabelGroupsExclusive]);
    let single = |id, name| label(id, name, Some(("g1", "area", Some("singleSelect"))));
    let multi = |id, name| label(id, name, Some(("g2", "tags", Some("multiSelect"))));
    let ungrouped = |id, name| label(id, name, None);
    let draft = |labels| Draft::new(Operation::IssueUpdate).labels(labels);

    // One per group, several ungrouped, several of a multi-select group: fine.
    let ok = vec![
        single("1", "api"),
        ungrouped("3", "Bug"),
        ungrouped("4", "Feature"),
        multi("5", "a"),
        multi("6", "b"),
    ];
    assert_eq!(rules.check(&draft(ok), &fetched()), Ok(Outcome::Proceed));

    // The same label twice is not a conflict.
    let twice = vec![single("1", "api"), single("1", "api")];
    assert_eq!(rules.check(&draft(twice), &fetched()), Ok(Outcome::Proceed));

    // Two of a single-select group, and a group with no declared type.
    let two = vec![single("1", "api"), single("2", "web")];
    let err = rules.check(&draft(two), &fetched()).unwrap_err();
    assert!(err.violations()[0].message.contains("api, web"), "{err}");

    let untyped = |id, name| label(id, name, Some(("g3", "kind", None)));
    let err = rules
        .check(
            &draft(vec![untyped("7", "x"), untyped("8", "y")]),
            &fetched(),
        )
        .unwrap_err();
    assert_eq!(err.violations()[0].subject.as_deref(), Some("kind"));

    // Labels left alone: nothing to check.
    assert_eq!(
        rules.check(&Draft::new(Operation::IssueUpdate), &fetched()),
        Ok(Outcome::Proceed)
    );
}

#[test]
fn rules_only_run_for_the_operations_they_support() {
    let rules = all_rules();
    // Project writes: source and label rules do not apply, so a project
    // create with no source and conflicting labels still passes them.
    let draft = Draft::new(Operation::ProjectUpdate).labels(vec![
        label("1", "a", Some(("g", "area", None))),
        label("2", "b", Some(("g", "area", None))),
    ]);
    assert_eq!(rules.check(&draft, &fetched()), Ok(Outcome::Proceed));
    assert!(!rules.applies(Rule::SourceAttachment, Operation::ProjectUpdate));
    assert!(rules.applies(Rule::SourceAttachment, Operation::IssueUpdate));
    assert!(rules.applies(Rule::TemplateSections, Operation::ProjectUpdate));
}

// ------------------------------------------------- source on an issue update

fn update_with_source() -> Draft {
    Draft::new(Operation::IssueUpdate)
        .issue("id-KK-1")
        .source(SOURCE)
}

#[test]
fn an_update_that_names_no_source_is_not_asked_for_one() {
    let rules = all_rules();
    let plain = Draft::new(Operation::IssueUpdate).issue("id-KK-1");
    assert_eq!(rules.check(&plain, &fetched()), Ok(Outcome::Proceed));
    assert!(rules.needs(&plain).source_urls.is_empty());
    // Naming one makes it be looked up.
    assert_eq!(
        rules.needs(&update_with_source()).source_urls,
        [SOURCE.to_owned()]
    );
}

#[test]
fn an_update_judges_the_source_url() {
    let rules = all_rules();
    let bad = Draft::new(Operation::IssueUpdate).source("ftp://example.com/x");
    let err = rules.check(&bad, &fetched()).unwrap_err();
    assert_eq!(
        kinds(&err),
        [(
            Rule::SourceAttachment,
            ViolationKind::SourceInvalid,
            Some("ftp://example.com/x")
        )]
    );
    // A new source on an issue that has none is fine, and nothing is "already there".
    assert_eq!(
        rules.check(&update_with_source(), &fetched()),
        Ok(Outcome::Proceed)
    );
}

#[test]
fn an_update_may_upsert_its_own_attachment_but_not_take_another_issues() {
    let rules = all_rules();
    let mut f = fetched();
    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-1"));
    // It is this issue's attachment: an upsert, never "already exists".
    assert_eq!(rules.check(&update_with_source(), &f), Ok(Outcome::Proceed));

    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-7"));
    let err = rules.check(&update_with_source(), &f).unwrap_err();
    assert_eq!(
        kinds(&err),
        [(
            Rule::SourceAttachment,
            ViolationKind::SourceTaken,
            Some(SOURCE)
        )]
    );
    assert!(
        err.to_string().contains("already attached to KK-7"),
        "{err}"
    );
}

#[test]
fn an_update_judges_the_kind_of_what_it_writes() {
    let rules = RuleSet::new(&[Rule::SourceAttachment])
        .source_kinds(vec!["slack".to_owned(), "github".to_owned()]);
    let with_kind = |kind: &str| {
        update_with_source()
            .source_metadata(AttachmentMetadata::from_pairs(&[format!("kind={kind}")]).unwrap())
    };
    assert_eq!(
        rules.check(&with_kind("slack"), &fetched()),
        Ok(Outcome::Proceed)
    );
    let err = rules.check(&with_kind("email"), &fetched()).unwrap_err();
    assert_eq!(err.violations()[0].kind, ViolationKind::SourceKindInvalid);
    // A new attachment with no kind at all.
    let err = rules.check(&update_with_source(), &fetched()).unwrap_err();
    assert_eq!(err.violations()[0].kind, ViolationKind::SourceKindInvalid);

    // The issue's own attachment, left as it is: nothing is written, nothing is judged.
    let mut f = fetched();
    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-1"));
    assert_eq!(rules.check(&update_with_source(), &f), Ok(Outcome::Proceed));
    // ... unless metadata is written onto it.
    assert!(rules.check(&with_kind("email"), &f).is_err());
}

#[test]
fn the_source_rule_can_be_narrowed_to_creation_in_the_configuration() {
    let cfg = Config::parse(
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"source-attachment\"]\n\
         [workspaces.w.rule_operations]\nsource-attachment = [\"issue_create\"]\n",
    )
    .unwrap();
    let rules = RuleSet::from_workspace(cfg.get("w").unwrap());
    assert!(rules.applies(Rule::SourceAttachment, Operation::IssueCreate));
    assert!(!rules.applies(Rule::SourceAttachment, Operation::IssueUpdate));
    let bad = Draft::new(Operation::IssueUpdate).source("ftp://example.com/x");
    assert_eq!(rules.check(&bad, &fetched()), Ok(Outcome::Proceed));
}

// ------------------------------------------------------------ configuration

fn workspace_toml(extra: &str) -> String {
    format!(
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"template-sections\", \"label-groups-exclusive\"]\n{extra}"
    )
}

#[test]
fn rules_can_be_narrowed_to_operations_in_the_configuration() {
    let cfg = Config::parse(&workspace_toml(
        "[workspaces.w.rule_operations]\nlabel-groups-exclusive = [\"issue_create\"]\n",
    ))
    .unwrap();
    let rules = RuleSet::from_workspace(cfg.get("w").unwrap());
    assert!(rules.applies(Rule::LabelGroupsExclusive, Operation::IssueCreate));
    assert!(!rules.applies(Rule::LabelGroupsExclusive, Operation::IssueUpdate));
    // Not narrowed: all four.
    for op in Operation::ALL {
        assert!(rules.applies(Rule::TemplateSections, op), "{op}");
    }
    // Disabled rules never apply.
    assert!(!rules.applies(Rule::SourceAttachment, Operation::IssueCreate));
}

#[test]
fn naming_an_operation_a_rule_cannot_use_is_a_load_time_error() {
    let err = Config::parse(&workspace_toml(
        "[workspaces.w.rule_operations]\nlabel-groups-exclusive = [\"project_create\"]\n",
    ))
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("label-groups-exclusive cannot apply to project_create"),
        "{err}"
    );
    assert!(err.contains("issue_create, issue_update"), "{err}");
}

#[test]
fn bad_rule_operations_are_load_time_errors() {
    for (extra, hint) in [
        // A rule that is not enabled.
        (
            "[workspaces.w.rule_operations]\nsource-attachment = [\"issue_create\"]\n",
            "not in rules",
        ),
        // An unknown rule name.
        (
            "[workspaces.w.rule_operations]\nno-such-rule = [\"issue_create\"]\n",
            "no-such-rule",
        ),
        // An unknown operation name.
        (
            "[workspaces.w.rule_operations]\ntemplate-sections = [\"issue_delete\"]\n",
            "issue_delete",
        ),
        // An empty list would silently disable the rule.
        (
            "[workspaces.w.rule_operations]\ntemplate-sections = []\n",
            "is empty",
        ),
    ] {
        let err = Config::parse(&workspace_toml(extra))
            .unwrap_err()
            .to_string();
        assert!(err.contains(hint), "{extra}: {err}");
    }
}

#[test]
fn rule_operations_round_trip_and_default_to_absent() {
    let cfg = Config::parse(&workspace_toml("")).unwrap();
    assert!(cfg.get("w").unwrap().rule_operations.is_empty());
    let text = toml_edit::ser::to_string(&cfg).unwrap();
    assert!(!text.contains("rule_operations"), "{text}");
}

// -------------------------------------------------------------------- audit

#[test]
fn audit_runs_the_same_rules_on_an_existing_issue() {
    let d: IssueById = fixture("issue");
    let issue = d.issue;
    assert!(all_rules().audit_issue(&issue).is_empty());

    // Two labels of the "area" group, and no source.
    let mut bad = issue.clone();
    let mut second = bad.labels[0].clone();
    second.id = cynic::Id::new("other");
    second.name = "web".into();
    bad.labels.nodes.push(second);
    bad.attachments.nodes.clear();

    let found = all_rules().audit_issue(&bad);
    let got: Vec<_> = found.iter().map(|v| (v.rule, v.kind)).collect();
    assert_eq!(
        got,
        vec![
            (Rule::SourceAttachment, ViolationKind::SourceRequired),
            (
                Rule::LabelGroupsExclusive,
                ViolationKind::LabelGroupConflict
            ),
        ]
    );
    assert_eq!(found[0].subject.as_deref(), Some("EX-23"));

    // Rules that are off do not report.
    assert!(RuleSet::default().audit_issue(&bad).is_empty());
}

// ------------------------------------------------------- source kinds

fn kind_rules() -> RuleSet {
    RuleSet::new(&[Rule::SourceAttachment]).source_kinds(vec![
        "life-decision".into(),
        "slack".into(),
        "repo".into(),
    ])
}

fn with_kind(kind: &str) -> Draft {
    Draft::new(Operation::IssueCreate)
        .source(SOURCE)
        .source_metadata(AttachmentMetadata::from_pairs(&[format!("kind={kind}")]).unwrap())
}

#[test]
fn a_source_kind_from_the_list_passes() {
    for kind in ["life-decision", "slack", "repo"] {
        assert_eq!(
            kind_rules().check(&with_kind(kind), &fetched()),
            Ok(Outcome::Proceed),
            "{kind}"
        );
    }
}

#[test]
fn a_source_kind_outside_the_list_is_rejected_naming_the_source() {
    let err = kind_rules()
        .check(&with_kind("email"), &fetched())
        .unwrap_err();
    assert_eq!(
        kinds(&err),
        vec![(
            Rule::SourceAttachment,
            ViolationKind::SourceKindInvalid,
            Some(SOURCE)
        )]
    );
    let text = err.to_string();
    assert!(text.contains("\"email\""), "{text}");
    assert!(text.contains("life-decision, slack, repo"), "{text}");
}

#[test]
fn a_missing_or_non_string_kind_is_rejected() {
    let no_meta = Draft::new(Operation::IssueCreate).source(SOURCE);
    let other_key = Draft::new(Operation::IssueCreate)
        .source(SOURCE)
        .source_metadata(AttachmentMetadata::from_pairs(&["ticket=7"]).unwrap());
    let number = Draft::new(Operation::IssueCreate)
        .source(SOURCE)
        .source_metadata(AttachmentMetadata::from_pairs(&["kind=7"]).unwrap());
    for draft in [no_meta, other_key, number] {
        let err = kind_rules().check(&draft, &fetched()).unwrap_err();
        assert_eq!(
            kinds(&err),
            vec![(
                Rule::SourceAttachment,
                ViolationKind::SourceKindInvalid,
                Some(SOURCE)
            )],
            "{draft:?}"
        );
    }
}

#[test]
fn without_source_kinds_the_metadata_is_not_looked_at() {
    let rules = RuleSet::new(&[Rule::SourceAttachment]);
    assert_eq!(
        rules.check(&with_kind("anything"), &fetched()),
        Ok(Outcome::Proceed)
    );
    let no_meta = Draft::new(Operation::IssueCreate).source(SOURCE);
    assert_eq!(rules.check(&no_meta, &fetched()), Ok(Outcome::Proceed));
    // An empty list is the same as none.
    let empty = RuleSet::new(&[Rule::SourceAttachment]).source_kinds(vec![]);
    assert_eq!(empty.check(&no_meta, &fetched()), Ok(Outcome::Proceed));
}

#[test]
fn an_existing_source_stays_existing_unless_new_metadata_has_a_bad_kind() {
    let mut f = fetched();
    f.existing_by_source
        .insert(SOURCE.into(), issue_ref("KK-7"));
    // Nothing is written without metadata, so the kind is not judged.
    let plain = Draft::new(Operation::IssueCreate).source(SOURCE);
    assert!(matches!(
        kind_rules().check(&plain, &f),
        Ok(Outcome::AlreadyExists(_))
    ));
    // A good kind goes through to the update.
    assert!(matches!(
        kind_rules().check(&with_kind("slack"), &f),
        Ok(Outcome::AlreadyExists(_))
    ));
    // A bad kind would be written onto the attachment: refused.
    let err = kind_rules().check(&with_kind("email"), &f).unwrap_err();
    assert_eq!(err.violations()[0].kind, ViolationKind::SourceKindInvalid);
}

fn issue_with_attachments(attachments: serde_json::Value) -> linear_core::types::Issue {
    let d: IssueById = fixture("issue");
    let mut issue = serde_json::to_value(d.issue).unwrap();
    issue["attachments"] = json!({ "nodes": attachments });
    serde_json::from_value(issue).unwrap()
}

fn attachment(url: &str, metadata: serde_json::Value) -> serde_json::Value {
    json!({
        "id": format!("a-{url}"), "title": "Source", "subtitle": null, "url": url,
        "sourceType": null, "metadata": metadata, "createdAt": "2026-09-01T00:00:00Z",
    })
}

#[test]
fn audit_flags_an_issue_whose_source_has_no_allowed_kind() {
    let rules = kind_rules();
    let good = issue_with_attachments(json!([attachment(SOURCE, json!({ "kind": "slack" }))]));
    assert!(rules.audit_issue(&good).is_empty());

    for metadata in [
        json!({}),
        json!({ "kind": "email" }),
        json!({ "kind": 3 }),
        json!({ "kind": { "a": 1 } }),
    ] {
        let bad = issue_with_attachments(json!([attachment(SOURCE, metadata.clone())]));
        let found = rules.audit_issue(&bad);
        assert_eq!(found.len(), 1, "{metadata}");
        assert_eq!(found[0].kind, ViolationKind::SourceKindInvalid);
        assert_eq!(found[0].subject.as_deref(), Some("EX-23"));
        assert!(found[0].message.contains("EX-23"), "{}", found[0].message);
        assert!(found[0].message.contains("life-decision, slack, repo"));
    }
}

#[test]
fn audit_accepts_any_http_attachment_with_an_allowed_kind() {
    let rules = kind_rules();
    let issue = issue_with_attachments(json!([
        attachment("https://example.com/pr/1", json!({ "nested": { "a": 1 } })),
        attachment(SOURCE, json!({ "kind": "life-decision" })),
    ]));
    assert!(rules.audit_issue(&issue).is_empty());
    // An attachment that is not http(s) never counts, whatever its kind.
    let ftp = issue_with_attachments(json!([attachment("ftp://x/y", json!({ "kind": "slack" }))]));
    let found = rules.audit_issue(&ftp);
    assert_eq!(found[0].kind, ViolationKind::SourceRequired);
}

#[test]
fn audit_without_source_kinds_ignores_metadata() {
    let rules = RuleSet::new(&[Rule::SourceAttachment]);
    let issue = issue_with_attachments(json!([attachment(SOURCE, json!({ "kind": "email" }))]));
    assert!(rules.audit_issue(&issue).is_empty());
}

#[test]
fn the_metadata_update_is_needed_only_when_given_and_different() {
    use linear_core::rules::source_attachment::metadata_needs_update;
    let stored = serde_json::Map::from_iter([("kind".to_owned(), json!("slack"))]);
    let same = AttachmentMetadata::from_pairs(&["kind=slack"]).unwrap();
    let other = AttachmentMetadata::from_pairs(&["kind=repo"]).unwrap();
    assert!(!metadata_needs_update(None, &stored));
    assert!(!metadata_needs_update(Some(&same), &stored));
    assert!(metadata_needs_update(Some(&other), &stored));
    assert!(metadata_needs_update(Some(&same), &serde_json::Map::new()));
}
