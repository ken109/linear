//! What `label create` and `label update` need: the labels with the fields a
//! write looks at, the mutations, and the checks that run before anything is sent.
//!
//! The fixed [`Label`] fragment (what `label list` and an issue's `labels`
//! print) has no team and no description, so a write reads the labels as a
//! [`LabelDetail`] instead. Names are unique **across** scopes: a label of a
//! team may not take the name of a workspace label, and Linear refuses a
//! second label with the same name whatever its group or case. A label sits in
//! a group of its own team (or of the workspace, when it is a workspace
//! label), and a group cannot be put in another group. [`check_group`] and
//! [`find_taken`] say so before a request is made.

use crate::error::{Error, Result};
use crate::inputs::Patch;
use crate::matching::label_path;
use crate::nodes::paged_container;
use crate::schema;
use crate::types::{Label, LabelGroup, LabelGroupType, Team};
use cynic::{MutationBuilder, Operation, QueryBuilder};
use schemars::JsonSchema;
use serde::Serialize;

// ---------------------------------------------------------------- reading

/// A label with everything a label write has to compare against.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "IssueLabel")]
#[serde(rename_all = "camelCase")]
pub struct LabelDetail {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    pub is_group: bool,
    /// The selection mode of a group; `None` for a plain label.
    pub group_type: Option<LabelGroupType>,
    pub parent: Option<LabelGroup>,
    /// The team the label belongs to; `None` for a workspace label.
    pub team: Option<Team>,
}

paged_container!(LabelDetailConnection, "IssueLabelConnection", LabelDetail);

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct LabelDetailsVars {
    pub first: i32,
    pub after: Option<String>,
}

impl LabelDetailsVars {
    pub fn new(page: crate::types::PageVars) -> Self {
        Self {
            first: page.first,
            after: page.after,
        }
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "LabelDetailsVars")]
pub struct LabelDetails {
    #[arguments(first: $first, after: $after)]
    pub issue_labels: LabelDetailConnection,
}

pub const LABEL_DETAILS_PAGE_SIZE: i32 = 100;

/// Every label, with the fields a write compares against.
pub fn label_details(vars: LabelDetailsVars) -> Operation<LabelDetails, LabelDetailsVars> {
    LabelDetails::build(vars)
}

impl LabelDetail {
    /// The same label as the fixed fragment, for the code that matches by name.
    pub fn to_label(&self) -> Label {
        Label {
            id: self.id.clone(),
            name: self.name.clone(),
            color: self.color.clone(),
            is_group: self.is_group,
            parent: self.parent.clone(),
        }
    }

    /// `group/name` for a label inside a group, otherwise just the name.
    pub fn path(&self) -> String {
        label_path(&self.to_label())
    }

    /// The team key, or `workspace` for a workspace label.
    pub fn scope(&self) -> &str {
        self.team.as_ref().map_or("workspace", |t| t.key.as_str())
    }

    fn team_id(&self) -> Option<&str> {
        self.team.as_ref().map(|t| t.id.inner())
    }
}

// ---------------------------------------------------------------- resolving

/// One label by `group/name` path, id or name (ignoring case).
///
/// The same name can exist in several teams, and a name matching more than
/// one label is a usage error that lists them with their ids.
pub fn resolve<'a>(all: &'a [LabelDetail], reference: &str) -> Result<&'a LabelDetail> {
    let reference = reference.trim();
    let fixed: Vec<Label> = all.iter().map(LabelDetail::to_label).collect();
    let by_path: Vec<&LabelDetail> = all
        .iter()
        .filter(|l| l.parent.is_some() && l.path().eq_ignore_ascii_case(reference))
        .collect();
    let hits: Vec<&LabelDetail> = if by_path.is_empty() {
        crate::matching::match_labels(&fixed, reference)?
            .into_iter()
            .filter_map(|m| all.iter().find(|l| l.id == m.id))
            .collect()
    } else {
        by_path
    };
    match hits.as_slice() {
        [one] => Ok(one),
        many => Err(Error::Usage(format!(
            "label {reference:?} is ambiguous; it matches {} labels ({}); pass an id",
            many.len(),
            many.iter()
                .map(|l| format!("{} [{}, {}]", l.path(), l.scope(), l.id.inner()))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// The group named by `reference`, which must be one.
pub fn resolve_group<'a>(all: &'a [LabelDetail], reference: &str) -> Result<&'a LabelDetail> {
    let group = resolve(all, reference)?;
    if !group.is_group {
        return Err(Error::Usage(format!(
            "{:?} is a label, not a group; only a group can hold labels (create it with --is-group)",
            group.path()
        )));
    }
    Ok(group)
}

// ---------------------------------------------------------------- checks

/// May a label of team `team` (`None`: the workspace) be put in `group`?
///
/// `label_is_group` is `true` for a group, which cannot be put in another one.
/// Linear refuses each of these after the request; saying so first keeps the
/// message clear and sends nothing.
pub fn check_group(
    group: &LabelDetail,
    label_is_group: bool,
    team: Option<&str>,
    team_label: &str,
) -> Result<()> {
    if !group.is_group {
        return Err(Error::Usage(format!(
            "{:?} is a label, not a group; only a group can hold labels",
            group.path()
        )));
    }
    if label_is_group {
        return Err(Error::Usage(
            "a group cannot be put in another group (groups have one level)".to_owned(),
        ));
    }
    if group.team_id() != team {
        return Err(Error::Usage(format!(
            "the group {:?} belongs to {}, but the label belongs to {team_label}; a label goes in a group of its own team",
            group.path(),
            match group.team_id() {
                Some(_) => format!("team {}", group.scope()),
                None => "the workspace".to_owned(),
            },
        )));
    }
    Ok(())
}

/// The label that already uses `name` where a label of `team` (`None`: the
/// workspace) would be made, ignoring case, other than `except` (the label being
/// renamed).
///
/// A workspace label takes its name from every team, and a team's label from
/// the workspace and from its own team, so those are the ones that collide.
pub fn find_taken<'a>(
    all: &'a [LabelDetail],
    name: &str,
    team: Option<&str>,
    except: Option<&str>,
) -> Option<&'a LabelDetail> {
    all.iter().find(|l| {
        l.name.eq_ignore_ascii_case(name)
            && except != Some(l.id.inner())
            && match (team, l.team_id()) {
                // A workspace label collides with every label.
                (None, _) => true,
                // A team's label collides with workspace labels and its own team's.
                (Some(_), None) => true,
                (Some(a), Some(b)) => a == b,
            }
    })
}

/// A color as Linear takes it: `#RRGGBB`.
pub fn check_color(color: &str) -> Result<String> {
    let c = color.trim();
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if ok {
        Ok(c.to_owned())
    } else {
        Err(Error::Usage(format!(
            "{color:?} is not a color; use a hex color such as #4EA7FC"
        )))
    }
}

// ---------------------------------------------------------------- mutations

#[derive(cynic::InputObject, Debug, Clone)]
#[cynic(graphql_type = "IssueLabelCreateInput")]
#[cynic(rename_all = "camelCase")]
pub struct LabelCreateInput {
    pub name: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Left out for a workspace label.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_group: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub group_type: Option<LabelGroupType>,
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct LabelCreateVars {
    pub input: LabelCreateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "LabelCreateVars")]
pub struct LabelCreate {
    #[arguments(input: $input)]
    pub issue_label_create: LabelPayload,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "IssueLabelPayload")]
pub struct LabelPayload {
    pub success: bool,
    pub issue_label: LabelDetail,
}

pub fn label_create(input: LabelCreateInput) -> Operation<LabelCreate, LabelCreateVars> {
    LabelCreate::build(LabelCreateVars { input })
}

#[derive(cynic::InputObject, Debug, Clone, Default)]
#[cynic(graphql_type = "IssueLabelUpdateInput")]
#[cynic(rename_all = "camelCase")]
pub struct LabelUpdateInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// An empty string clears the description.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `Patch::Clear` takes the label out of its group.
    #[cynic(skip_serializing_if = "Patch::is_keep")]
    pub parent_id: Patch<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub group_type: Option<LabelGroupType>,
}

impl LabelUpdateInput {
    /// `true` when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.color.is_none()
            && self.description.is_none()
            && self.parent_id.is_keep()
            && self.group_type.is_none()
    }
}

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct LabelUpdateVars {
    pub id: String,
    pub input: LabelUpdateInput,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "LabelUpdateVars")]
pub struct LabelUpdate {
    #[arguments(id: $id, input: $input)]
    pub issue_label_update: LabelPayload,
}

pub fn label_update(
    id: impl Into<String>,
    input: LabelUpdateInput,
) -> Operation<LabelUpdate, LabelUpdateVars> {
    LabelUpdate::build(LabelUpdateVars {
        id: id.into(),
        input,
    })
}
