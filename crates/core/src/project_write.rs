//! What project writes are built from: the inputs, the mutations that carry
//! them, and the queries a write reads first.
//!
//! The same rule as [`crate::inputs`] holds: every `Option` field carries
//! `skip_serializing_if`, so an update that names no value for a field never
//! sends `null` for it. A field that must be able to say "clear" explicitly
//! uses [`Patch`].

use crate::error::Result;
use crate::filters::{ProjectStatusFilter, StringComparator};
use crate::inputs::Patch;
use crate::matching::pick;
use crate::nodes::nodes_container;
use crate::read::{IdVars, IdVarsFields};
use crate::schema;
use crate::types::{
    Project, ProjectRef, ProjectStatus, ProjectUpdateHealthType, StatusUpdate, User,
};
use chrono::NaiveDate;
use cynic::{MutationBuilder, Operation, QueryBuilder};

/// Project status types that end a project. A project of another type is "not finished".
pub const FINISHED_STATUS_TYPES: [&str; 2] = ["completed", "canceled"];

// ---------------------------------------------------------------- project create

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "ProjectCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct ProjectCreateInput {
    pub name: String,
    pub team_ids: Vec<String>,
    /// The short summary.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The body (markdown).
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<NaiveDate>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct ProjectCreateVars {
    pub input: ProjectCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "ProjectCreateVars")]
pub struct ProjectCreate {
    #[arguments(input: $input)]
    pub project_create: ProjectPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct ProjectPayload {
    pub success: bool,
    pub project: Option<Project>,
}

pub fn project_create(input: ProjectCreateInput) -> Operation<ProjectCreate, ProjectCreateVars> {
    ProjectCreate::build(ProjectCreateVars { input })
}

// ---------------------------------------------------------------- project update

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "ProjectUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct ProjectUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub content: Patch<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub status_id: Option<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub target_date: Patch<NaiveDate>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub lead_id: Patch<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<f64>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub priority_sort_order: Option<f64>,
}

impl ProjectUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.content.is_keep()
            && self.status_id.is_none()
            && self.target_date.is_keep()
            && self.lead_id.is_keep()
            && self.sort_order.is_none()
            && self.priority_sort_order.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct ProjectUpdateVars {
    pub id: String,
    pub input: ProjectUpdateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "ProjectUpdateVars")]
pub struct ProjectUpdate {
    #[arguments(id: $id, input: $input)]
    pub project_update: ProjectPayload,
}

pub fn project_update(
    id: impl Into<String>,
    input: ProjectUpdateInput,
) -> Operation<ProjectUpdate, ProjectUpdateVars> {
    ProjectUpdate::build(ProjectUpdateVars {
        id: id.into(),
        input,
    })
}

// ---------------------------------------------------------------- project delete

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct ProjectDeleteVars {
    pub id: String,
}

/// Trashes a project (Linear keeps it for a while). Used to roll back a
/// half-made project.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "ProjectDeleteVars")]
pub struct ProjectDelete {
    #[arguments(id: $id)]
    pub project_delete: ProjectArchivePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct ProjectArchivePayload {
    pub success: bool,
}

pub fn project_delete(id: impl Into<String>) -> Operation<ProjectDelete, ProjectDeleteVars> {
    ProjectDelete::build(ProjectDeleteVars { id: id.into() })
}

// ---------------------------------------------------------------- status update

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "ProjectUpdateCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct StatusUpdateCreateInput {
    pub project_id: String,
    pub health: ProjectUpdateHealthType,
    pub body: String,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct ProjectUpdateCreateVars {
    pub input: StatusUpdateCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "ProjectUpdateCreateVars")]
pub struct ProjectUpdateCreate {
    #[arguments(input: $input)]
    pub project_update_create: StatusUpdatePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "ProjectUpdatePayload")]
pub struct StatusUpdatePayload {
    pub success: bool,
    pub project_update: StatusUpdate,
}

/// Writes a status update (Linear's `ProjectUpdate`) on a project.
pub fn project_update_create(
    input: StatusUpdateCreateInput,
) -> Operation<ProjectUpdateCreate, ProjectUpdateCreateVars> {
    ProjectUpdateCreate::build(ProjectUpdateCreateVars { input })
}

// ---------------------------------------------------------------- initiative link

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "InitiativeToProjectCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct InitiativeToProjectCreateInput {
    pub initiative_id: String,
    pub project_id: String,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeToProjectCreateVars {
    pub input: InitiativeToProjectCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "InitiativeToProjectCreateVars")]
pub struct InitiativeToProjectCreate {
    #[arguments(input: $input)]
    pub initiative_to_project_create: InitiativeToProjectPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct InitiativeToProjectPayload {
    pub success: bool,
}

/// Puts a project under an initiative.
pub fn initiative_to_project_create(
    initiative_id: impl Into<String>,
    project_id: impl Into<String>,
) -> Operation<InitiativeToProjectCreate, InitiativeToProjectCreateVars> {
    InitiativeToProjectCreate::build(InitiativeToProjectCreateVars {
        input: InitiativeToProjectCreateInput {
            initiative_id: initiative_id.into(),
            project_id: project_id.into(),
        },
    })
}

// ---------------------------------------------------------------- queries a write reads

nodes_container!(ProjectStatusNodes, "ProjectStatusConnection", ProjectStatus);

/// The workspace's project statuses, to turn a status name into an id.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query")]
pub struct ProjectStatuses {
    #[arguments(first: 50)]
    pub project_statuses: ProjectStatusNodes,
}

pub fn project_statuses() -> Operation<ProjectStatuses, ()> {
    ProjectStatuses::build(())
}

/// A project status by id or name (ignoring case).
pub fn match_project_status<'a>(
    rows: &'a [ProjectStatus],
    reference: &str,
) -> Result<&'a ProjectStatus> {
    pick(
        rows,
        "project status",
        reference,
        |s| s.id.inner() == reference || s.name == reference,
        |s| s.name.to_lowercase() == reference.to_lowercase(),
        |s| s.name.clone(),
    )
}

/// Narrows `projects` to the unfinished ones with exactly one name.
#[derive(cynic::InputObject, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "ProjectFilter")]
pub struct UnfinishedNamedFilter {
    pub name: StringComparator,
    pub status: ProjectStatusFilter,
}

impl UnfinishedNamedFilter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: StringComparator::eq(name),
            status: ProjectStatusFilter {
                type_: Some(StringComparator::none_of(FINISHED_STATUS_TYPES)),
            },
        }
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct UnfinishedNamedVars {
    pub filter: UnfinishedNamedFilter,
}

nodes_container!(ProjectRefNodes, "ProjectConnection", ProjectRef);

/// The projects that are not completed or canceled and carry exactly this
/// name. The name is what makes `project create` repeatable.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "UnfinishedNamedVars")]
pub struct UnfinishedNamed {
    #[arguments(first: 10, filter: $filter)]
    pub projects: ProjectRefNodes,
}

pub fn unfinished_named(
    name: impl Into<String>,
) -> Operation<UnfinishedNamed, UnfinishedNamedVars> {
    UnfinishedNamed::build(UnfinishedNamedVars {
        filter: UnfinishedNamedFilter::new(name),
    })
}

/// A project as `project reorder` sees it: who leads it and where it sits.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Project")]
pub struct ProjectOrder {
    pub id: cynic::Id,
    pub slug_id: String,
    pub name: String,
    pub lead: Option<User>,
    /// Position in a manual-order view (ascending, top first).
    pub sort_order: f64,
    /// Position in a priority-order view (ascending, top first).
    pub priority_sort_order: f64,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct ProjectOrderQuery {
    #[arguments(id: $id)]
    pub project: ProjectOrder,
}

/// `id` is the project's id (resolve a name or URL first).
pub fn project_order(id: impl Into<String>) -> Operation<ProjectOrderQuery, IdVars> {
    ProjectOrderQuery::build(IdVars { id: id.into() })
}
