//! `linear initiative create|update|archive|unarchive|delete|add-project|remove-project|status-update`.
//!
//! An initiative belongs to the workspace, not to a project, so no ownership
//! rule applies and neither does a validator. What is always checked: it needs
//! a name, and one with the same name is returned instead of creating another.
//! Updating, archiving, restoring, deleting and writing a status update follow
//! the same rule: no ownership check.
//!
//! Putting a project under an initiative, or taking it out, changes the
//! project's initiatives as well, so `add-project` and `remove-project` also
//! ask the ownership rules about the project, as `project update --initiative`
//! does.

use super::{read_text, resolve, ForceArg, WriteSession};
use crate::commands::format::initiative_status;
use crate::commands::listing::paginate;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use chrono::NaiveDate;
use clap::Args;
use linear_core::guard::Write;
use linear_core::initiative_write::{
    self as iw, InitiativeStatusUpdateCreateInput, InitiativeToProjectDelete, InitiativeUpdate,
    InitiativeUpdateCreate, InitiativeUpdateInput, ProjectLinksQuery,
};
use linear_core::inputs::{self, InitiativeCreate, InitiativeCreateInput};
use linear_core::matching::match_initiative;
use linear_core::project_write as pw;
use linear_core::read::{
    self, InitiativeList, InitiativeListVars, InitiativeListWithArchived, ProjectOwnership,
    INITIATIVE_LIST_PAGE_SIZE,
};
use linear_core::types::{
    Initiative, InitiativeStatus, InitiativeStatusUpdate, InitiativeUpdateHealthType,
};
use serde::Serialize;
use std::path::PathBuf;

/// The health values an initiative status update takes, as Linear spells them.
pub const HEALTHS: [&str; 3] = ["onTrack", "atRisk", "offTrack"];

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// The initiative's name; one with this exact name is returned instead of creating another
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// Read the description from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub description_file: Option<PathBuf>,
}

/// What `create` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    /// `true` when an initiative with the same name already existed and nothing was created.
    existing: bool,
    #[serde(flatten)]
    initiative: &'a Initiative,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let name = cmd.name.trim();
    if name.is_empty() {
        return Err(CliError::usage("--name must not be empty"));
    }
    let description = match cmd.description_file.as_deref() {
        Some(path) => {
            let text = read_text(path)?;
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
        None => None,
    };

    let ws = ctx.write_session()?;
    let all = paginate(INITIATIVE_LIST_PAGE_SIZE, None, |page| {
        let vars = InitiativeListVars::new(page, None);
        let data: InitiativeList = ws.client.execute(&read::initiative_list(vars))?;
        Ok(data.initiatives)
    })?
    .items;
    if let Some(existing) = all.iter().find(|i| i.name == name) {
        emit(ctx, &ws, existing, true);
        return Ok(());
    }

    let data: InitiativeCreate =
        ws.client
            .execute(&inputs::initiative_create(InitiativeCreateInput {
                name: name.to_owned(),
                description,
            }))?;
    let payload = data.initiative_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the initiative"));
    }
    emit(ctx, &ws, &payload.initiative, false);
    Ok(())
}

fn emit(ctx: &Ctx, ws: &WriteSession, initiative: &Initiative, existing: bool) {
    let value = Created {
        workspace: &ws.workspace,
        existing,
        initiative,
    };
    ctx.out.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!(
                "{}  {}  {}  ({how})",
                initiative.slug_id,
                initiative.name,
                initiative_status(&initiative.status)
            )
        },
        || initiative.slug_id.clone(),
    );
}

// ---------------------------------------------------------------- archive, unarchive, delete

#[derive(Debug, Args)]
pub struct InitiativeTargetCmd {
    /// Initiative id, slug id, URL or name
    pub initiative: String,
}

/// What `archive`, `unarchive` and `delete` print.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Changed<'a> {
    workspace: &'a str,
    id: &'a str,
    slug_id: &'a str,
    name: &'a str,
    url: &'a str,
    /// What was done: `archived`, `unarchived` or `deleted` (trashed).
    action: &'static str,
}

pub fn archive(ctx: &Ctx, cmd: &InitiativeTargetCmd) -> Result<()> {
    change(ctx, cmd, "archived", false, |ws, id| {
        let r: inputs::InitiativeArchive = ws.client.execute(&inputs::initiative_archive(id))?;
        Ok(r.initiative_archive.success)
    })
}

/// Restore an archived initiative.
pub fn unarchive(ctx: &Ctx, cmd: &InitiativeTargetCmd) -> Result<()> {
    change(ctx, cmd, "unarchived", true, |ws, id| {
        let r: inputs::InitiativeUnarchive =
            ws.client.execute(&inputs::initiative_unarchive(id))?;
        Ok(r.initiative_unarchive.success)
    })
}

/// Trash an initiative. Linear keeps it for a while.
pub fn delete(ctx: &Ctx, cmd: &InitiativeTargetCmd) -> Result<()> {
    change(ctx, cmd, "deleted", false, |ws, id| {
        let r: inputs::InitiativeDelete = ws.client.execute(&inputs::initiative_delete(id))?;
        Ok(r.initiative_delete.success)
    })
}

/// The shared path: find the initiative (an archived one too, when
/// `include_archived`), send `mutate`, print one line. No ownership rule
/// applies (an initiative belongs to the workspace), like `create`.
fn change(
    ctx: &Ctx,
    cmd: &InitiativeTargetCmd,
    action: &'static str,
    include_archived: bool,
    mutate: impl FnOnce(&WriteSession, &str) -> Result<bool>,
) -> Result<()> {
    let ws = ctx.write_session()?;
    let all = if include_archived {
        paginate(INITIATIVE_LIST_PAGE_SIZE, None, |vars| {
            let data: InitiativeListWithArchived = ws
                .client
                .execute(&read::initiative_list_with_archived(vars))?;
            Ok(data.initiatives)
        })?
        .items
    } else {
        paginate(INITIATIVE_LIST_PAGE_SIZE, None, |page| {
            let vars = InitiativeListVars::new(page, None);
            let data: InitiativeList = ws.client.execute(&read::initiative_list(vars))?;
            Ok(data.initiatives)
        })?
        .items
    };
    let found = match_initiative(&all, &cmd.initiative)?;
    if !mutate(&ws, found.id.inner())? {
        return Err(CliError::general(format!(
            "Linear could not change the initiative {} ({action})",
            found.name
        )));
    }
    let value = Changed {
        workspace: &ws.workspace,
        id: found.id.inner(),
        slug_id: &found.slug_id,
        name: &found.name,
        url: &found.url,
        action,
    };
    ctx.out.emit(
        &value,
        || format!("{}  {}  ({action})", found.slug_id, found.name),
        || found.slug_id.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// Initiative id, slug id, URL or name
    pub initiative: String,
    /// New name
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Replace the description with the contents of a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub description_file: Option<PathBuf>,
    /// Status: active, planned, proposed, completed, canceled
    #[arg(long, value_name = "STATUS", value_parser = ["active", "completed", "canceled", "planned", "proposed"])]
    pub status: Option<String>,
    /// Target date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub target_date: Option<NaiveDate>,
    /// New owner: `me`, an email or a name
    #[arg(long, value_name = "WHO")]
    pub owner: Option<String>,
}

/// What `update` prints: the initiative as it is now, and which fields were written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    initiative: &'a Initiative,
    /// The fields this run changed (empty: everything already was as asked).
    changed: Vec<&'static str>,
}

fn status_of(name: &str) -> InitiativeStatus {
    match name {
        "active" => InitiativeStatus::Active,
        "completed" => InitiativeStatus::Completed,
        "canceled" => InitiativeStatus::Canceled,
        "planned" => InitiativeStatus::Planned,
        _ => InitiativeStatus::Proposed,
    }
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.name.is_none()
        && cmd.description_file.is_none()
        && cmd.status.is_none()
        && cmd.target_date.is_none()
        && cmd.owner.is_none()
    {
        return Err(CliError::usage(
            "nothing to change: pass --name, --description-file, --status, --target-date or --owner",
        ));
    }
    let name = match cmd.name.as_deref().map(str::trim) {
        Some("") => return Err(CliError::usage("--name must not be empty")),
        other => other,
    };
    let description = match cmd.description_file.as_deref() {
        Some(path) => {
            let text = read_text(path)?;
            let text = text.trim();
            if text.is_empty() {
                return Err(CliError::usage("the description file is empty"));
            }
            Some(text.to_owned())
        }
        None => None,
    };

    let ws = ctx.write_session()?;
    let all = list_initiatives(&ws)?;
    let found = match_initiative(&all, &cmd.initiative)?;
    // An unknown owner stops here, before anything is written. No ownership rule
    // applies: an initiative belongs to the workspace, like `create`.
    let owner = cmd
        .owner
        .as_deref()
        .map(|who| resolve::user_id(&ws, who))
        .transpose()?;

    let mut input = InitiativeUpdateInput::default();
    let mut changed: Vec<&'static str> = Vec::new();
    if let Some(n) = name.filter(|n| *n != found.name) {
        input.name = Some(n.to_owned());
        changed.push("name");
    }
    if let Some(d) =
        description.filter(|d| found.description.as_deref().map(str::trim) != Some(d.as_str()))
    {
        input.description = Some(d);
        changed.push("description");
    }
    if let Some(s) = cmd
        .status
        .as_deref()
        .map(status_of)
        .filter(|s| *s != found.status)
    {
        input.status = Some(s);
        changed.push("status");
    }
    if let Some(d) = cmd.target_date.filter(|d| Some(*d) != found.target_date) {
        input.target_date = Some(d);
        changed.push("targetDate");
    }
    if let Some(o) =
        owner.filter(|o| Some(o.as_str()) != found.owner.as_ref().map(|u| u.id.inner()))
    {
        input.owner_id = Some(o);
        changed.push("owner");
    }

    let now = if input.is_empty() {
        found.clone()
    } else {
        let data: InitiativeUpdate = ws
            .client
            .execute(&iw::initiative_update(found.id.inner(), input))?;
        if !data.initiative_update.success {
            return Err(CliError::general(format!(
                "Linear could not update the initiative {}",
                found.name
            )));
        }
        data.initiative_update.initiative
    };
    let value = Updated {
        workspace: &ws.workspace,
        initiative: &now,
        changed: changed.clone(),
    };
    ctx.out.emit(
        &value,
        || {
            let verb = if changed.is_empty() {
                "already as asked, nothing changed".to_owned()
            } else {
                format!("updated {}", changed.join(", "))
            };
            format!(
                "{}  {}  {}  ({verb})",
                now.slug_id,
                now.name,
                initiative_status(&now.status)
            )
        },
        || now.slug_id.clone(),
    );
    Ok(())
}

fn list_initiatives(ws: &WriteSession) -> Result<Vec<Initiative>> {
    Ok(paginate(INITIATIVE_LIST_PAGE_SIZE, None, |page| {
        let vars = InitiativeListVars::new(page, None);
        let data: InitiativeList = ws.client.execute(&read::initiative_list(vars))?;
        Ok(data.initiatives)
    })?
    .items)
}

// ---------------------------------------------------------------- add-project, remove-project

#[derive(Debug, Args)]
pub struct ProjectLinkCmd {
    /// Initiative id, slug id, URL or name
    pub initiative: String,
    /// Project id, slug id, URL or name
    pub project: String,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `add-project` and `remove-project` print.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Linked<'a> {
    workspace: &'a str,
    initiative: LinkedInitiative<'a>,
    project: LinkedProject<'a>,
    /// What was done: `added` or `removed`.
    action: &'static str,
    /// `true` when the project already was (or already was not) under the initiative, so
    /// nothing was sent.
    unchanged: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkedInitiative<'a> {
    id: &'a str,
    slug_id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkedProject<'a> {
    id: &'a str,
    slug_id: &'a str,
    name: &'a str,
}

pub fn add_project(ctx: &Ctx, cmd: &ProjectLinkCmd) -> Result<()> {
    link_change(ctx, cmd, true)
}

pub fn remove_project(ctx: &Ctx, cmd: &ProjectLinkCmd) -> Result<()> {
    link_change(ctx, cmd, false)
}

fn link_change(ctx: &Ctx, cmd: &ProjectLinkCmd, add: bool) -> Result<()> {
    let ws = ctx.write_session_with(cmd.force)?;
    let all = list_initiatives(&ws)?;
    let initiative = match_initiative(&all, &cmd.initiative)?;
    let project: ProjectOwnership = resolve::project(&ws, &cmd.project)?;
    // The project's initiatives change, so the project's ownership applies.
    ws.guard(
        &format!("project {:?}", project.name),
        &Write::ProjectUpdate {
            lead: project.lead.as_ref().map(|u| u.id.inner()),
        },
        false,
    )?;

    let links: ProjectLinksQuery = ws.client.execute(&iw::project_links(project.id.inner()))?;
    let link = links
        .project
        .initiative_to_projects
        .iter()
        .find(|l| l.initiative.id == initiative.id);

    let (action, unchanged) = if add {
        match link {
            Some(_) => ("added", true),
            None => {
                let r: pw::InitiativeToProjectCreate = ws.client.execute(
                    &pw::initiative_to_project_create(initiative.id.inner(), project.id.inner()),
                )?;
                if !r.initiative_to_project_create.success {
                    return Err(CliError::general(format!(
                        "Linear could not put {} under {}",
                        project.name, initiative.name
                    )));
                }
                ("added", false)
            }
        }
    } else {
        match link {
            None => ("removed", true),
            Some(link) => {
                let r: InitiativeToProjectDelete = ws
                    .client
                    .execute(&iw::initiative_to_project_delete(link.id.inner()))?;
                if !r.initiative_to_project_delete.success {
                    return Err(CliError::general(format!(
                        "Linear could not take {} out of {}",
                        project.name, initiative.name
                    )));
                }
                ("removed", false)
            }
        }
    };

    let value = Linked {
        workspace: &ws.workspace,
        initiative: LinkedInitiative {
            id: initiative.id.inner(),
            slug_id: &initiative.slug_id,
            name: &initiative.name,
        },
        project: LinkedProject {
            id: project.id.inner(),
            slug_id: &project.slug_id,
            name: &project.name,
        },
        action,
        unchanged,
    };
    ws.emit(
        &value,
        || {
            let how = match (add, unchanged) {
                (true, false) => "added".to_owned(),
                (true, true) => "already under it, nothing sent".to_owned(),
                (false, false) => "removed".to_owned(),
                (false, true) => "not under it, nothing sent".to_owned(),
            };
            format!(
                "{}  {}  <-  {}  ({how})",
                initiative.slug_id, initiative.name, project.name
            )
        },
        || initiative.slug_id.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- status update

#[derive(Debug, Args)]
pub struct StatusUpdateCmd {
    /// Initiative id, slug id, URL or name
    pub initiative: String,
    /// How the initiative is doing
    #[arg(long, value_name = "HEALTH", value_parser = HEALTHS)]
    pub health: String,
    /// Read the update from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: PathBuf,
}

/// What `status-update` prints.
#[derive(Serialize)]
struct StatusUpdated<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    update: &'a InitiativeStatusUpdate,
}

/// The health value as Linear spells it.
pub fn health_word(h: &InitiativeUpdateHealthType) -> String {
    match h {
        InitiativeUpdateHealthType::OnTrack => "onTrack".into(),
        InitiativeUpdateHealthType::AtRisk => "atRisk".into(),
        InitiativeUpdateHealthType::OffTrack => "offTrack".into(),
        InitiativeUpdateHealthType::Other(s) => s.clone(),
    }
}

pub fn status_update(ctx: &Ctx, cmd: &StatusUpdateCmd) -> Result<()> {
    let body = read_text(&cmd.body_file)?;
    if body.trim().is_empty() {
        return Err(CliError::usage(
            "the status update is empty: say where the initiative stands, what comes next and what it waits for",
        ));
    }
    let health = match cmd.health.as_str() {
        "onTrack" => InitiativeUpdateHealthType::OnTrack,
        "atRisk" => InitiativeUpdateHealthType::AtRisk,
        _ => InitiativeUpdateHealthType::OffTrack,
    };

    let ws = ctx.write_session()?;
    let all = list_initiatives(&ws)?;
    // No ownership rule applies: an initiative belongs to the workspace, like `create`.
    let initiative = match_initiative(&all, &cmd.initiative)?;
    let input = InitiativeStatusUpdateCreateInput {
        initiative_id: initiative.id.inner().to_owned(),
        health,
        body: body.trim().to_owned(),
    };
    let data: InitiativeUpdateCreate = ws.client.execute(&iw::initiative_update_create(input))?;
    if !data.initiative_update_create.success {
        return Err(CliError::general(
            "Linear could not write the status update",
        ));
    }
    let update = &data.initiative_update_create.initiative_update;
    let value = StatusUpdated {
        workspace: &ws.workspace,
        update,
    };
    ctx.out.emit(
        &value,
        || {
            format!(
                "{}  {}  {}",
                update.initiative.name,
                health_word(&update.health),
                update.url
            )
        },
        || update.url.clone(),
    );
    Ok(())
}
