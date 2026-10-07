//! `audit`: rules that find Linear data that has drifted.
//!
//! Everything here is a pure function of a [`Snapshot`] and a `now` handed in
//! by the caller (this crate never reads the clock), so the same rules run in
//! the CLI and, through WebAssembly, in a Worker. Rules decide on workflow
//! state *types*, never on names, which teams may rename.

mod consistency;
mod diff;
mod finding;
mod pull_requests;
mod scope;
mod stale;
mod validators;

use crate::config::{Rule, WorkspaceConfig};
use crate::error::Result;
use crate::rules::RuleSet;
use crate::types::{Issue, Project, ProjectStatusType, Template};
use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use diff::diff;
pub use finding::{AuditReport, Finding, FindingKey, RuleId, Severity, Target, TargetKind};
pub use scope::AuditOptions;

/// Everything the audit looks at, for one workspace.
///
/// `issues` should hold the open issues worth auditing plus any issue whose
/// state changed recently; the rules only see what is here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub workspace: String,
    pub issues: Vec<Issue>,
    pub projects: Vec<Project>,
    /// The workspace's issue templates, for `template-sections`. Only needed
    /// when that rule is enabled.
    #[serde(default)]
    pub templates: Vec<Template>,
}

/// Audit settings, per workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditConfig {
    /// `stale-in-progress`: an In Progress issue with no update for this many
    /// days is stale.
    pub stale_days: u32,
    /// `status-update-outdated`: a project's latest status update this many
    /// days old is outdated.
    pub status_update_days: u32,
    /// `pr-open-too-long`: a GitHub pull request linked to an issue that has
    /// been open (not a draft) this many days is flagged.
    pub pr_open_days: u32,
    /// Validator rules to apply to existing issues: the workspace's enabled
    /// rules. `template-sections` needs [`Snapshot::templates`]; an issue does
    /// not record which template it came from, so the closest one is used.
    pub validators: Vec<Rule>,
    /// `source-attachment`: the values `metadata.kind` of a source attachment
    /// may take. Empty: the kind is not checked.
    pub source_kinds: Vec<String>,
}

impl AuditConfig {
    /// The audit settings of a workspace: its `[audit]` thresholds over the
    /// defaults, and every validator rule it enables.
    pub fn from_workspace(workspace: &WorkspaceConfig) -> Self {
        let default = Self::default();
        Self {
            stale_days: workspace.audit.stale_days.unwrap_or(default.stale_days),
            status_update_days: workspace
                .audit
                .status_update_days
                .unwrap_or(default.status_update_days),
            pr_open_days: workspace.audit.pr_open_days.unwrap_or(default.pr_open_days),
            validators: workspace.rules.clone(),
            source_kinds: workspace.source_kinds.clone(),
        }
    }
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            stale_days: 7,
            status_update_days: 14,
            pr_open_days: 14,
            validators: Vec::new(),
            source_kinds: Vec::new(),
        }
    }
}

/// Run every rule over `snapshot`.
///
/// `now` decides what is overdue or stale; target dates are compared as
/// calendar dates in UTC, ages in whole days since the timestamp.
pub fn audit(snapshot: &Snapshot, config: &AuditConfig, now: DateTime<Utc>) -> AuditReport {
    run(snapshot, config, now, None, Vec::new())
}

/// Like [`audit`], narrowed to the issues the caller names.
///
/// With `options.issues`, only findings about those issues are returned, plus
/// findings about the projects (and their milestones) those issues belong to,
/// since changing an issue is what makes its project's state or status update
/// wrong. Identifiers that the snapshot does not contain are listed in
/// [`AuditReport::unresolved_issues`] rather than dropped.
///
/// With `options.since` as well, each named issue not updated at or after that
/// time yields a `not-updated-since` finding, always actionable: the caller
/// said it worked on the issue.
///
/// `since` without `issues` is a usage error: it has no issues to be about.
pub fn audit_scoped(
    snapshot: &Snapshot,
    config: &AuditConfig,
    options: &AuditOptions,
    now: DateTime<Utc>,
) -> Result<AuditReport> {
    match scope::resolve(snapshot, options)? {
        None => Ok(audit(snapshot, config, now)),
        Some((scope, unresolved)) => Ok(run(snapshot, config, now, Some(scope), unresolved)),
    }
}

fn run(
    snapshot: &Snapshot,
    config: &AuditConfig,
    now: DateTime<Utc>,
    scope: Option<scope::Scope>,
    unresolved_issues: Vec<String>,
) -> AuditReport {
    let ctx = Ctx::new(snapshot, config, now, scope);
    let mut findings = Vec::new();
    consistency::run(&ctx, &mut findings);
    stale::run(&ctx, &mut findings);
    pull_requests::run(&ctx, &mut findings);
    validators::run(&ctx, &mut findings);
    scope::run(&ctx, &mut findings);
    findings.sort_by(|a, b| {
        (a.rule, a.target.kind, &a.target.identifier).cmp(&(
            b.rule,
            b.target.kind,
            &b.target.identifier,
        ))
    });
    AuditReport {
        findings,
        unresolved_issues,
    }
}

/// What the rules share.
pub(crate) struct Ctx<'a> {
    pub snapshot: &'a Snapshot,
    pub config: &'a AuditConfig,
    pub now: DateTime<Utc>,
    pub today: NaiveDate,
    pub rules: RuleSet,
    pub scope: Option<scope::Scope<'a>>,
    projects: HashMap<&'a str, &'a Project>,
}

impl<'a> Ctx<'a> {
    fn new(
        snapshot: &'a Snapshot,
        config: &'a AuditConfig,
        now: DateTime<Utc>,
        scope: Option<scope::Scope<'a>>,
    ) -> Self {
        Self {
            snapshot,
            config,
            now,
            today: now.date_naive(),
            rules: RuleSet::new(&config.validators).source_kinds(config.source_kinds.clone()),
            scope,
            projects: snapshot
                .projects
                .iter()
                .map(|p| (p.id.inner(), p))
                .collect(),
        }
    }

    /// The project an issue belongs to, if the snapshot has it.
    pub fn project_of(&self, issue: &Issue) -> Option<&'a Project> {
        let id = issue.project.as_ref()?.id.inner();
        self.projects.get(id).copied()
    }

    /// Is the issue among those the audit was narrowed to (always, if it was not)?
    pub fn issue_in_scope(&self, issue: &Issue) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|s| s.has_issue(issue.id.inner()))
    }

    /// Is the project one the narrowed issues belong to (always, if not narrowed)?
    pub fn project_in_scope(&self, project: &Project) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|s| s.has_project(project.id.inner()))
    }

    pub fn workspace(&self) -> &str {
        &self.snapshot.workspace
    }

    /// Build a finding for this workspace. `fix` is the command without the
    /// workspace flag, which is added here.
    pub fn finding(
        &self,
        rule: RuleId,
        severity: Severity,
        target: Target,
        actionable: bool,
        message: String,
        fix: String,
    ) -> Finding {
        Finding {
            rule,
            severity,
            workspace: self.workspace().to_owned(),
            target,
            message,
            actionable,
            fix: format!("{fix} -w {}", self.workspace()),
        }
    }
}

/// Completed or canceled. A project in any other status is still live.
pub(crate) fn project_is_closed(p: &Project) -> bool {
    matches!(
        p.status.type_,
        ProjectStatusType::Completed | ProjectStatusType::Canceled
    )
}

/// Not completed, canceled or duplicate (so triage and backlog count as open).
pub(crate) fn issue_is_open(i: &Issue) -> bool {
    !i.state.state_type().is_closed()
}

/// Ownership: the viewer owns an issue it is assigned and a project it leads.
pub(crate) fn owns_issue(i: &Issue) -> bool {
    i.assignee.as_ref().is_some_and(|u| u.is_me)
}

pub(crate) fn owns_project(p: &Project) -> bool {
    p.lead.as_ref().is_some_and(|u| u.is_me)
}

pub(crate) fn issue_target(i: &Issue) -> Target {
    Target {
        kind: TargetKind::Issue,
        id: i.id.inner().to_owned(),
        identifier: i.identifier.clone(),
        title: i.title.clone(),
        url: i.url.clone(),
    }
}

pub(crate) fn project_target(p: &Project) -> Target {
    Target {
        kind: TargetKind::Project,
        id: p.id.inner().to_owned(),
        identifier: p.slug_id.clone(),
        title: p.name.clone(),
        url: p.url.clone(),
    }
}
