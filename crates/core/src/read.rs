//! Queries for the read commands (`issue list|view`, `project list|view`, ...).
//!
//! The entity fragments live in [`crate::types`]. The "detail" fragments here
//! add what only a `view` needs (a description, comments, status updates) and
//! are selected under an alias next to the entity's fixed fragment, so a view
//! is still one request and `list` never pays for the extra fields.

use crate::filters::{CycleFilter, InitiativeFilter, IssueFilter, ProjectFilter};
use crate::nodes::{nodes_container, paged_container};
use crate::queries::CommentNodes;
use crate::schema;
use crate::types::*;
use chrono::NaiveDate;
use cynic::{Operation, QueryBuilder};
use schemars::JsonSchema;
use serde::Serialize;

/// Variables of a query that takes a single id (or identifier, or slug).
#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IdVars {
    pub id: String,
}

// ---------------------------------------------------------------- issues

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IssueListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<IssueFilter>,
}

impl IssueListVars {
    pub fn new(page: PageVars, filter: Option<IssueFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IssueListVars")]
pub struct IssueList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub issues: IssueConnection,
}

/// Same size as the assigned-issues listing, which stays under the complexity limit.
pub const ISSUE_LIST_PAGE_SIZE: i32 = 50;

pub fn issue_list(vars: IssueListVars) -> Operation<IssueList, IssueListVars> {
    IssueList::build(vars)
}

/// What a `view` shows beyond the issue's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Issue")]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub priority: f64,
    pub priority_label: String,
    #[arguments(first: 50)]
    pub comments: CommentNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
#[serde(rename_all = "camelCase")]
pub struct IssueView {
    #[arguments(id: $id)]
    pub issue: Issue,
    #[cynic(alias, rename = "issue")]
    #[arguments(id: $id)]
    pub detail: IssueDetail,
}

/// `id` is an issue id or an identifier such as `KK-1`.
pub fn issue_view(id: impl Into<String>) -> Operation<IssueView, IdVars> {
    IssueView::build(IdVars { id: id.into() })
}

/// An issue reduced to what a list inside another entity needs.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Issue")]
#[serde(rename_all = "camelCase")]
pub struct IssueBrief {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: WorkflowState,
    pub assignee: Option<User>,
}

nodes_container!(IssueBriefNodes, "IssueConnection", IssueBrief);

// ---------------------------------------------------------------- write context
//
// What a write needs to know before it sends anything: who owns the thing
// (for the write guard) and the names it may refer to (workflow states,
// milestones). Read-only; shared by every command that writes.

nodes_container!(WorkflowStateNodes, "WorkflowStateConnection", WorkflowState);

/// A team's workflow states, to turn a state name into an id.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Team")]
pub struct TeamStates {
    #[arguments(first: 50)]
    pub states: WorkflowStateNodes,
}

/// A project as the write guard and name resolution see it: its lead and its
/// milestones.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Project")]
pub struct ProjectOwnership {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
    pub lead: Option<User>,
    #[arguments(first: 100)]
    pub project_milestones: MilestoneNodes,
}

impl ProjectOwnership {
    /// Where an issue in this project sits, as far as ownership is concerned.
    pub fn placement(&self) -> crate::guard::Placement<'_> {
        crate::guard::Placement::Project {
            lead: self.lead.as_ref().map(|u| u.id.inner()),
        }
    }
}

/// What an issue write needs beyond the issue's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Issue")]
pub struct IssueWriteDetail {
    pub team: TeamStates,
    pub project: Option<ProjectOwnership>,
    /// The cycle the issue is in, if any (`issue create --held-on` only
    /// fills an empty one).
    pub cycle: Option<Cycle>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct IssueWriteView {
    #[arguments(id: $id)]
    pub issue: Issue,
    #[cynic(alias, rename = "issue")]
    #[arguments(id: $id)]
    pub write: IssueWriteDetail,
}

/// `id` is an issue id or an identifier such as `KK-1`.
pub fn issue_write_view(id: impl Into<String>) -> Operation<IssueWriteView, IdVars> {
    IssueWriteView::build(IdVars { id: id.into() })
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct ProjectOwnershipQuery {
    #[arguments(id: $id)]
    pub project: ProjectOwnership,
}

/// `id` is the project's id (resolve a name or URL first with `project_refs`).
pub fn project_ownership(id: impl Into<String>) -> Operation<ProjectOwnershipQuery, IdVars> {
    ProjectOwnershipQuery::build(IdVars { id: id.into() })
}

// ---------------------------------------------------------------- issues by origin

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct UrlVars {
    pub url: String,
}

/// An attachment as the origin lookup needs it: the issue it hangs on, and
/// what it stores (to tell whether `--meta` would change it).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Attachment")]
pub struct AttachmentOwner {
    pub issue: IssueRef,
    pub title: String,
    pub subtitle: Option<String>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

nodes_container!(
    AttachmentOwnerNodes,
    "AttachmentConnection",
    AttachmentOwner
);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "UrlVars")]
pub struct AttachmentsForUrlQuery {
    #[cynic(rename = "attachmentsForURL")]
    #[arguments(url: $url, first: 1)]
    pub attachments_for_url: AttachmentOwnerNodes,
}

/// The issue that carries an attachment with exactly this URL, if any. This is
/// the lookup behind source idempotence (`source-attachment`).
pub fn attachments_for_url(url: impl Into<String>) -> Operation<AttachmentsForUrlQuery, UrlVars> {
    AttachmentsForUrlQuery::build(UrlVars { url: url.into() })
}

// ---------------------------------------------------------------- cycles

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct CycleListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<CycleFilter>,
}

impl CycleListVars {
    pub fn new(page: PageVars, filter: Option<CycleFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "CycleListVars")]
pub struct CycleList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub cycles: CycleConnection,
}

pub const CYCLE_LIST_PAGE_SIZE: i32 = 100;

/// The cycles that match `filter` (`CycleQuery::filter` narrows them to a team).
pub fn cycles(vars: CycleListVars) -> Operation<CycleList, CycleListVars> {
    CycleList::build(vars)
}

// ---------------------------------------------------------------- projects

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct ProjectListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<ProjectFilter>,
}

impl ProjectListVars {
    pub fn new(page: PageVars, filter: Option<ProjectFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "ProjectListVars")]
pub struct ProjectList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub projects: ProjectConnection,
}

pub fn project_list(vars: ProjectListVars) -> Operation<ProjectList, ProjectListVars> {
    ProjectList::build(vars)
}

paged_container!(ProjectRefConnection, "ProjectConnection", ProjectRef);

/// Every project, reduced to the fields needed to resolve a reference.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
pub struct ProjectRefs {
    #[arguments(first: $first, after: $after)]
    pub projects: ProjectRefConnection,
}

pub const PROJECT_REFS_PAGE_SIZE: i32 = 100;

pub fn project_refs(vars: PageVars) -> Operation<ProjectRefs, PageVars> {
    ProjectRefs::build(vars)
}

nodes_container!(StatusUpdateNodes, "ProjectUpdateConnection", StatusUpdate);

/// How many status updates a project `view` shows.
pub const PROJECT_VIEW_UPDATES: usize = 5;

/// What a project `view` shows beyond the project's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub description: String,
    pub content: Option<String>,
    #[arguments(first: 5)]
    pub project_updates: StatusUpdateNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[arguments(id: $id)]
    pub project: Project,
    #[cynic(alias, rename = "project")]
    #[arguments(id: $id)]
    pub detail: ProjectDetail,
}

pub fn project_view(id: impl Into<String>) -> Operation<ProjectView, IdVars> {
    ProjectView::build(IdVars { id: id.into() })
}

/// A project reduced to what a list inside another entity needs.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectBrief {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
    pub status: ProjectStatus,
    pub target_date: Option<NaiveDate>,
}

nodes_container!(ProjectBriefNodes, "ProjectConnection", ProjectBrief);

// ---------------------------------------------------------------- milestones

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectMilestones {
    #[arguments(first: 100)]
    pub project_milestones: MilestoneNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
#[serde(rename_all = "camelCase")]
pub struct MilestonesOfProject {
    #[arguments(id: $id)]
    pub project: ProjectMilestones,
}

/// The milestones of one project (id, slug id or whatever `project(id:)` takes).
pub fn milestones_of_project(
    project_id: impl Into<String>,
) -> Operation<MilestonesOfProject, IdVars> {
    MilestonesOfProject::build(IdVars {
        id: project_id.into(),
    })
}

/// What a milestone `view` shows beyond the milestone's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "ProjectMilestone")]
pub struct MilestoneDetail {
    #[arguments(first: 100)]
    pub issues: IssueBriefNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
#[serde(rename_all = "camelCase")]
pub struct MilestoneView {
    #[arguments(id: $id)]
    pub project_milestone: Milestone,
    #[cynic(alias, rename = "projectMilestone")]
    #[arguments(id: $id)]
    pub detail: MilestoneDetail,
}

pub fn milestone_view(id: impl Into<String>) -> Operation<MilestoneView, IdVars> {
    MilestoneView::build(IdVars { id: id.into() })
}

// ---------------------------------------------------------------- initiatives

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<InitiativeFilter>,
}

impl InitiativeListVars {
    pub fn new(page: PageVars, filter: Option<InitiativeFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "InitiativeListVars")]
pub struct InitiativeList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub initiatives: InitiativeConnection,
}

pub const INITIATIVE_LIST_PAGE_SIZE: i32 = 50;

pub fn initiative_list(vars: InitiativeListVars) -> Operation<InitiativeList, InitiativeListVars> {
    InitiativeList::build(vars)
}

/// What an initiative `view` shows beyond the initiative's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Initiative")]
pub struct InitiativeDetail {
    #[arguments(first: 50)]
    pub projects: ProjectBriefNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
#[serde(rename_all = "camelCase")]
pub struct InitiativeView {
    #[arguments(id: $id)]
    pub initiative: Initiative,
    #[cynic(alias, rename = "initiative")]
    #[arguments(id: $id)]
    pub detail: InitiativeDetail,
}

pub fn initiative_view(id: impl Into<String>) -> Operation<InitiativeView, IdVars> {
    InitiativeView::build(IdVars { id: id.into() })
}

// ---------------------------------------------------------------- labels, teams, users

paged_container!(LabelConnection, "IssueLabelConnection", Label);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
#[serde(rename_all = "camelCase")]
pub struct Labels {
    #[arguments(first: $first, after: $after)]
    pub issue_labels: LabelConnection,
}

pub const LABELS_PAGE_SIZE: i32 = 100;

pub fn labels(vars: PageVars) -> Operation<Labels, PageVars> {
    Labels::build(vars)
}

paged_container!(TeamConnection, "TeamConnection", Team);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
#[serde(rename_all = "camelCase")]
pub struct Teams {
    #[arguments(first: $first, after: $after)]
    pub teams: TeamConnection,
}

pub const TEAMS_PAGE_SIZE: i32 = 100;

pub fn teams(vars: PageVars) -> Operation<Teams, PageVars> {
    Teams::build(vars)
}

paged_container!(UserConnection, "UserConnection", User);

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct UserListVars {
    pub first: i32,
    pub after: Option<String>,
    pub include_disabled: Option<bool>,
}

impl UserListVars {
    pub fn new(page: PageVars, include_disabled: bool) -> Self {
        Self {
            first: page.first,
            after: page.after,
            // Left out (null) unless asked for, so Linear's own default applies.
            include_disabled: include_disabled.then_some(true),
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "UserListVars")]
#[serde(rename_all = "camelCase")]
pub struct Users {
    #[arguments(first: $first, after: $after, includeDisabled: $include_disabled)]
    pub users: UserConnection,
}

pub const USERS_PAGE_SIZE: i32 = 100;

pub fn users(vars: UserListVars) -> Operation<Users, UserListVars> {
    Users::build(vars)
}
