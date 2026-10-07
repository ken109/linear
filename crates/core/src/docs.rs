//! Linear documents (`document list|view|create|update`).
//!
//! A document hangs off a project or an initiative (the only two parents the
//! CLI writes under); it can also hang off an issue, a team or a cycle, which
//! the CLI reads but does not write. The body is markdown.
//!
//! This module is called `docs` because [`crate::document`] is the scanner for
//! GraphQL documents.

use crate::filters::{EntityIdComparator, StringComparator};
use crate::nodes::paged_container;
use crate::read::{IdVars, IdVarsFields};
use crate::schema;
use crate::types::{InitiativeRef, IssueRef, PageVars, ProjectRef, User};
use chrono::{DateTime, Utc};
use cynic::{MutationBuilder, Operation, QueryBuilder};
use schemars::JsonSchema;
use serde::Serialize;

// ---------------------------------------------------------------- the entity

/// A document, without its body (a listing does not pay for the bodies).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Document")]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    pub slug_id: String,
    pub title: String,
    pub url: String,
    pub icon: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub creator: Option<User>,
    pub project: Option<ProjectRef>,
    pub initiative: Option<InitiativeRef>,
    pub issue: Option<IssueRef>,
}

impl Doc {
    /// What the document hangs off, for a listing: `project: Name`, `initiative: Name`,
    /// `issue: KK-1`, or `-`.
    pub fn parent_label(&self) -> String {
        if let Some(p) = &self.project {
            format!("project: {}", p.name)
        } else if let Some(i) = &self.initiative {
            format!("initiative: {}", i.name)
        } else if let Some(i) = &self.issue {
            format!("issue: {}", i.identifier)
        } else {
            "-".to_owned()
        }
    }
}

paged_container!(DocConnection, "DocumentConnection", Doc);

/// What a `view` shows beyond the document's fixed fragment.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Document")]
pub struct DocDetail {
    /// The body, as markdown.
    pub content: Option<String>,
}

// ---------------------------------------------------------------- filters

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "ProjectFilter")]
pub struct DocProjectFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntityIdComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "InitiativeFilter")]
pub struct DocInitiativeFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntityIdComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "DocumentFilter")]
pub struct DocFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project: Option<DocProjectFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub initiative: Option<DocInitiativeFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub title: Option<StringComparator>,
}

/// What `document list` can be narrowed by. Unset fields do not narrow.
#[derive(Debug, Clone, Default)]
pub struct DocQuery {
    /// The id of an already resolved project.
    pub project_id: Option<String>,
    /// The id of an already resolved initiative.
    pub initiative_id: Option<String>,
    /// A title, ignoring case.
    pub title: Option<String>,
}

impl DocQuery {
    /// `None` when nothing narrows the listing.
    pub fn filter(&self) -> Option<DocFilter> {
        let f = DocFilter {
            project: self.project_id.as_ref().map(|id| DocProjectFilter {
                id: Some(EntityIdComparator {
                    eq: Some(cynic::Id::new(id.clone())),
                }),
            }),
            initiative: self.initiative_id.as_ref().map(|id| DocInitiativeFilter {
                id: Some(EntityIdComparator {
                    eq: Some(cynic::Id::new(id.clone())),
                }),
            }),
            title: self
                .title
                .as_ref()
                .map(|t| StringComparator::eq_ignore_case(t.clone())),
        };
        (f != DocFilter::default()).then_some(f)
    }
}

// ---------------------------------------------------------------- reading

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct DocListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<DocFilter>,
}

impl DocListVars {
    pub fn new(page: PageVars, filter: Option<DocFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "DocListVars")]
pub struct DocList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub documents: DocConnection,
}

pub const DOC_LIST_PAGE_SIZE: i32 = 50;

pub fn doc_list(vars: DocListVars) -> Operation<DocList, DocListVars> {
    DocList::build(vars)
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Query", variables = "IdVars")]
pub struct DocView {
    #[arguments(id: $id)]
    pub document: Doc,
    #[cynic(alias, rename = "document")]
    #[arguments(id: $id)]
    pub detail: DocDetail,
}

/// `id` is a document's id, its slug id, or the `title-slugid` of its URL.
pub fn doc_view(id: impl Into<String>) -> Operation<DocView, IdVars> {
    DocView::build(IdVars { id: id.into() })
}

// ---------------------------------------------------------------- writing

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "DocumentCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct DocCreateInput {
    pub title: String,
    /// The body, as markdown.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Exactly one of the project and the initiative is set.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub initiative_id: Option<String>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct DocCreateVars {
    pub input: DocCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "DocCreateVars")]
pub struct DocCreate {
    #[arguments(input: $input)]
    pub document_create: DocPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "DocumentPayload")]
pub struct DocPayload {
    pub success: bool,
    pub document: Doc,
}

pub fn doc_create(input: DocCreateInput) -> Operation<DocCreate, DocCreateVars> {
    DocCreate::build(DocCreateVars { input })
}

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "DocumentUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct DocUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The body, as markdown.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl DocUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.content.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct DocUpdateVars {
    pub id: String,
    pub input: DocUpdateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "DocUpdateVars")]
pub struct DocUpdate {
    #[arguments(id: $id, input: $input)]
    pub document_update: DocPayload,
}

pub fn doc_update(
    id: impl Into<String>,
    input: DocUpdateInput,
) -> Operation<DocUpdate, DocUpdateVars> {
    DocUpdate::build(DocUpdateVars {
        id: id.into(),
        input,
    })
}
