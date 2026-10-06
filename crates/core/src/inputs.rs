//! Mutation inputs.
//!
//! Rule: every `Option` field carries `skip_serializing_if = "Option::is_none"`.
//! cynic otherwise sends `null` for `None`, and Linear treats an explicit
//! `null` as "clear this field", which would turn a partial update into a
//! destructive one. An input with every field `None` therefore serializes as
//! `{}`.
//!
//! Only the inputs the CLI needs so far are defined; cynic input structs may
//! list a subset of the schema's fields.

use crate::schema;
use chrono::NaiveDate;
use cynic::MutationBuilder;

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "IssueUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct IssueUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<NaiveDate>,
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
