//! `audit`: rules that find Linear data that has drifted.
//!
//! Everything here is a pure function of a [`Snapshot`] and a `now` handed in
//! by the caller (this crate never reads the clock), so the same rules run in
//! the CLI and, through WebAssembly, in a Worker. Rules decide on workflow
//! state *types*, never on names, which teams may rename.

mod consistency;
mod finding;
mod stale;

use crate::types::{Issue, Project, ProjectStatusType};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use finding::{AuditReport, Finding, FindingKey, RuleId, Severity, Target, TargetKind};

/// Everything the audit looks at, for one workspace.
///
/// `issues` should hold the open issues worth auditing plus any issue whose
/// state changed recently; the rules only see what is here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub workspace: String,
    pub issues: Vec<Issue>,
    pub projects: Vec<Project>,
}

/// The thresholds of the staleness rules, set per workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    /// `stale-in-progress`: an In Progress issue with no update for this many
    /// days is stale.
    pub stale_days: u32,
    /// `status-update-outdated`: a project's latest status update this many
    /// days old is outdated.
    pub status_update_days: u32,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            stale_days: 7,
            status_update_days: 14,
        }
    }
}

/// Run every rule over `snapshot`.
///
/// `now` decides what is overdue or stale; target dates are compared as
/// calendar dates in UTC, ages in whole days since the timestamp.
pub fn audit(snapshot: &Snapshot, config: &AuditConfig, now: DateTime<Utc>) -> AuditReport {
    let ctx = Ctx::new(snapshot, config, now);
    let mut findings = Vec::new();
    consistency::run(&ctx, &mut findings);
    stale::run(&ctx, &mut findings);
    findings.sort_by(|a, b| {
        (a.rule, a.target.kind, &a.target.identifier).cmp(&(
            b.rule,
            b.target.kind,
            &b.target.identifier,
        ))
    });
    AuditReport { findings }
}

/// What the rules share.
pub(crate) struct Ctx<'a> {
    pub snapshot: &'a Snapshot,
    pub config: &'a AuditConfig,
    pub now: DateTime<Utc>,
    pub today: NaiveDate,
    projects: HashMap<&'a str, &'a Project>,
}

impl<'a> Ctx<'a> {
    fn new(snapshot: &'a Snapshot, config: &'a AuditConfig, now: DateTime<Utc>) -> Self {
        Self {
            snapshot,
            config,
            now,
            today: now.date_naive(),
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
