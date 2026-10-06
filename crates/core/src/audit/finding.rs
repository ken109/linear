//! The shape of an audit result.

use serde::{Deserialize, Serialize};

/// Which rule produced a finding. The declaration order is the order findings
/// are reported in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleId {
    /// A project's status disagrees with the states of its issues.
    ProjectStateVsIssues,
    /// A project, milestone or issue is past its target date and still open.
    Overdue,
    /// An open issue sits in a project that has milestones, but in none of them.
    IssueWithoutMilestone,
    /// A project that is not closed has no lead.
    ProjectWithoutLead,
    /// An issue has been In Progress without any update for too long.
    StaleInProgress,
    /// An In Progress project's latest status update is out of date.
    StatusUpdateOutdated,
    /// A validator rule (`template-sections`) applied to an existing issue.
    TemplateSections,
    /// A validator rule (`source-attachment`) applied to an existing issue.
    SourceAttachment,
    /// A validator rule (`label-groups-exclusive`) applied to an existing issue.
    LabelGroupsExclusive,
    /// An issue the caller said it touched was not updated since a given time.
    NotUpdatedSince,
}

impl RuleId {
    /// The stable name used in output and configuration.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectStateVsIssues => "project-state-vs-issues",
            Self::Overdue => "overdue",
            Self::IssueWithoutMilestone => "issue-without-milestone",
            Self::ProjectWithoutLead => "project-without-lead",
            Self::StaleInProgress => "stale-in-progress",
            Self::StatusUpdateOutdated => "status-update-outdated",
            Self::TemplateSections => "template-sections",
            Self::SourceAttachment => "source-attachment",
            Self::LabelGroupsExclusive => "label-groups-exclusive",
            Self::NotUpdatedSince => "not-updated-since",
        }
    }
}

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Linear's data is wrong or out of date.
    Warn,
    /// Housekeeping: worth knowing, nothing is broken.
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Project,
    Milestone,
    Issue,
}

/// The thing a finding is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub kind: TargetKind,
    /// Linear's id; with the workspace it identifies the target.
    pub id: String,
    /// What a person calls it: the issue identifier (`KK-12`), the project's
    /// slug id, or the milestone's name.
    pub identifier: String,
    pub title: String,
    /// A link to the target (a milestone links to its project).
    pub url: String,
}

/// One problem the audit found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: RuleId,
    pub severity: Severity,
    pub workspace: String,
    pub target: Target,
    pub message: String,
    /// `true` when the viewer owns the target (leads the project, or is
    /// assigned the issue) and can fix it. Everything else is informational.
    pub actionable: bool,
    /// A command that fixes it. Placeholders such as `<state>` stand for a
    /// value the person has to choose.
    pub fix: String,
}

/// The identity of a finding: what stays the same while the finding persists.
/// The message is not part of it (it carries counts of days that change daily).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FindingKey {
    pub workspace: String,
    pub rule: RuleId,
    pub kind: TargetKind,
    pub target_id: String,
}

impl Finding {
    pub fn key(&self) -> FindingKey {
        FindingKey {
            workspace: self.workspace.clone(),
            rule: self.rule,
            kind: self.target.kind,
            target_id: self.target.id.clone(),
        }
    }
}

/// The result of one audit of one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuditReport {
    /// Sorted by rule, then target kind, then target identifier.
    pub findings: Vec<Finding>,
    /// Issue identifiers asked for with `issues` that the snapshot does not
    /// contain. They could not be checked, which is not the same as passing.
    #[serde(default)]
    pub unresolved_issues: Vec<String>,
}

impl AuditReport {
    /// Findings the viewer can fix.
    pub fn actionable(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.actionable)
    }

    /// Findings that are only informational.
    pub fn informational(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| !f.actionable)
    }

    /// What `--fail-on actionable` turns into exit code 6.
    pub fn has_actionable(&self) -> bool {
        self.findings.iter().any(|f| f.actionable)
    }
}
