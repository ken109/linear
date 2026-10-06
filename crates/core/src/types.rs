//! Domain types.
//!
//! Each entity has exactly one fixed fragment (the set of fields we select),
//! and every query that returns that entity uses it. The cynic fragment struct
//! *is* the domain type: it is what `--json`, the cache and the WebAssembly
//! boundary all share. Names serialize as camelCase so `--json` mirrors
//! Linear's own response shape.
//!
//! Relations are selected through small `*Ref` fragments (fragments cannot
//! recurse), and nested connections are bounded with `first`.

use crate::nodes::{nodes_container, paged_container};
use crate::schema;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

// ---------------------------------------------------------------- shared

/// Relay page info.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

/// Variables shared by every paginated query.
#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct PageVars {
    pub first: i32,
    pub after: Option<String>,
}

// ---------------------------------------------------------------- enums
//
// Every enum has a fallback so that a value Linear adds later never makes a
// whole response fail to parse.

#[derive(cynic::Enum, Debug, Clone, PartialEq, Eq)]
#[cynic(rename_all = "camelCase")]
pub enum ProjectStatusType {
    Backlog,
    Planned,
    Started,
    Paused,
    Completed,
    Canceled,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Debug, Clone, PartialEq, Eq)]
#[cynic(rename_all = "camelCase")]
pub enum ProjectUpdateHealthType {
    OnTrack,
    AtRisk,
    OffTrack,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Debug, Clone, PartialEq, Eq)]
#[cynic(rename_all = "camelCase")]
pub enum ProjectMilestoneStatus {
    Done,
    Next,
    Overdue,
    Unstarted,
    #[cynic(fallback)]
    Other(String),
}

/// Linear spells these values with a leading capital.
#[derive(cynic::Enum, Debug, Clone, PartialEq, Eq)]
#[cynic(rename_all = "None")]
pub enum InitiativeStatus {
    Active,
    Canceled,
    Completed,
    Planned,
    Proposed,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::Enum, Debug, Clone, PartialEq, Eq)]
#[cynic(rename_all = "camelCase")]
pub enum LabelGroupType {
    MultiSelect,
    SingleSelect,
    #[cynic(fallback)]
    Other(String),
}

// ---------------------------------------------------------------- refs

/// A team.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
pub struct Team {
    pub id: cynic::Id,
    pub key: String,
    pub name: String,
}

/// A person.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: cynic::Id,
    pub name: String,
    pub display_name: String,
    pub email: String,
    pub active: bool,
    pub is_me: bool,
}

/// A workflow state. Decide on [`WorkflowState::state_type`], never on the
/// name: names are per-team and renamable, types are not.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowState {
    pub id: cynic::Id,
    pub name: String,
    #[cynic(rename = "type")]
    #[serde(rename = "type")]
    pub type_: String,
}

/// The workflow state types Linear defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateType {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    Duplicate,
    Other(String),
}

impl StateType {
    pub fn parse(s: &str) -> Self {
        match s {
            "triage" => Self::Triage,
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "canceled" => Self::Canceled,
            "duplicate" => Self::Duplicate,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Completed, canceled or duplicate.
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Duplicate)
    }
}

impl WorkflowState {
    pub fn state_type(&self) -> StateType {
        StateType::parse(&self.type_)
    }
}

/// A reference to a project from another entity.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
}

/// A reference to an issue from another entity.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Issue")]
pub struct IssueRef {
    pub id: cynic::Id,
    pub identifier: String,
    pub url: String,
}

/// A reference to an initiative from a project.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Initiative")]
pub struct InitiativeRef {
    pub id: cynic::Id,
    pub name: String,
    pub url: String,
}

/// The state of an issue, selected only to count issues per state type.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Issue")]
pub struct IssueStateRef {
    pub state: WorkflowState,
}

/// A project's status (the project-level analogue of a workflow state).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "ProjectStatus")]
pub struct ProjectStatus {
    pub id: cynic::Id,
    pub name: String,
    #[cynic(rename = "type")]
    #[serde(rename = "type")]
    pub type_: ProjectStatusType,
}

// ---------------------------------------------------------------- label

/// The group a label belongs to.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "IssueLabel")]
#[serde(rename_all = "camelCase")]
pub struct LabelGroup {
    pub id: cynic::Id,
    pub name: String,
    pub group_type: Option<LabelGroupType>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "IssueLabel")]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: cynic::Id,
    pub name: String,
    pub color: String,
    pub is_group: bool,
    pub parent: Option<LabelGroup>,
}

nodes_container!(LabelNodes, "IssueLabelConnection", Label);

// ---------------------------------------------------------------- attachment

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: cynic::Id,
    pub title: String,
    pub subtitle: Option<String>,
    pub url: String,
    pub source_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

nodes_container!(AttachmentNodes, "AttachmentConnection", Attachment);

// ---------------------------------------------------------------- milestone

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "ProjectMilestone")]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    pub id: cynic::Id,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<NaiveDate>,
    pub status: ProjectMilestoneStatus,
    pub sort_order: f64,
    pub progress: f64,
    pub project: ProjectRef,
}

nodes_container!(MilestoneNodes, "ProjectMilestoneConnection", Milestone);

// ---------------------------------------------------------------- status update

/// A project status update (Linear's `ProjectUpdate`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "ProjectUpdate")]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate {
    pub id: cynic::Id,
    pub url: String,
    pub body: String,
    pub health: ProjectUpdateHealthType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user: User,
    pub project: ProjectRef,
}

// ---------------------------------------------------------------- issue

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: cynic::Id,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub team: Team,
    pub state: WorkflowState,
    pub assignee: Option<User>,
    pub project: Option<ProjectRef>,
    pub project_milestone: Option<Milestone>,
    #[arguments(first: 50)]
    pub labels: LabelNodes,
    pub due_date: Option<NaiveDate>,
    pub estimate: Option<f64>,
    /// Position in a manual-order view (ascending, top first). `reorder` writes it.
    pub sort_order: f64,
    /// Position in a priority-order view, Linear's default (ascending, top first).
    /// `reorder` writes it together with `sort_order`.
    pub priority_sort_order: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub parent: Option<IssueRef>,
    #[arguments(first: 10)]
    pub attachments: AttachmentNodes,
}

impl Issue {
    /// The URL of the first attachment, taken as the issue's origin.
    ///
    /// Attachments are the only place Linear lets us record where an issue
    /// came from, and the first one attached is the origin by convention.
    pub fn source_url(&self) -> Option<&str> {
        self.attachments.first().map(|a| a.url.as_str())
    }
}

paged_container!(IssueConnection, "IssueConnection", Issue);

// ---------------------------------------------------------------- comment

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: cynic::Id,
    pub url: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user: Option<User>,
    pub issue: Option<IssueRef>,
}

// ---------------------------------------------------------------- project

paged_container!(IssueStateConnection, "IssueConnection", IssueStateRef);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
    pub status: ProjectStatus,
    pub lead: Option<User>,
    pub start_date: Option<NaiveDate>,
    pub target_date: Option<NaiveDate>,
    pub health: Option<ProjectUpdateHealthType>,
    #[arguments(first: 50)]
    pub project_milestones: MilestoneNodes,
    #[arguments(first: 20)]
    pub initiatives: InitiativeRefNodes,
    pub last_update: Option<StatusUpdate>,
    #[arguments(first: 100)]
    pub issues: IssueStateConnection,
    pub updated_at: DateTime<Utc>,
}

nodes_container!(InitiativeRefNodes, "InitiativeConnection", InitiativeRef);

/// Issues of a project grouped by workflow state type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct IssueCounts {
    pub triage: u32,
    pub backlog: u32,
    pub unstarted: u32,
    pub started: u32,
    pub completed: u32,
    pub canceled: u32,
    /// Includes `duplicate`.
    pub other: u32,
    /// `false` when the project has more issues than the selection window
    /// (so the counts are a lower bound).
    pub complete: bool,
}

impl IssueCounts {
    pub fn total(&self) -> u32 {
        self.triage
            + self.backlog
            + self.unstarted
            + self.started
            + self.completed
            + self.canceled
            + self.other
    }
}

impl Project {
    pub fn issue_counts(&self) -> IssueCounts {
        let mut c = IssueCounts {
            complete: !self.issues.page_info.has_next_page,
            ..IssueCounts::default()
        };
        for i in &self.issues.nodes {
            match i.state.state_type() {
                StateType::Triage => c.triage += 1,
                StateType::Backlog => c.backlog += 1,
                StateType::Unstarted => c.unstarted += 1,
                StateType::Started => c.started += 1,
                StateType::Completed => c.completed += 1,
                StateType::Canceled => c.canceled += 1,
                StateType::Duplicate | StateType::Other(_) => c.other += 1,
            }
        }
        c
    }
}

paged_container!(ProjectConnection, "ProjectConnection", Project);

// ---------------------------------------------------------------- initiative

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Initiative {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
    pub status: InitiativeStatus,
    pub target_date: Option<NaiveDate>,
    pub owner: Option<User>,
}

paged_container!(InitiativeConnection, "InitiativeConnection", Initiative);

// ---------------------------------------------------------------- template

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: cynic::Id,
    pub name: String,
    pub description: Option<String>,
    #[cynic(rename = "type")]
    #[serde(rename = "type")]
    pub type_: String,
    pub team: Option<Team>,
    pub template_data: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

impl Template {
    /// The template payload as JSON.
    ///
    /// Linear's `JSON` scalar arrives as a JSON document *encoded in a string*
    /// for templates; this decodes it (and passes real JSON values through).
    pub fn data(&self) -> Option<serde_json::Value> {
        match &self.template_data {
            serde_json::Value::String(s) => serde_json::from_str(s).ok(),
            other => Some(other.clone()),
        }
    }
}

// ---------------------------------------------------------------- organization

/// The workspace itself, as Linear names it.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub id: cynic::Id,
    pub name: String,
    pub url_key: String,
}
