//! `linear initiative create`.
//!
//! An initiative belongs to the workspace, not to a project, so no ownership
//! rule applies and neither does a validator. What is always checked: it needs
//! a name, and one with the same name is returned instead of creating another.

use super::{read_text, WriteSession};
use crate::commands::format::initiative_status;
use crate::commands::listing::paginate;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::Args;
use linear_core::inputs::{self, InitiativeCreate, InitiativeCreateInput};
use linear_core::read::{self, InitiativeList, InitiativeListVars, INITIATIVE_LIST_PAGE_SIZE};
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
