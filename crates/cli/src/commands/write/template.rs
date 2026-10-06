//! `linear template create`.
//!
//! Creates an issue template from a markdown body. The sections of a template
//! are its headings, and Linear is where they live (the `template-sections`
//! rule reads them from there), so a body without a heading is refused: it
//! would make a template that nothing can be checked against. A template of
//! the same name is returned instead of creating another.
//!
//! No ownership rule applies (templates are not owned by a project) and no
//! validator does either.

use super::{read_text, resolve, WriteSession};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::Args;
use linear_core::inputs::{self, TemplateCreate, TemplateCreateInput};
use linear_core::queries::{self, Templates};
use linear_core::template::{headings_of, is_issue_template, markdown_to_doc};
use linear_core::types::{Team, Template};
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// The template's name; an issue template with this exact name is returned instead
    /// of creating another
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// Read the body (markdown with `## headings` as sections) from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: PathBuf,
    /// What the template is for
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
}

/// What `create` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    id: &'a str,
    name: &'a str,
    #[serde(rename = "type")]
    type_: &'a str,
    team: Option<&'a Team>,
    /// `true` when an issue template with the same name already existed and nothing was created.
    existing: bool,
    /// The headings of the template body: the sections an issue written from it must fill.
    sections: Vec<String>,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let name = cmd.name.trim();
    if name.is_empty() {
        return Err(CliError::usage("--name must not be empty"));
    }
    let body = read_text(&cmd.body_file)?;
    let doc = markdown_to_doc(&body);
    let sections = headings_of(&doc);
    if sections.is_empty() {
        return Err(CliError::usage(
            "the body has no sections; add `## heading` lines (they are what an issue is checked against)",
        ));
    }

    let ws = ctx.write_session()?;
    let team = resolve::team(&ws, cmd.team.as_deref())?;
    let all: Templates = ws.client.execute(&queries::templates())?;
    if let Some(existing) = all
        .templates
        .iter()
        .find(|t| is_issue_template(t) && t.name == name)
    {
        let existing_sections = linear_core::template::description_doc(existing)
            .map(|d| headings_of(&d))
            .unwrap_or_default();
        emit(ctx, &ws, existing, true, existing_sections);
        return Ok(());
    }

    let data: TemplateCreate =
        ws.client
            .execute(&inputs::template_create(TemplateCreateInput {
                kind: "issue".to_owned(),
                name: name.to_owned(),
                team_id: Some(team.id.inner().to_owned()),
                description: cmd
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned),
                template_data: json!({ "title": "", "descriptionData": doc }),
            }))?;
    let payload = data.template_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the template"));
    }
    emit(ctx, &ws, &payload.template, false, sections);
    Ok(())
}

fn emit(ctx: &Ctx, ws: &WriteSession, t: &Template, existing: bool, sections: Vec<String>) {
    let value = Created {
        workspace: &ws.workspace,
        id: t.id.inner(),
        name: &t.name,
        type_: &t.type_,
        team: t.team.as_ref(),
        existing,
        sections,
    };
    ctx.out.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!("{}  ({how})", t.name)
        },
        || t.name.clone(),
    );
}
