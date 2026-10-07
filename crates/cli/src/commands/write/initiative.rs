//! `linear initiative create|archive|unarchive|delete`.
//!
//! An initiative belongs to the workspace, not to a project, so no ownership
//! rule applies and neither does a validator. What is always checked: it needs
//! a name, and one with the same name is returned instead of creating another.
//! Archiving, restoring and deleting follow the same rule: no ownership check.

use super::{read_text, WriteSession};
use crate::commands::format::initiative_status;
use crate::commands::listing::paginate;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::Args;
use linear_core::inputs::{self, InitiativeCreate, InitiativeCreateInput};
use linear_core::matching::match_initiative;
use linear_core::read::{
    self, InitiativeList, InitiativeListVars, InitiativeListWithArchived, INITIATIVE_LIST_PAGE_SIZE,
};
use linear_core::types::Initiative;
use serde::Serialize;
use std::path::PathBuf;

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
