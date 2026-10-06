//! Query documents.
//!
//! Each query is a cynic fragment checked against the vendored schema at
//! compile time. Page sizes are chosen per query to stay under Linear's
//! query-complexity limit.

use crate::nodes::nodes_container;
use crate::schema;
use crate::types::*;
use cynic::{Operation, QueryBuilder};

// ---------------------------------------------------------------- whoami

/// The authenticated user and the workspace the credential belongs to.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query")]
pub struct Whoami {
    pub viewer: User,
    pub organization: Organization,
}

pub fn whoami() -> Operation<Whoami, ()> {
    Whoami::build(())
}

// ---------------------------------------------------------------- issues

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
pub struct AssignedStartedIssues {
    pub viewer: ViewerAssignedStarted,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "User", variables = "PageVars")]
pub struct ViewerAssignedStarted {
    #[arguments(first: $first, after: $after, filter: {state: {type: {eq: "started"}}})]
    pub assigned_issues: IssueConnection,
}

/// Issues assigned to the viewer whose state type is `started`.
pub const ASSIGNED_STARTED_PAGE_SIZE: i32 = 50;

pub fn assigned_started_issues(vars: PageVars) -> Operation<AssignedStartedIssues, PageVars> {
    AssignedStartedIssues::build(vars)
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IssueVars {
    /// An issue id or identifier such as `KK-1`.
    pub id: String,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IssueVars")]
pub struct IssueById {
    #[arguments(id: $id)]
    pub issue: Issue,
}

pub fn issue(id: impl Into<String>) -> Operation<IssueById, IssueVars> {
    IssueById::build(IssueVars { id: id.into() })
}

// ---------------------------------------------------------------- projects

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
pub struct Projects {
    #[arguments(first: $first, after: $after)]
    pub projects: ProjectConnection,
}

/// Each project selects up to 100 issues and 50 milestones. At 20 projects per
/// page Linear rejected the query (complexity 12720 > 10000), so pages stay small.
pub const PROJECTS_PAGE_SIZE: i32 = 10;

pub fn projects(vars: PageVars) -> Operation<Projects, PageVars> {
    Projects::build(vars)
}

// ---------------------------------------------------------------- comments

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IssueVars")]
pub struct IssueCommentsQuery {
    #[arguments(id: $id)]
    pub issue: IssueWithComments,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Issue")]
pub struct IssueWithComments {
    #[arguments(first: 50)]
    pub comments: CommentNodes,
}

nodes_container!(CommentNodes, "CommentConnection", Comment);

pub fn issue_comments(id: impl Into<String>) -> Operation<IssueCommentsQuery, IssueVars> {
    IssueCommentsQuery::build(IssueVars { id: id.into() })
}

// ---------------------------------------------------------------- templates

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query")]
pub struct Templates {
    pub templates: Vec<Template>,
}

pub fn templates() -> Operation<Templates, ()> {
    Templates::build(())
}

// ---------------------------------------------------------------- initiatives

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "PageVars")]
pub struct Initiatives {
    #[arguments(first: $first, after: $after)]
    pub initiatives: InitiativeConnection,
}

pub const INITIATIVES_PAGE_SIZE: i32 = 50;

pub fn initiatives(vars: PageVars) -> Operation<Initiatives, PageVars> {
    Initiatives::build(vars)
}
