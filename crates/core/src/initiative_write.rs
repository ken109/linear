//! What initiative writes beyond `create`, `archive` and `delete` are built
//! from: the update, the link between an initiative and a project, and the
//! initiative's status updates.
//!
//! The same rule as [`crate::inputs`] holds: every `Option` field carries
//! `skip_serializing_if`, so an update that names no value for a field never
//! sends `null` for it.

use crate::inputs::{DeleteResult, InitiativePayload};
use crate::nodes::nodes_container;
use crate::read::{IdVars, IdVarsFields};
use crate::schema;
use crate::types::{InitiativeStatus, InitiativeStatusUpdate, InitiativeUpdateHealthType};
use chrono::NaiveDate;
use cynic::{MutationBuilder, Operation, QueryBuilder};
use schemars::JsonSchema;
use serde::Serialize;

// ---------------------------------------------------------------- initiative update

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "InitiativeUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct InitiativeUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub status: Option<InitiativeStatus>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<NaiveDate>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

impl InitiativeUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.status.is_none()
            && self.target_date.is_none()
            && self.owner_id.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeUpdateVars {
    pub id: String,
    pub input: InitiativeUpdateInput,
}

/// Changes an initiative (`initiativeUpdate`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "InitiativeUpdateVars")]
pub struct InitiativeUpdate {
    #[arguments(id: $id, input: $input)]
    pub initiative_update: InitiativePayload,
}

pub fn initiative_update(
    id: impl Into<String>,
    input: InitiativeUpdateInput,
) -> Operation<InitiativeUpdate, InitiativeUpdateVars> {
    InitiativeUpdate::build(InitiativeUpdateVars {
        id: id.into(),
        input,
    })
}

// ---------------------------------------------------------------- project link

/// One link between a project and an initiative (Linear's `InitiativeToProject`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "InitiativeToProject")]
pub struct InitiativeLink {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    pub initiative: InitiativeLinkTarget,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Initiative")]
pub struct InitiativeLinkTarget {
    #[schemars(with = "String")]
    pub id: cynic::Id,
}

nodes_container!(
    InitiativeLinkNodes,
    "InitiativeToProjectConnection",
    InitiativeLink
);

/// A project's links to its initiatives.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Project")]
pub struct ProjectLinks {
    #[arguments(first: 50)]
    pub initiative_to_projects: InitiativeLinkNodes,
}

/// The links of one project: the id `initiativeToProjectDelete` needs.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct ProjectLinksQuery {
    #[arguments(id: $id)]
    pub project: ProjectLinks,
}

pub fn project_links(project_id: impl Into<String>) -> Operation<ProjectLinksQuery, IdVars> {
    ProjectLinksQuery::build(IdVars {
        id: project_id.into(),
    })
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeToProjectDeleteVars {
    pub id: String,
}

/// Takes a project out of an initiative (`initiativeToProjectDelete`). Only the
/// link goes; the project and the initiative stay.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "InitiativeToProjectDeleteVars")]
pub struct InitiativeToProjectDelete {
    #[arguments(id: $id)]
    pub initiative_to_project_delete: DeleteResult,
}

pub fn initiative_to_project_delete(
    link_id: impl Into<String>,
) -> Operation<InitiativeToProjectDelete, InitiativeToProjectDeleteVars> {
    InitiativeToProjectDelete::build(InitiativeToProjectDeleteVars { id: link_id.into() })
}

// ---------------------------------------------------------------- status update

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "InitiativeUpdateCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct InitiativeStatusUpdateCreateInput {
    pub initiative_id: String,
    pub health: InitiativeUpdateHealthType,
    pub body: String,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeUpdateCreateVars {
    pub input: InitiativeStatusUpdateCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "InitiativeUpdateCreateVars")]
pub struct InitiativeUpdateCreate {
    #[arguments(input: $input)]
    pub initiative_update_create: InitiativeStatusUpdatePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "InitiativeUpdatePayload")]
pub struct InitiativeStatusUpdatePayload {
    pub success: bool,
    pub initiative_update: InitiativeStatusUpdate,
}

/// Writes a status update (Linear's `InitiativeUpdate`) on an initiative.
pub fn initiative_update_create(
    input: InitiativeStatusUpdateCreateInput,
) -> Operation<InitiativeUpdateCreate, InitiativeUpdateCreateVars> {
    InitiativeUpdateCreate::build(InitiativeUpdateCreateVars { input })
}

nodes_container!(
    InitiativeStatusUpdateNodes,
    "InitiativeUpdateConnection",
    InitiativeStatusUpdate
);

/// The latest status updates of an initiative (50 at most, newest first).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Initiative")]
pub struct InitiativeStatusUpdates {
    #[arguments(first: 50)]
    pub initiative_updates: InitiativeStatusUpdateNodes,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct InitiativeStatusUpdatesQuery {
    #[arguments(id: $id)]
    pub initiative: InitiativeStatusUpdates,
}

pub fn initiative_status_updates(
    initiative_id: impl Into<String>,
) -> Operation<InitiativeStatusUpdatesQuery, IdVars> {
    InitiativeStatusUpdatesQuery::build(IdVars {
        id: initiative_id.into(),
    })
}
