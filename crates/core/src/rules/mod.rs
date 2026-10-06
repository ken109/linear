//! The declarative validator engine.
//!
//! A workspace enables validator [`Rule`]s in `workspaces.toml`; the engine is
//! pure and works in two phases so that checking needs no I/O and tests need
//! no mocked fetching:
//!
//! 1. [`RuleSet::needs`] looks at what the caller is about to write (a
//!    [`Draft`]) and declares which data the enabled rules need ([`Needs`]):
//!    templates to read from Linear, source URLs to look up.
//! 2. The caller fetches that data into a [`Fetched`], then [`RuleSet::check`]
//!    returns every violation at once, or says the write is already done.
//!
//! Rules never run scripts or external commands: the set of rule kinds is the
//! [`Rule`] enum and a new kind is a new variant plus a module here.
//!
//! The per-rule functions in [`label_groups`], [`source_attachment`] and
//! [`template_sections`] are public so `audit` can run the same checks on
//! existing issues ([`RuleSet::audit_issue`]).

pub mod label_groups;
pub mod source_attachment;
pub mod template_sections;

use crate::config::{Rule, WorkspaceConfig};
use crate::error::ErrorCode;
use crate::types::{Issue, IssueRef, Label, Template};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A write the rules apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    IssueCreate,
    IssueUpdate,
    ProjectCreate,
    ProjectUpdate,
}

impl Operation {
    pub const ALL: [Operation; 4] = [
        Self::IssueCreate,
        Self::IssueUpdate,
        Self::ProjectCreate,
        Self::ProjectUpdate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::IssueCreate => "issue_create",
            Self::IssueUpdate => "issue_update",
            Self::ProjectCreate => "project_create",
            Self::ProjectUpdate => "project_update",
        }
    }

    pub fn is_create(self) -> bool {
        matches!(self, Self::IssueCreate | Self::ProjectCreate)
    }

    /// The kind of Linear template this operation's body is checked against.
    pub fn template_kind(self) -> TemplateKind {
        match self {
            Self::IssueCreate | Self::IssueUpdate => TemplateKind::Issue,
            Self::ProjectCreate | Self::ProjectUpdate => TemplateKind::Project,
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Rule {
    /// The operations this rule can be applied to. Naming any other operation
    /// for it in the configuration is an error at load time.
    pub fn operations(self) -> &'static [Operation] {
        match self {
            Self::TemplateSections => &Operation::ALL,
            Self::SourceAttachment => &[Operation::IssueCreate],
            Self::LabelGroupsExclusive => &[Operation::IssueCreate, Operation::IssueUpdate],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TemplateSections => "template-sections",
            Self::SourceAttachment => "source-attachment",
            Self::LabelGroupsExclusive => "label-groups-exclusive",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which family of Linear templates a template is looked up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    Issue,
    Project,
}

impl TemplateKind {
    /// The `type` Linear reports for templates of this kind.
    pub fn linear_type(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Project => "project",
        }
    }
}

/// The enabled rules and the operations each applies to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    rules: BTreeMap<Rule, BTreeSet<Operation>>,
}

impl RuleSet {
    /// Every rule applies to all the operations it supports.
    pub fn new(rules: &[Rule]) -> Self {
        Self {
            rules: rules
                .iter()
                .map(|r| (*r, r.operations().iter().copied().collect()))
                .collect(),
        }
    }

    /// Build from a workspace's configuration, honouring `rule_operations`.
    /// A configuration that passed [`crate::config::Config::validate`] never
    /// names an unsupported operation; if one slips through it is dropped
    /// here rather than widened.
    pub fn from_workspace(cfg: &WorkspaceConfig) -> Self {
        let mut set = Self::new(&cfg.rules);
        for (rule, ops) in &cfg.rule_operations {
            if let Some(slot) = set.rules.get_mut(rule) {
                *slot = ops
                    .iter()
                    .copied()
                    .filter(|o| rule.operations().contains(o))
                    .collect();
            }
        }
        set
    }

    /// `true` when no rule runs for any operation.
    pub fn is_empty(&self) -> bool {
        self.rules.values().all(BTreeSet::is_empty)
    }

    /// Does `rule` run for `op`?
    pub fn applies(&self, rule: Rule, op: Operation) -> bool {
        self.rules.get(&rule).is_some_and(|ops| ops.contains(&op))
    }

    /// Phase 1: what must be fetched before [`RuleSet::check`] can run.
    pub fn needs(&self, draft: &Draft) -> Needs {
        let mut needs = Needs::default();
        let op = draft.operation;
        if self.applies(Rule::TemplateSections, op) {
            if let Some(name) = template_sections::template_to_check(draft) {
                needs.templates.push(TemplateNeed {
                    name: name.to_owned(),
                    kind: op.template_kind(),
                });
            }
        }
        if self.applies(Rule::SourceAttachment, op) {
            if let Ok(url) = source_attachment::validate(draft.source.as_deref()) {
                needs.source_urls.push(url.to_owned());
            }
        }
        needs
    }

    /// Phase 2: check the draft against the fetched data.
    ///
    /// Everything is checked before anything is written, and all violations
    /// are returned together. When source idempotence finds that the issue
    /// already exists the result is [`Outcome::AlreadyExists`], which takes
    /// precedence over other findings: nothing will be written.
    pub fn check(&self, draft: &Draft, fetched: &Fetched) -> Result<Outcome, Rejected> {
        let op = draft.operation;
        let mut violations = Vec::new();

        for rule in self.rules.keys().copied().filter(|r| self.applies(*r, op)) {
            match rule {
                Rule::TemplateSections => {
                    violations.extend(template_sections::check(draft, fetched));
                }
                Rule::SourceAttachment => match source_attachment::check(draft, fetched) {
                    source_attachment::Check::Existing(issue) => {
                        return Ok(Outcome::AlreadyExists(issue));
                    }
                    source_attachment::Check::Violations(v) => violations.extend(v),
                },
                Rule::LabelGroupsExclusive => {
                    if let Some(labels) = &draft.labels {
                        violations.extend(label_groups::check(labels));
                    }
                }
            }
        }

        match Rejected::new(violations) {
            Some(rejected) => Err(rejected),
            None => Ok(Outcome::Proceed),
        }
    }

    /// Run the enabled rules that can be judged from an existing issue alone:
    /// `label-groups-exclusive`, and `source-attachment` (the issue must carry
    /// an http(s) source attachment).
    ///
    /// `template-sections` needs the issue body and the template it was
    /// written from, neither of which an [`Issue`] carries; `audit` can call
    /// [`template_sections::missing_sections`] itself when it has both.
    pub fn audit_issue(&self, issue: &Issue) -> Vec<Violation> {
        let enabled = |rule: Rule| self.rules.get(&rule).is_some_and(|ops| !ops.is_empty());
        let mut violations = Vec::new();
        if enabled(Rule::SourceAttachment) {
            violations.extend(source_attachment::check_existing(issue));
        }
        if enabled(Rule::LabelGroupsExclusive) {
            violations.extend(label_groups::check(&issue.labels));
        }
        violations
    }
}

/// What the caller is about to write, reduced to what rules look at.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Draft {
    pub operation: Operation,
    /// The template the body is meant to follow (`--template`).
    pub template: Option<String>,
    /// The new body (issue description / project content). `None` on an
    /// update that leaves the body alone.
    pub body: Option<String>,
    /// The origin URL (`--source`).
    pub source: Option<String>,
    /// The labels the issue will have, with their groups resolved. `None`
    /// when the write leaves labels alone.
    pub labels: Option<Vec<Label>>,
}

impl Draft {
    pub fn new(operation: Operation) -> Self {
        Self {
            operation,
            template: None,
            body: None,
            source: None,
            labels: None,
        }
    }

    pub fn template(mut self, name: impl Into<String>) -> Self {
        self.template = Some(name.into());
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn source(mut self, url: impl Into<String>) -> Self {
        self.source = Some(url.into());
        self
    }

    pub fn labels(mut self, labels: Vec<Label>) -> Self {
        self.labels = Some(labels);
        self
    }
}

/// A template to read from Linear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateNeed {
    pub name: String,
    pub kind: TemplateKind,
}

/// Phase 1 output: the data the caller must fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Needs {
    /// Templates to read (by name and kind).
    pub templates: Vec<TemplateNeed>,
    /// Source URLs to look up: the issue that carries an attachment with each
    /// URL, if any.
    pub source_urls: Vec<String>,
}

impl Needs {
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty() && self.source_urls.is_empty()
    }
}

/// Phase 2 input: what the caller fetched for a [`Needs`].
///
/// The contract is "fetch everything [`Needs`] named": a template or source
/// URL that is absent here is taken to not exist in Linear.
#[derive(Debug, Clone, Default)]
pub struct Fetched {
    /// Templates of the needed kinds (the engine picks by name and kind).
    pub templates: Vec<Template>,
    /// For each looked-up source URL, the issue that already carries it.
    /// URLs with no issue are simply absent.
    pub existing_by_source: BTreeMap<String, IssueRef>,
}

/// The result of a successful check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing is wrong; go ahead and write.
    Proceed,
    /// An issue with the same source already exists; do not create another,
    /// report this one instead.
    AlreadyExists(IssueRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// The rule needs a template name and none was given.
    TemplateRequired,
    /// No template with that name exists in Linear.
    TemplateNotFound,
    /// The template exists but its body could not be read.
    TemplateUnreadable,
    /// The body has no heading for a section of the template.
    SectionMissing,
    /// The body has the heading but nothing under it.
    SectionEmpty,
    /// The rule needs a source URL and none was given.
    SourceRequired,
    /// The source is not an http(s) URL.
    SourceInvalid,
    /// More than one label of a single-select group.
    LabelGroupConflict,
}

/// One broken rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub rule: Rule,
    pub kind: ViolationKind,
    /// What it is about: the section, template, URL or label group.
    pub subject: Option<String>,
    pub message: String,
}

impl Violation {
    pub(crate) fn new(
        rule: Rule,
        kind: ViolationKind,
        subject: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            kind,
            subject,
            message: message.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)
    }
}

/// A write refused by the validators: always at least one violation. Maps to
/// exit code 5 ([`ErrorCode::Validation`]).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}", self.summary())]
pub struct Rejected {
    violations: Vec<Violation>,
}

impl Rejected {
    /// `None` when there is nothing to reject.
    pub fn new(violations: Vec<Violation>) -> Option<Self> {
        if violations.is_empty() {
            None
        } else {
            Some(Self { violations })
        }
    }

    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    pub fn into_violations(self) -> Vec<Violation> {
        self.violations
    }

    pub fn code(&self) -> ErrorCode {
        ErrorCode::Validation
    }

    fn summary(&self) -> String {
        let n = self.violations.len();
        let mut s = format!(
            "{n} validation {} failed:",
            if n == 1 { "rule" } else { "rules" }
        );
        for v in &self.violations {
            s.push_str("\n  - ");
            s.push_str(&v.to_string());
        }
        s
    }
}
