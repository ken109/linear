//! Queries for the read commands (`issue list|view`, `project list|view`, ...).
//!
//! The entity fragments live in [`crate::types`]. The "detail" fragments here
//! add what only a `view` needs (a description, comments, status updates) and
//! are selected under an alias next to the entity's fixed fragment, so a view
//! is still one request and `list` never pays for the extra fields.

use crate::filters::{InitiativeFilter, IssueFilter, ProjectFilter};
use crate::nodes::{nodes_container, paged_container};
use crate::queries::CommentNodes;
use crate::schema;
use crate::types::*;
use chrono::NaiveDate;
use cynic::{Operation, QueryBuilder};
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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Issue")]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub priority: f64,
    pub priority_label: String,
    #[arguments(first: 50)]
    pub comments: CommentNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Issue")]
#[serde(rename_all = "camelCase")]
pub struct IssueBrief {
    pub id: cynic::Id,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: WorkflowState,
    pub assignee: Option<User>,
}

nodes_container!(IssueBriefNodes, "IssueConnection", IssueBrief);

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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub description: String,
    pub content: Option<String>,
    #[arguments(first: 5)]
    pub project_updates: StatusUpdateNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Project")]
#[serde(rename_all = "camelCase")]
pub struct ProjectBrief {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub url: String,
    pub status: ProjectStatus,
    pub target_date: Option<NaiveDate>,
}

nodes_container!(ProjectBriefNodes, "ProjectConnection", ProjectBrief);

// ---------------------------------------------------------------- milestones

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Project")]
pub struct ProjectMilestones {
    #[arguments(first: 100)]
    pub project_milestones: MilestoneNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "ProjectMilestone")]
pub struct MilestoneDetail {
    #[arguments(first: 100)]
    pub issues: IssueBriefNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
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
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize)]
#[cynic(graphql_type = "Initiative")]
pub struct InitiativeDetail {
    pub description: Option<String>,
    #[arguments(first: 50)]
    pub projects: ProjectBriefNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
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

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
pub struct Labels {
    #[arguments(first: $first, after: $after)]
    pub issue_labels: LabelConnection,
}

pub const LABELS_PAGE_SIZE: i32 = 100;

pub fn labels(vars: PageVars) -> Operation<Labels, PageVars> {
    Labels::build(vars)
}

paged_container!(TeamConnection, "TeamConnection", Team);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
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

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "UserListVars")]
pub struct Users {
    #[arguments(first: $first, after: $after, includeDisabled: $include_disabled)]
    pub users: UserConnection,
}

pub const USERS_PAGE_SIZE: i32 = 100;

pub fn users(vars: UserListVars) -> Operation<Users, UserListVars> {
    Users::build(vars)
}
