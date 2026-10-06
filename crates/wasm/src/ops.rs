//! The operations `build_request` and `parse_response` know by name.
//!
//! One table says, for each operation, what its parameters are, which query it
//! builds and which type its result has. Adding an operation is one row here:
//! the request building, the response parsing and the JSON Schema of both all
//! come from the row, so the three cannot drift apart.

use chrono::{DateTime, Utc};
use linear_core::queries::{self, ASSIGNED_STARTED_PAGE_SIZE, INITIATIVES_PAGE_SIZE};
use linear_core::read::{
    self, InitiativeView, IssueView, Labels, MilestoneView, MilestonesOfProject, ProjectView,
    Teams, UserListVars, Users, LABELS_PAGE_SIZE, TEAMS_PAGE_SIZE, USERS_PAGE_SIZE,
};
use linear_core::types::PageVars;
use linear_core::wire::{self, Request, ResponseMeta};
use linear_core::Result;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// No parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// One page of a listing. Both fields may be left out: the first page, at the
/// size that stays under Linear's query-complexity limit for that listing.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageParams {
    /// How many items to ask for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<i32>,
    /// The `endCursor` of the previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

impl PageParams {
    fn vars(self, default_first: i32) -> PageVars {
        PageVars {
            first: self.first.unwrap_or(default_first),
            after: self.after,
        }
    }
}

/// An issue, project, milestone or initiative: its id, or for issues and
/// projects its identifier (`KK-12`, a project slug).
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdParams {
    pub id: String,
}

/// A page of users.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsersParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Also list users who have been disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_disabled: Option<bool>,
}

/// One row of the table.
pub struct Operation {
    pub name: &'static str,
    /// Parameters (JSON text) to the request to send.
    build: fn(&str) -> std::result::Result<Request, String>,
    /// The response to the result, as JSON.
    parse: fn(&ResponseMeta, &str, DateTime<Utc>) -> Result<Value>,
    /// The schema of the parameters and of the result, for generating types.
    #[cfg(not(target_arch = "wasm32"))]
    pub schemas: fn(&mut schemars::SchemaGenerator) -> (schemars::Schema, schemars::Schema),
}

impl Operation {
    pub fn build(&self, params_json: &str) -> std::result::Result<Request, String> {
        (self.build)(params_json)
    }

    pub fn parse(&self, meta: &ResponseMeta, body: &str, now: DateTime<Utc>) -> Result<Value> {
        (self.parse)(meta, body, now)
    }
}

pub fn find(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.name == name)
}

fn params<T: DeserializeOwned>(json: &str) -> std::result::Result<T, String> {
    let text = match json.trim() {
        "" | "null" => "{}",
        other => other,
    };
    serde_json::from_str(text).map_err(|e| format!("invalid parameters: {e}"))
}

fn result<D: Serialize>(data: D) -> Value {
    serde_json::to_value(data).expect("query results always serialize")
}

fn parse_as<D: DeserializeOwned + Serialize>(
    meta: &ResponseMeta,
    body: &str,
    now: DateTime<Utc>,
) -> Result<Value> {
    wire::parse_response::<D>(meta, body, now).map(result)
}

#[cfg(not(target_arch = "wasm32"))]
fn schemas_of<P: JsonSchema, D: JsonSchema>(
    generator: &mut schemars::SchemaGenerator,
) -> (schemars::Schema, schemars::Schema) {
    (
        generator.subschema_for::<P>(),
        generator.subschema_for::<D>(),
    )
}

/// `"name" => Params, |p| operation -> Data;`
macro_rules! operations {
    ($($name:literal => $params:ty, |$p:ident| $op:expr => $data:ty;)+) => {
        pub static OPERATIONS: &[Operation] = &[$(
            Operation {
                name: $name,
                build: |json| {
                    let $p: $params = params(json)?;
                    Ok(wire::build_request(&$op))
                },
                parse: parse_as::<$data>,
                #[cfg(not(target_arch = "wasm32"))]
                schemas: schemas_of::<$params, $data>,
            },
        )+];
    };
}

operations! {
    "whoami" => NoParams, |_p| queries::whoami() => queries::Whoami;
    "assigned_started_issues" => PageParams,
        |p| queries::assigned_started_issues(p.vars(ASSIGNED_STARTED_PAGE_SIZE))
        => queries::AssignedStartedIssues;
    "issue" => IdParams, |p| queries::issue(p.id) => queries::IssueById;
    "issue_view" => IdParams, |p| read::issue_view(p.id) => IssueView;
    "issue_comments" => IdParams, |p| queries::issue_comments(p.id) => queries::IssueCommentsQuery;
    "projects" => PageParams,
        |p| queries::projects(p.vars(queries::PROJECTS_PAGE_SIZE)) => queries::Projects;
    "project_view" => IdParams, |p| read::project_view(p.id) => ProjectView;
    "milestones_of_project" => IdParams,
        |p| read::milestones_of_project(p.id) => MilestonesOfProject;
    "milestone_view" => IdParams, |p| read::milestone_view(p.id) => MilestoneView;
    "initiatives" => PageParams,
        |p| queries::initiatives(p.vars(INITIATIVES_PAGE_SIZE)) => queries::Initiatives;
    "initiative_view" => IdParams, |p| read::initiative_view(p.id) => InitiativeView;
    "labels" => PageParams, |p| read::labels(p.vars(LABELS_PAGE_SIZE)) => Labels;
    "teams" => PageParams, |p| read::teams(p.vars(TEAMS_PAGE_SIZE)) => Teams;
    "users" => UsersParams,
        |p| read::users(UserListVars::new(
            PageVars { first: p.first.unwrap_or(USERS_PAGE_SIZE), after: p.after },
            p.include_disabled.unwrap_or(false),
        )) => Users;
    "templates" => NoParams, |_p| queries::templates() => queries::Templates;
}
