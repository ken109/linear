//! Mutation inputs and the mutations that carry them.
//!
//! Rule: every `Option` field carries `skip_serializing_if = "Option::is_none"`.
//! cynic otherwise sends `null` for `None`, and Linear treats an explicit
//! `null` as "clear this field", which would turn a partial update into a
//! destructive one. An input with every field `None` therefore serializes as
//! `{}`. A field that must be able to say "clear" explicitly uses [`Patch`].
//!
//! Only the inputs the CLI needs so far are defined; cynic input structs may
//! list a subset of the schema's fields.

use crate::metadata::AttachmentMetadata;
use crate::schema;
use chrono::NaiveDate;
use cynic::MutationBuilder;
use serde::{Serialize, Serializer};

/// A field of an update that can be left alone, set, or cleared.
///
/// `Option` cannot say all three: `None` has to mean "leave alone" (see the
/// module rule), so clearing needs its own value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    /// Leave the field as it is (omitted from the request).
    #[default]
    Keep,
    /// Set the field to `null`.
    Clear,
    Set(T),
}

impl<T> Patch<T> {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Keep | Self::Clear => s.serialize_none(),
            Self::Set(v) => v.serialize(s),
        }
    }
}

/// cynic checks a field's Rust type against the schema's scalar, not against
/// its nullability, so a `Patch<T>` stands in for the `T` it may carry.
impl<T, U> cynic::schema::IsScalar<T> for Patch<U>
where
    U: cynic::schema::IsScalar<T>,
{
    type SchemaType = U::SchemaType;
}

// ---------------------------------------------------------------- issue update

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "IssueUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct IssueUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The description (markdown). `Patch::Clear` empties it.
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub description: Patch<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub assignee_id: Patch<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub project_id: Patch<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub project_milestone_id: Patch<String>,
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub due_date: Patch<NaiveDate>,
    /// The complete set of labels the issue ends up with (replaces the old
    /// set; an empty list removes them all).
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<f64>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub priority_sort_order: Option<f64>,
    /// Put the issue in this cycle (never sent as a clear: nothing here takes one out).
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
}

impl IssueUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_keep()
            && self.state_id.is_none()
            && self.assignee_id.is_keep()
            && self.project_id.is_keep()
            && self.project_milestone_id.is_keep()
            && self.due_date.is_keep()
            && self.label_ids.is_none()
            && self.sort_order.is_none()
            && self.priority_sort_order.is_none()
            && self.cycle_id.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IssueUpdateVars {
    pub id: String,
    pub input: IssueUpdateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "IssueUpdateVars")]
pub struct IssueUpdate {
    #[arguments(id: $id, input: $input)]
    pub issue_update: IssuePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct IssuePayload {
    pub success: bool,
    pub issue: Option<crate::types::Issue>,
}

pub fn issue_update(
    id: impl Into<String>,
    input: IssueUpdateInput,
) -> cynic::Operation<IssueUpdate, IssueUpdateVars> {
    IssueUpdate::build(IssueUpdateVars {
        id: id.into(),
        input,
    })
}

// ---------------------------------------------------------------- issue create

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "IssueCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct IssueCreateInput {
    pub team_id: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IssueCreateVars {
    pub input: IssueCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "IssueCreateVars")]
pub struct IssueCreate {
    #[arguments(input: $input)]
    pub issue_create: IssuePayload,
}

pub fn issue_create(input: IssueCreateInput) -> cynic::Operation<IssueCreate, IssueCreateVars> {
    IssueCreate::build(IssueCreateVars { input })
}

// ---------------------------------------------------------------- issue delete

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct IssueDeleteVars {
    pub id: String,
}

/// Trashes an issue (Linear keeps it for 30 days). Used to roll back a
/// half-made issue.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "IssueDeleteVars")]
pub struct IssueDelete {
    #[arguments(id: $id)]
    pub issue_delete: ArchivePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "IssueArchivePayload")]
pub struct ArchivePayload {
    pub success: bool,
}

pub fn issue_delete(id: impl Into<String>) -> cynic::Operation<IssueDelete, IssueDeleteVars> {
    IssueDelete::build(IssueDeleteVars { id: id.into() })
}

// ---------------------------------------------------------------- attachment

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "AttachmentCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct AttachmentCreateInput {
    pub issue_id: String,
    pub url: String,
    pub title: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Linear upserts on `url`: sending the URL of an existing attachment
    /// replaces its title, subtitle and metadata.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AttachmentMetadata>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct AttachmentCreateVars {
    pub input: AttachmentCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "AttachmentCreateVars")]
pub struct AttachmentCreate {
    #[arguments(input: $input)]
    pub attachment_create: AttachmentPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct AttachmentPayload {
    pub success: bool,
    pub attachment: crate::types::Attachment,
}

pub fn attachment_create(
    input: AttachmentCreateInput,
) -> cynic::Operation<AttachmentCreate, AttachmentCreateVars> {
    AttachmentCreate::build(AttachmentCreateVars { input })
}

// ---------------------------------------------------------------- comment

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "CommentCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct CommentCreateInput {
    pub issue_id: String,
    pub body: String,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct CommentCreateVars {
    pub input: CommentCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "CommentCreateVars")]
pub struct CommentCreate {
    #[arguments(input: $input)]
    pub comment_create: CommentPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct CommentPayload {
    pub success: bool,
    pub comment: crate::types::Comment,
}

pub fn comment_create(
    input: CommentCreateInput,
) -> cynic::Operation<CommentCreate, CommentCreateVars> {
    CommentCreate::build(CommentCreateVars { input })
}

// ---------------------------------------------------------------- milestone

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "ProjectMilestoneCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct MilestoneCreateInput {
    pub project_id: String,
    pub name: String,
    /// Always sent: a milestone without a target date does not show on the timeline.
    pub target_date: NaiveDate,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct MilestoneCreateVars {
    pub input: MilestoneCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "MilestoneCreateVars")]
pub struct MilestoneCreate {
    #[arguments(input: $input)]
    pub project_milestone_create: MilestonePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "ProjectMilestonePayload")]
pub struct MilestonePayload {
    pub success: bool,
    pub project_milestone: crate::types::Milestone,
}

pub fn milestone_create(
    input: MilestoneCreateInput,
) -> cynic::Operation<MilestoneCreate, MilestoneCreateVars> {
    MilestoneCreate::build(MilestoneCreateVars { input })
}

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "ProjectMilestoneUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct MilestoneUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<NaiveDate>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl MilestoneUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.target_date.is_none() && self.description.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct MilestoneUpdateVars {
    pub id: String,
    pub input: MilestoneUpdateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "MilestoneUpdateVars")]
pub struct MilestoneUpdate {
    #[arguments(id: $id, input: $input)]
    pub project_milestone_update: MilestonePayload,
}

pub fn milestone_update(
    id: impl Into<String>,
    input: MilestoneUpdateInput,
) -> cynic::Operation<MilestoneUpdate, MilestoneUpdateVars> {
    MilestoneUpdate::build(MilestoneUpdateVars {
        id: id.into(),
        input,
    })
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct MilestoneDeleteVars {
    pub id: String,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "MilestoneDeleteVars")]
pub struct MilestoneDelete {
    #[arguments(id: $id)]
    pub project_milestone_delete: DeleteResult,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "DeletePayload")]
pub struct DeleteResult {
    pub success: bool,
}

pub fn milestone_delete(
    id: impl Into<String>,
) -> cynic::Operation<MilestoneDelete, MilestoneDeleteVars> {
    MilestoneDelete::build(MilestoneDeleteVars { id: id.into() })
}

// ---------------------------------------------------------------- initiative

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "InitiativeCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct InitiativeCreateInput {
    pub name: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct InitiativeCreateVars {
    pub input: InitiativeCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "InitiativeCreateVars")]
pub struct InitiativeCreate {
    #[arguments(input: $input)]
    pub initiative_create: InitiativePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct InitiativePayload {
    pub success: bool,
    pub initiative: crate::types::Initiative,
}

pub fn initiative_create(
    input: InitiativeCreateInput,
) -> cynic::Operation<InitiativeCreate, InitiativeCreateVars> {
    InitiativeCreate::build(InitiativeCreateVars { input })
}

// ---------------------------------------------------------------- template

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "TemplateCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct TemplateCreateInput {
    /// The kind of template: `issue`, `project` or `document`.
    #[cynic(rename = "type")]
    pub kind: String,
    pub name: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// For an issue template: `{ "title": "", "descriptionData": <ProseMirror doc> }`.
    pub template_data: serde_json::Value,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct TemplateCreateVars {
    pub input: TemplateCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "TemplateCreateVars")]
pub struct TemplateCreate {
    #[arguments(input: $input)]
    pub template_create: TemplatePayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
pub struct TemplatePayload {
    pub success: bool,
    pub template: crate::types::Template,
}

pub fn template_create(
    input: TemplateCreateInput,
) -> cynic::Operation<TemplateCreate, TemplateCreateVars> {
    TemplateCreate::build(TemplateCreateVars { input })
}
