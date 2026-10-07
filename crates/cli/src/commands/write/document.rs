//! `linear document create|update`.
//!
//! A document belongs to a project or an initiative, so these writes follow
//! the owner of that parent: only the lead of a project, or the owner of an
//! initiative, may add or change its documents (exit code 4 otherwise). A
//! document of an issue, a team or a cycle is read but not written here.
//!
//! The `template-sections` validator applies to the body like it does to an
//! issue's or a project's (`document_create`, `document_update`): with the rule
//! on, `--template` names a Linear *document* template and the body has to fill
//! every one of its sections (exit 5). Without the rule the flag is ignored,
//! with a note. The checks that are always on come first: a document needs a
//! title, a title is not given twice under one parent (a second `create` returns
//! the first), and a body file is not empty.

use super::dry_run::{Plan, Target};
use super::{read_text, resolve, ForceArg, WriteSession};
use crate::commands::document::{find_initiative, key_of};
use crate::commands::format::date_time;
use crate::commands::listing::paginate;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::{ArgGroup, Args};
use linear_core::config::Rule;
use linear_core::docs::{
    self, Doc, DocCreate, DocCreateInput, DocList, DocListVars, DocQuery, DocUpdate,
    DocUpdateInput, DocView, DOC_LIST_PAGE_SIZE,
};
use linear_core::guard::Write;
use linear_core::rules::{Draft, Operation};
use serde::Serialize;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- create

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("parent").required(true).args(["project", "initiative"])))]
pub struct CreateCmd {
    /// The document's title; a project or initiative that already has one with this title
    /// returns it instead of getting another
    #[arg(long, value_name = "TITLE")]
    pub title: String,
    /// Add it to this project (id, slug id, URL or name); you must lead it
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,
    /// Add it to this initiative (id, slug id, URL or name); you must own it
    #[arg(long, value_name = "INITIATIVE")]
    pub initiative: Option<String>,
    /// Read the body (markdown) from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear document template the body has to follow (needed when the
    /// `template-sections` rule is on; otherwise ignored)
    #[arg(long, value_name = "NAME")]
    pub template: Option<String>,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `create` prints: the document, and whether it was already there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    /// `true` when the parent already had a document with this title and nothing was created.
    existing: bool,
    #[serde(flatten)]
    document: &'a Doc,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let title = non_blank(&cmd.title, "--title")?;
    let body = body(cmd.body_file.as_deref())?;

    let ws = ctx.write_session_with(cmd.force)?;
    let (project_id, initiative_id) =
        guarded_parent(&ws, cmd.project.as_deref(), cmd.initiative.as_deref())?;

    // A document with this title under the same parent is the one to return.
    let query = DocQuery {
        project_id: project_id.clone(),
        initiative_id: initiative_id.clone(),
        title: Some(title.clone()),
    };
    let filter = query.filter();
    let same = paginate(DOC_LIST_PAGE_SIZE, None, |page| {
        let vars = DocListVars::new(page, filter.clone());
        let data: DocList = ws.client.execute(&docs::doc_list(vars))?;
        Ok(data.documents)
    })?
    .items;
    if let Some(existing) = same.iter().find(|d| d.title == title) {
        if ws.dry_run {
            return ws.finish_dry_run(
                Plan::new(
                    "document create",
                    Target::existing("document", &existing.title, existing.id.inner()),
                )
                .reason("the parent already has a document with this title, so none is created"),
            );
        }
        emit_created(&ws, existing, true);
        return Ok(());
    }

    validate(
        &ws,
        Operation::DocumentCreate,
        cmd.template.as_deref(),
        Some(body.as_deref().unwrap_or("")),
    )?;

    let new_title = title.clone();
    let op = docs::doc_create(DocCreateInput {
        title,
        content: body,
        project_id,
        initiative_id,
    });
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new(
            "document create",
            Target::new("document", new_title),
        ));
    }
    let data: DocCreate = ws.client.execute(&op)?;
    let payload = data.document_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the document"));
    }
    emit_created(&ws, &payload.document, false);
    Ok(())
}

fn emit_created(ws: &WriteSession, document: &Doc, existing: bool) {
    let value = Created {
        workspace: &ws.workspace,
        existing,
        document,
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
                "{}  {}  {}  ({how})",
                document.slug_id,
                document.title,
                document.parent_label()
            )
        },
        || document.slug_id.clone(),
    );
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// The document: id, slug id, or the URL (or its `title-slugid` part)
    pub document: String,
    /// Rename it
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    /// Replace the body with the contents of a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear document template the new body has to follow (needed with --body-file
    /// when the `template-sections` rule is on; otherwise ignored)
    #[arg(long, value_name = "NAME")]
    pub template: Option<String>,
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
    document: &'a Doc,
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.title.is_none() && cmd.body_file.is_none() {
        return Err(CliError::usage(
            "nothing to change: pass --title or --body-file",
        ));
    }
    let new_title = cmd
        .title
        .as_deref()
        .map(|t| non_blank(t, "--title"))
        .transpose()?;
    let new_body = match cmd.body_file.as_deref() {
        Some(path) => Some(body(Some(path))?.ok_or_else(|| {
            CliError::usage("--body-file is empty; a document's body is not cleared this way")
        })?),
        None => None,
    };

    let ws = ctx.write_session_with(cmd.force)?;
    let data: DocView = ws.client.execute(&docs::doc_view(key_of(&cmd.document)))?;
    let current = &data.document;

    // Whose document is it? Only a project's or an initiative's can be written.
    match (&current.project, &current.initiative) {
        (Some(project), _) => {
            let owned = resolve::project(&ws, project.id.inner())?;
            ws.guard(
                &format!("project {:?} (document {:?})", owned.name, current.title),
                &Write::ProjectUpdate {
                    lead: owned.lead.as_ref().map(|u| u.id.inner()),
                },
                false,
            )?;
        }
        (None, Some(initiative)) => {
            let owned = find_initiative(&ws.client, initiative.id.inner())?;
            ws.guard(
                &format!(
                    "initiative {:?} (document {:?})",
                    owned.name, current.title
                ),
                &Write::InitiativeUpdate {
                    owner: owned.owner.as_ref().map(|u| u.id.inner()),
                },
                false,
            )?;
        }
        (None, None) => {
            return Err(CliError::usage(format!(
                "{:?} belongs to neither a project nor an initiative ({}); only those documents can be written",
                current.title,
                current.parent_label()
            )))
        }
    }

    validate(
        &ws,
        Operation::DocumentUpdate,
        cmd.template.as_deref(),
        new_body.as_deref(),
    )?;

    // Only what differs from now is sent.
    let current_body = data.detail.content.as_deref().map(str::trim_end);
    let input = DocUpdateInput {
        title: new_title.filter(|t| *t != current.title),
        content: new_body.filter(|b| current_body != Some(b.as_str())),
    };
    if ws.dry_run {
        let mut changed = Vec::new();
        if input.title.is_some() {
            changed.push("title");
        }
        if input.content.is_some() {
            changed.push("body");
        }
        if !input.is_empty() {
            ws.record(&docs::doc_update(current.id.inner(), input));
        }
        return ws.finish_dry_run(
            Plan::new(
                "document update",
                Target::existing("document", &current.title, current.id.inner()),
            )
            .changed(changed)
            .reason("the document already has these values"),
        );
    }
    if input.is_empty() {
        ws.note("nothing to change: the document already has these values");
        emit_updated(&ws, current, false);
        return Ok(());
    }

    let data: DocUpdate = ws
        .client
        .execute(&docs::doc_update(current.id.inner(), input))?;
    let payload = data.document_update;
    if !payload.success {
        return Err(CliError::general("Linear could not update the document"));
    }
    emit_updated(&ws, &payload.document, true);
    Ok(())
}

fn emit_updated(ws: &WriteSession, document: &Doc, changed: bool) {
    let value = Updated {
        workspace: &ws.workspace,
        changed,
        document,
    };
    ws.emit(
        &value,
        || {
            format!(
                "{}  {}  updated {}  ({})",
                document.slug_id,
                document.title,
                date_time(&document.updated_at),
                if changed { "updated" } else { "unchanged" }
            )
        },
        || document.slug_id.clone(),
    );
}

// ---------------------------------------------------------------- shared

/// Resolve the parent named by `--project` or `--initiative` and ask the
/// ownership rule about it. Returns the ids to send: exactly one is `Some`.
fn guarded_parent(
    ws: &WriteSession,
    project: Option<&str>,
    initiative: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    if let Some(reference) = project {
        let project = resolve::project(ws, reference)?;
        ws.guard(
            &format!("project {:?} (a document under it)", project.name),
            &Write::ProjectUpdate {
                lead: project.lead.as_ref().map(|u| u.id.inner()),
            },
            false,
        )?;
        return Ok((Some(project.id.inner().to_owned()), None));
    }
    let reference = initiative.ok_or_else(|| CliError::usage("pass --project or --initiative"))?;
    let initiative = find_initiative(&ws.client, reference)?;
    ws.guard(
        &format!("initiative {:?} (a document under it)", initiative.name),
        &Write::InitiativeUpdate {
            owner: initiative.owner.as_ref().map(|u| u.id.inner()),
        },
        false,
    )?;
    Ok((None, Some(initiative.id.inner().to_owned())))
}

/// Run the validators on a body about to be written.
fn validate(
    ws: &WriteSession,
    operation: Operation,
    template: Option<&str>,
    body: Option<&str>,
) -> Result<()> {
    if template.is_some() && !ws.rules.applies(Rule::TemplateSections, operation) {
        ws.note(
            "--template is ignored: the template-sections rule is not enabled for this workspace",
        );
    }
    let mut draft = Draft::new(operation);
    if let Some(t) = template {
        draft = draft.template(t);
    }
    if let Some(b) = body {
        draft = draft.body(b);
    }
    ws.validate(&draft)?;
    Ok(())
}

fn non_blank(value: &str, flag: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CliError::usage(format!("{flag} must not be empty")))
    } else {
        Ok(trimmed.to_owned())
    }
}

/// The body in a file, without trailing whitespace; `None` when there is no file or it is blank.
fn body(path: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = path else { return Ok(None) };
    let text = read_text(path)?;
    let text = text.trim_end();
    Ok((!text.trim().is_empty()).then(|| text.to_owned()))
}
