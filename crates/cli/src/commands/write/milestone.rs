//! `linear milestone create|update|delete`.
//!
//! A milestone belongs to a project, so these writes follow the project's
//! ownership rule: only the project's lead may add, change or remove one
//! (exit code 4 otherwise). No validator rule applies to milestones; the
//! checks that are always on are made here before anything is sent: a
//! milestone needs a name and a target date (without one it does not show on
//! the timeline), and a project never gets two milestones with the same name.

use super::{read_text, resolve, ForceArg, WriteSession};
use crate::commands::format::{milestone_status, opt_date};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use chrono::NaiveDate;
use clap::Args;
use linear_core::guard::Write;
use linear_core::inputs::{
    self, MilestoneCreateInput, MilestoneDelete, MilestoneUpdate, MilestoneUpdateInput,
};
use linear_core::read::{self, MilestoneView, ProjectOwnership};
use linear_core::types::Milestone;
use serde::Serialize;
use std::path::PathBuf;

/// How many issues to name in the error that refuses a delete.
const LEFTOVERS_SHOWN: usize = 10;

// ---------------------------------------------------------------- create

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
    /// The milestone's name; a project already having one with this name returns it
    /// instead of creating another
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// When the milestone is due (YYYY-MM-DD); required, or it does not show on the timeline
    #[arg(long, value_name = "DATE")]
    pub target_date: NaiveDate,
    /// Read the description from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub description_file: Option<PathBuf>,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `create` prints: the milestone, and whether it was already there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    /// `true` when the project already had a milestone with this name and nothing was created.
    existing: bool,
    #[serde(flatten)]
    milestone: &'a Milestone,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let name = non_blank(&cmd.name, "--name")?;
    let description = description(cmd.description_file.as_deref())?;

    let ws = ctx.write_session_with(cmd.force)?;
    let project = resolve::project(&ws, &cmd.project)?;
    ws.guard(&target(&project), &led_by(&project), false)?;

    if let Some(existing) = project.project_milestones.iter().find(|m| m.name == name) {
        emit_created(&ws, existing, true);
        return Ok(());
    }

    let data: inputs::MilestoneCreate =
        ws.client
            .execute(&inputs::milestone_create(MilestoneCreateInput {
                project_id: project.id.inner().to_owned(),
                name,
                target_date: cmd.target_date,
                description,
            }))?;
    let payload = data.project_milestone_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the milestone"));
    }
    emit_created(&ws, &payload.project_milestone, false);
    Ok(())
}

fn emit_created(ws: &WriteSession, milestone: &Milestone, existing: bool) {
    let value = Created {
        workspace: &ws.workspace,
        existing,
        milestone,
    };
    ws.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!(
                "{}  target {}  ({how})",
                milestone.name,
                opt_date(&milestone.target_date)
            )
        },
        || milestone.name.clone(),
    );
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// The milestone's current name (ignoring case) or id
    pub milestone: String,
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
    /// Rename it; refused when another milestone of the project has that name
    #[arg(long, value_name = "NAME")]
    pub new_name: Option<String>,
    /// New target date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub target_date: Option<NaiveDate>,
    /// Replace the description with the contents of a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub description_file: Option<PathBuf>,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `update` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated<'a> {
    workspace: &'a str,
    /// `false` when every value asked for was already the current one, so nothing was sent.
    changed: bool,
    #[serde(flatten)]
    milestone: &'a Milestone,
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.new_name.is_none() && cmd.target_date.is_none() && cmd.description_file.is_none() {
        return Err(CliError::usage(
            "nothing to change: pass --new-name, --target-date or --description-file",
        ));
    }
    let new_name = cmd
        .new_name
        .as_deref()
        .map(|n| non_blank(n, "--new-name"))
        .transpose()?;
    let new_description = match cmd.description_file.as_deref() {
        Some(path) => Some(description(Some(path))?.ok_or_else(|| {
            CliError::usage(
                "--description-file is empty; a milestone's description is not cleared this way",
            )
        })?),
        None => None,
    };

    let ws = ctx.write_session_with(cmd.force)?;
    let project = resolve::project(&ws, &cmd.project)?;
    let current = resolve::milestone(&project, &cmd.milestone)?;
    ws.guard(&target(&project), &led_by(&project), false)?;

    // Only what differs from now is sent.
    let mut input = MilestoneUpdateInput::default();
    if let Some(name) = new_name.filter(|n| *n != current.name) {
        if project
            .project_milestones
            .iter()
            .any(|m| m.id != current.id && m.name == name)
        {
            return Err(CliError::usage(format!(
                "{} already has a milestone named {name:?}",
                project.name
            )));
        }
        input.name = Some(name);
    }
    input.target_date = cmd.target_date.filter(|d| Some(*d) != current.target_date);
    input.description = new_description
        .filter(|d| current.description.as_deref().map(str::trim_end) != Some(d.as_str()));

    if input.is_empty() {
        ws.note("nothing to change: the milestone already has these values");
        emit_updated(&ws, current, false);
        return Ok(());
    }

    let data: MilestoneUpdate = ws
        .client
        .execute(&inputs::milestone_update(current.id.inner(), input))?;
    let payload = data.project_milestone_update;
    if !payload.success {
        return Err(CliError::general("Linear could not update the milestone"));
    }
    emit_updated(&ws, &payload.project_milestone, true);
    Ok(())
}

fn emit_updated(ws: &WriteSession, milestone: &Milestone, changed: bool) {
    let value = Updated {
        workspace: &ws.workspace,
        changed,
        milestone,
    };
    ws.emit(
        &value,
        || {
            format!(
                "{}  {}  target {}  ({})",
                milestone.name,
                milestone_status(&milestone.status),
                opt_date(&milestone.target_date),
                if changed { "updated" } else { "unchanged" }
            )
        },
        || milestone.name.clone(),
    );
}

// ---------------------------------------------------------------- delete

#[derive(Debug, Args)]
pub struct DeleteCmd {
    /// The milestone's name (ignoring case) or id
    pub milestone: String,
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `delete` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Deleted<'a> {
    workspace: &'a str,
    id: &'a str,
    name: &'a str,
    deleted: bool,
}

pub fn delete(ctx: &Ctx, cmd: &DeleteCmd) -> Result<()> {
    let ws = ctx.write_session_with(cmd.force)?;
    let project = resolve::project(&ws, &cmd.project)?;
    let found = resolve::milestone(&project, &cmd.milestone)?;
    ws.guard(&target(&project), &led_by(&project), false)?;

    // Deleting a milestone silently unfiles its issues, and what stage they
    // belonged to is lost. Refuse until they have been moved.
    let view: MilestoneView = ws.client.execute(&read::milestone_view(found.id.inner()))?;
    let left = &view.detail.issues;
    if !left.is_empty() {
        let mut names: Vec<&str> = left
            .iter()
            .take(LEFTOVERS_SHOWN)
            .map(|i| i.identifier.as_str())
            .collect();
        if left.len() > LEFTOVERS_SHOWN {
            names.push("...");
        }
        return Err(CliError::general(format!(
            "milestone {:?} still has {} issue(s) ({}); move them with `linear issue update --milestone` first",
            found.name,
            left.len(),
            names.join(", ")
        )));
    }

    let data: MilestoneDelete = ws
        .client
        .execute(&inputs::milestone_delete(found.id.inner()))?;
    if !data.project_milestone_delete.success {
        return Err(CliError::general("Linear could not delete the milestone"));
    }
    let value = Deleted {
        workspace: &ws.workspace,
        id: found.id.inner(),
        name: &found.name,
        deleted: true,
    };
    ws.emit(
        &value,
        || format!("{}  (deleted)", found.name),
        || found.name.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- shared

/// What a milestone write is refused (or forced) on: the project it belongs to.
fn target(project: &ProjectOwnership) -> String {
    format!("project {:?} (a milestone of it)", project.name)
}

/// The ownership question a milestone write asks: does the viewer lead the project?
fn led_by(project: &ProjectOwnership) -> Write<'_> {
    Write::ProjectUpdate {
        lead: project.lead.as_ref().map(|u| u.id.inner()),
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

/// The description in a file, without trailing whitespace; `None` when there is none or it is blank.
fn description(path: Option<&std::path::Path>) -> Result<Option<String>> {
    let Some(path) = path else { return Ok(None) };
    let text = read_text(path)?;
    let text = text.trim_end();
    Ok((!text.trim().is_empty()).then(|| text.to_owned()))
}
