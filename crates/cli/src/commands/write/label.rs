//! `linear label create|update`.
//!
//! A label belongs to a team or to the whole workspace, not to a project or an
//! issue, so no ownership rule applies (Linear itself decides who may add a
//! workspace label). What is checked before anything is sent: a label needs a
//! name that no other label uses (Linear keeps names unique across the
//! workspace and its teams, whatever the case), a color is `#RRGGBB`, a group
//! is an existing group of the label's own team, and a group is not itself put
//! in a group. A label that already exists in the place asked for is returned
//! instead of made again.
//!
//! The `label-groups-exclusive` validator rule joins in on `update`: moving a
//! label into a single-select group, or making a group single-select, is
//! refused (exit 5) when an issue that is already in Linear would end up with
//! two labels of that group. A label that is created is on no issue, so a
//! creation has nothing to hold against the rule.

use super::dry_run::{Plan, Target};
use super::{resolve, WriteSession};
use crate::commands::listing::{paginate, Listing};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::{Args, ValueEnum};
use linear_core::filters::{issues_with_label, issues_with_label_of_group};
use linear_core::inputs::Patch;
use linear_core::label_write::{
    self, check_color, check_group, find_taken, label_create, label_update, resolve_group,
    LabelCreate, LabelCreateInput, LabelDetail, LabelDetails, LabelDetailsVars, LabelUpdate,
    LabelUpdateInput, LABEL_DETAILS_PAGE_SIZE,
};
use linear_core::read::{self, IssueList, IssueListVars, ISSUE_LIST_PAGE_SIZE};
use linear_core::rules::label_groups::{regroup_violations, Regroup};
use linear_core::rules::Rejected;
use linear_core::types::{Issue, LabelGroup, LabelGroupType};
use serde::Serialize;

/// How many issues the exclusivity check reads at most. More than that and the
/// check says it looked at the first ones only.
const REGROUP_ISSUES_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GroupTypeArg {
    /// An issue may have only one label of the group
    SingleSelect,
    /// An issue may have several labels of the group
    MultiSelect,
}

impl GroupTypeArg {
    fn to_linear(self) -> LabelGroupType {
        match self {
            Self::SingleSelect => LabelGroupType::SingleSelect,
            Self::MultiSelect => LabelGroupType::MultiSelect,
        }
    }
}

// ---------------------------------------------------------------- create

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// The label's name; names are unique across the workspace and its teams, so one that is
    /// already used in the same place with the same group is returned instead of made again
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// Team key; without it the label belongs to the whole workspace (or to its group's team)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Put the label in this group (name, `group/name` or id; it must be a group of the same team)
    #[arg(long, value_name = "GROUP", conflicts_with = "is_group")]
    pub group: Option<String>,
    /// Make a group (a container for labels) instead of a label
    #[arg(long)]
    pub is_group: bool,
    /// With --is-group: whether an issue may have one label of the group or several
    /// (default: single-select; multi-select needs a workspace that has it enabled)
    #[arg(long, value_enum, requires = "is_group", value_name = "MODE")]
    pub group_type: Option<GroupTypeArg>,
    /// Color as `#RRGGBB` (default: Linear picks one)
    #[arg(long, value_name = "COLOR")]
    pub color: Option<String>,
    /// A short description
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
}

/// What `create` prints: the label, and whether it was already there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    /// `true` when the label already existed here and nothing was created.
    existing: bool,
    #[serde(flatten)]
    label: &'a LabelDetail,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let name = non_blank(&cmd.name, "--name")?;
    let color = cmd.color.as_deref().map(check_color).transpose()?;
    let description = cmd
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned);

    let ws = ctx.write_session()?;
    let all = all_labels(&ws)?;

    // The team: asked for, else the group's, else none (a workspace label).
    let asked_team = cmd
        .team
        .as_deref()
        .map(|key| resolve::team(&ws, Some(key)))
        .transpose()?;
    let group = cmd
        .group
        .as_deref()
        .map(|g| resolve_group(&all, g))
        .transpose()?;
    let team_id: Option<String> = match (&asked_team, group) {
        (Some(t), _) => Some(t.id.inner().to_owned()),
        (None, Some(g)) => g.team.as_ref().map(|t| t.id.inner().to_owned()),
        (None, None) => None,
    };
    let team_label = match &asked_team {
        Some(t) => format!("team {}", t.key),
        None => "the workspace".to_owned(),
    };
    if let Some(g) = group {
        check_group(g, cmd.is_group, team_id.as_deref(), &team_label)?;
    }

    if let Some(taken) = find_taken(&all, &name, team_id.as_deref(), None) {
        let same_place = taken.team.as_ref().map(|t| t.id.inner()) == team_id.as_deref()
            && taken.parent.as_ref().map(|p| p.id.inner()) == group.map(|g| g.id.inner())
            && taken.is_group == cmd.is_group;
        if same_place {
            if ws.dry_run {
                return ws.finish_dry_run(
                    Plan::new(
                        "label create",
                        Target::existing("label", taken.path(), taken.id.inner()),
                    )
                    .reason("the label already exists here, so none is created"),
                );
            }
            emit_created(ctx, &ws, taken, true);
            return Ok(());
        }
        return Err(CliError::usage(format!(
            "a label named {:?} already exists ({}, {}); Linear keeps label names unique across the workspace and its teams",
            taken.name,
            taken.path(),
            taken.scope()
        )));
    }

    let label_name = name.clone();
    let op = label_create(LabelCreateInput {
        name,
        color,
        description,
        team_id,
        parent_id: group.map(|g| g.id.inner().to_owned()),
        is_group: cmd.is_group.then_some(true),
        group_type: cmd.group_type.map(GroupTypeArg::to_linear),
    });
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new(
            "label create",
            Target::new(
                if cmd.is_group { "label group" } else { "label" },
                label_name,
            ),
        ));
    }
    let data: LabelCreate = ws.client.execute(&op)?;
    let payload = data.issue_label_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the label"));
    }
    emit_created(ctx, &ws, &payload.issue_label, false);
    Ok(())
}

fn emit_created(ctx: &Ctx, ws: &WriteSession, label: &LabelDetail, existing: bool) {
    let value = Created {
        workspace: &ws.workspace,
        existing,
        label,
    };
    ctx.out.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!("{}  ({how})", summary(label))
        },
        || label.path(),
    );
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// The label: name (ignoring case), `group/name` or id
    pub label: String,
    /// Rename it; refused when another label already has that name
    #[arg(long, value_name = "NAME")]
    pub new_name: Option<String>,
    /// New color as `#RRGGBB`
    #[arg(long, value_name = "COLOR")]
    pub color: Option<String>,
    /// New description (an empty value clears it)
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    /// Move the label into this group (name, `group/name` or id; a group of the label's own team)
    #[arg(long, value_name = "GROUP", conflicts_with = "no_group")]
    pub group: Option<String>,
    /// Take the label out of its group
    #[arg(long)]
    pub no_group: bool,
    /// Change a group's selection mode (the label must be a group)
    #[arg(long, value_enum, value_name = "MODE")]
    pub group_type: Option<GroupTypeArg>,
}

/// What `update` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated<'a> {
    workspace: &'a str,
    /// `false` when every value asked for was already the current one, so nothing was sent.
    changed: bool,
    #[serde(flatten)]
    label: &'a LabelDetail,
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.new_name.is_none()
        && cmd.color.is_none()
        && cmd.description.is_none()
        && cmd.group.is_none()
        && !cmd.no_group
        && cmd.group_type.is_none()
    {
        return Err(CliError::usage(
            "nothing to change: pass --new-name, --color, --description, --group, --no-group or --group-type",
        ));
    }
    let new_name = cmd
        .new_name
        .as_deref()
        .map(|n| non_blank(n, "--new-name"))
        .transpose()?;
    let color = cmd.color.as_deref().map(check_color).transpose()?;

    let ws = ctx.write_session()?;
    let all = all_labels(&ws)?;
    let current = label_write::resolve(&all, &cmd.label)?;
    let team_id = current.team.as_ref().map(|t| t.id.inner());
    if cmd.group_type.is_some() && !current.is_group {
        return Err(CliError::usage(format!(
            "{:?} is a label, not a group; --group-type is for groups",
            current.path()
        )));
    }

    // Only what differs from now is sent.
    let mut input = LabelUpdateInput::default();
    if let Some(name) = new_name.filter(|n| *n != current.name) {
        if let Some(taken) = find_taken(&all, &name, team_id, Some(current.id.inner())) {
            return Err(CliError::usage(format!(
                "a label named {:?} already exists ({}, {}); Linear keeps label names unique across the workspace and its teams",
                taken.name,
                taken.path(),
                taken.scope()
            )));
        }
        input.name = Some(name);
    }
    input.color = color.filter(|c| !c.eq_ignore_ascii_case(&current.color));
    input.description = cmd.description.as_deref().map(str::trim).and_then(|d| {
        (d != current.description.as_deref().unwrap_or("").trim()).then(|| d.to_owned())
    });

    let mut target_group: Option<&LabelDetail> = None;
    if let Some(reference) = cmd.group.as_deref() {
        let group = resolve_group(&all, reference)?;
        check_group(group, current.is_group, team_id, &scope_words(current))?;
        if current.parent.as_ref().map(|p| p.id.inner()) != Some(group.id.inner()) {
            input.parent_id = Patch::Set(group.id.inner().to_owned());
            target_group = Some(group);
        }
    } else if cmd.no_group && current.parent.is_some() {
        input.parent_id = Patch::Clear;
    }
    let new_type = cmd.group_type.map(GroupTypeArg::to_linear);
    input.group_type = new_type
        .clone()
        .filter(|t| current.group_type.as_ref() != Some(t));

    if input.is_empty() {
        if ws.dry_run {
            return ws.finish_dry_run(
                Plan::new(
                    "label update",
                    Target::existing("label", current.path(), current.id.inner()),
                )
                .reason("the label already has these values"),
            );
        }
        ws.note("nothing to change: the label already has these values");
        emit_updated(ctx, &ws, current, false);
        return Ok(());
    }
    let mut changed: Vec<&'static str> = Vec::new();
    if input.name.is_some() {
        changed.push("name");
    }
    if input.color.is_some() {
        changed.push("color");
    }
    if input.description.is_some() {
        changed.push("description");
    }
    if !matches!(input.parent_id, Patch::Keep) {
        changed.push("group");
    }
    if input.group_type.is_some() {
        changed.push("groupType");
    }

    // The label-groups-exclusive rule, asked about the labels that are already on issues.
    if ws
        .rules
        .enabled(linear_core::config::Rule::LabelGroupsExclusive)
    {
        let moved_into = target_group.map(|g| LabelGroup {
            id: g.id.clone(),
            name: g.name.clone(),
            group_type: g.group_type.clone(),
        });
        // A multi-select group never conflicts, so there is nothing to look up for one.
        let change = match (&moved_into, &input.group_type) {
            (Some(group), _) if group.group_type != Some(LabelGroupType::MultiSelect) => Some((
                issues_with_label(current.id.inner()),
                Regroup::Move {
                    label_id: current.id.inner(),
                    group,
                },
            )),
            (None, Some(group_type)) if *group_type != LabelGroupType::MultiSelect => Some((
                issues_with_label_of_group(current.id.inner()),
                Regroup::Retype {
                    group_id: current.id.inner(),
                    group_type,
                },
            )),
            _ => None,
        };
        if let Some((filter, change)) = change {
            let issues = issues_carrying(&ws, filter)?;
            if issues.truncated {
                ws.note(&format!(
                    "label-groups-exclusive: only the first {REGROUP_ISSUES_LIMIT} issues that carry the label were checked"
                ));
            }
            if let Some(rejected) = Rejected::new(regroup_violations(&issues.items, &change)) {
                return Err(rejected.into());
            }
        }
    }

    let op = label_update(current.id.inner(), input);
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(
            Plan::new(
                "label update",
                Target::existing("label", current.path(), current.id.inner()),
            )
            .changed(changed),
        );
    }
    let data: LabelUpdate = ws.client.execute(&op)?;
    let payload = data.issue_label_update;
    if !payload.success {
        return Err(CliError::general("Linear could not update the label"));
    }
    emit_updated(ctx, &ws, &payload.issue_label, true);
    Ok(())
}

fn emit_updated(ctx: &Ctx, ws: &WriteSession, label: &LabelDetail, changed: bool) {
    let value = Updated {
        workspace: &ws.workspace,
        changed,
        label,
    };
    ctx.out.emit(
        &value,
        || {
            format!(
                "{}  ({})",
                summary(label),
                if changed { "updated" } else { "unchanged" }
            )
        },
        || label.path(),
    );
}

// ---------------------------------------------------------------- shared

/// Every label, with team and description.
fn all_labels(ws: &WriteSession) -> Result<Vec<LabelDetail>> {
    Ok(paginate(LABEL_DETAILS_PAGE_SIZE, None, |page| {
        let data: LabelDetails = ws
            .client
            .execute(&label_write::label_details(LabelDetailsVars::new(page)))?;
        Ok(data.issue_labels)
    })?
    .items)
}

/// The issues (up to a limit) that match a label filter, with their labels.
fn issues_carrying(
    ws: &WriteSession,
    filter: linear_core::filters::IssueFilter,
) -> Result<Listing<Issue>> {
    paginate(ISSUE_LIST_PAGE_SIZE, Some(REGROUP_ISSUES_LIMIT), |page| {
        let vars = IssueListVars::new(page, Some(filter.clone()));
        let data: IssueList = ws.client.execute(&read::issue_list(vars))?;
        Ok(data.issues)
    })
}

/// `area/api  label  #4EA7FC  workspace`
fn summary(label: &LabelDetail) -> String {
    format!(
        "{}  {}  {}  {}",
        label.path(),
        if label.is_group { "group" } else { "label" },
        label.color,
        label.scope()
    )
}

fn scope_words(label: &LabelDetail) -> String {
    match &label.team {
        Some(t) => format!("team {}", t.key),
        None => "the workspace".to_owned(),
    }
}

fn non_blank(value: &str, flag: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CliError::usage(format!("{flag} must not be empty")))
    } else {
        Ok(trimmed.to_owned())
    }
}
