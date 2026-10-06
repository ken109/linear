//! `audit`: rules that find Linear data that has drifted.
//!
//! Everything here is a pure function of a [`Snapshot`] and a `now` handed in
//! by the caller (this crate never reads the clock), so the same rules run in
//! the CLI and, through WebAssembly, in a Worker. Rules decide on workflow
//! state *types*, never on names, which teams may rename.

mod consistency;
mod finding;

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

/// Run every rule over `snapshot`.
///
/// `now` decides what is overdue; target dates are compared as calendar dates
/// in UTC.
pub fn audit(snapshot: &Snapshot, now: DateTime<Utc>) -> AuditReport {
    let ctx = Ctx::new(snapshot, now);
    let mut findings = Vec::new();
    consistency::run(&ctx, &mut findings);
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
    pub today: NaiveDate,
    projects: HashMap<&'a str, &'a Project>,
}

impl<'a> Ctx<'a> {
    fn new(snapshot: &'a Snapshot, now: DateTime<Utc>) -> Self {
        Self {
            snapshot,
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
