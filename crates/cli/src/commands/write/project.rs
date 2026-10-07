//! `linear project create|update|reorder|status-update`.

use super::{read_text, resolve, retry, Rollback, WriteSession, ATTACH_WAITS};
use crate::commands::format::{health, person};
use crate::commands::listing::{paginate, resolve_project};
use crate::commands::project::{out as project_out, ProjectOut};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use chrono::NaiveDate;
use clap::Args;
use linear_core::config::Rule;
use linear_core::guard::Write;
use linear_core::inputs::Patch;
use linear_core::matching::match_initiative;
use linear_core::project_write::{
    self as pw, ProjectCreate, ProjectCreateInput, ProjectDelete, ProjectOrderQuery,
    ProjectStatuses, ProjectUpdate, ProjectUpdateCreate, ProjectUpdateInput,
    StatusUpdateCreateInput, UnfinishedNamed,
};
use linear_core::read::{
    self, InitiativeList, InitiativeListVars, ProjectView, INITIATIVE_LIST_PAGE_SIZE,
};
use linear_core::reorder::{self, OrderRow};
use linear_core::rules::{Draft, Operation};
use linear_core::types::{Initiative, ProjectUpdateHealthType, StatusUpdate};
use serde::Serialize;
use std::path::PathBuf;

/// The health values a status update takes, as Linear spells them.
const HEALTHS: [&str; 3] = ["onTrack", "atRisk", "offTrack"];

// ---------------------------------------------------------------- create

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// The project's name. An unfinished project with this name is returned
    /// instead of creating another
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// The short summary shown under the name
    #[arg(long, value_name = "TEXT")]
    pub summary: Option<String>,
    /// Read the body from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear project template the body must follow (checked by the
    /// `template-sections` rule)
    #[arg(long, value_name = "NAME")]
    pub template: Option<String>,
    /// Target date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub target_date: Option<NaiveDate>,
    /// Put the project under this initiative (id, slug id, URL or name)
    #[arg(long, value_name = "INITIATIVE")]
    pub initiative: Option<String>,
    /// Lead: `me`, an email or a name (default: you). You may only create a
    /// project you lead
    #[arg(long, value_name = "WHO")]
    pub lead: Option<String>,
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
    slug_id: &'a str,
    name: &'a str,
    url: &'a str,
    /// `true` when an unfinished project with this name already existed and nothing was created.
    existing: bool,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    // Everything that can be judged from the arguments alone comes first.
    let name = cmd.name.trim();
    if name.is_empty() {
        return Err(CliError::usage("the project name is empty"));
    }
    let summary = non_blank(cmd.summary.as_deref(), "the summary")?;
    let body = read_body(cmd.body_file.as_deref())?;

    let ws = ctx.write_session()?;

    // Resolve names to ids (read-only; unknown names stop here).
    let team = resolve::team(&ws, cmd.team.as_deref())?;
    let lead = match cmd.lead.as_deref() {
        Some(who) => resolve::user_id(&ws, who)?,
        None => ws.viewer.id.clone(),
    };
    let initiative = cmd
        .initiative
        .as_deref()
        .map(|r| initiative(&ws, r))
        .transpose()?;

    // Guard, then the idempotence check, then validators.
    ws.guard(&Write::ProjectCreate { lead: Some(&lead) }, false)?;
    let existing: pw::UnfinishedNamed = ws.client.execute(&pw::unfinished_named(name))?;
    if let Some(found) = existing.projects.nodes.first() {
        emit_created(
            ctx,
            &ws,
            found.id.inner(),
            &found.slug_id,
            &found.name,
            &found.url,
            true,
        );
        return Ok(());
    }
    if cmd.template.is_some()
        && !ws
            .rules
            .applies(Rule::TemplateSections, Operation::ProjectCreate)
    {
        ws.note(
            "--template is ignored: the template-sections rule is not enabled for this workspace",
        );
    }
    let mut draft = Draft::new(Operation::ProjectCreate);
    if let Some(t) = &cmd.template {
        draft = draft.template(t);
    }
    if let Some(b) = &body {
        draft = draft.body(b);
    }
    ws.validate(&draft)?;

    // Mutate: create, then put it under the initiative; a failed link takes the project with it.
    let input = ProjectCreateInput {
        name: name.to_owned(),
        team_ids: vec![team.id.inner().to_owned()],
        description: summary.map(str::to_owned),
        content: body.as_ref().map(|b| b.trim_end().to_owned()),
        lead_id: Some(lead),
        target_date: cmd.target_date,
    };
    let data: ProjectCreate = ws.client.execute(&pw::project_create(input))?;
    let project = match data.project_create {
        p if p.success => p
            .project
            .ok_or_else(|| CliError::general("Linear created the project but returned none"))?,
        _ => return Err(CliError::general("Linear could not create the project")),
    };

    if let Some(initiative) = &initiative {
        let mut rollback = Rollback::new();
        rollback.on_failure(format!("deleted project {}", project.name), || {
            let r: ProjectDelete = ws.client.execute(&pw::project_delete(project.id.inner()))?;
            if r.project_delete.success {
                Ok(())
            } else {
                Err(CliError::general("Linear refused to delete it"))
            }
        });
        if let Err(cause) = link_initiative(&ws, initiative, project.id.inner()) {
            return Err(rollback.fail(cause));
        }
    }

    emit_created(
        ctx,
        &ws,
        project.id.inner(),
        &project.slug_id,
        &project.name,
        &project.url,
        false,
    );
    Ok(())
}

fn emit_created(
    ctx: &Ctx,
    ws: &WriteSession,
    id: &str,
    slug_id: &str,
    name: &str,
    url: &str,
    existing: bool,
) {
    let value = Created {
        workspace: &ws.workspace,
        id,
        slug_id,
        name,
        url,
        existing,
    };
    ctx.out.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!("{name}  ({slug_id})\n{url}  ({how})")
        },
        || slug_id.to_owned(),
    );
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// Project id, slug id, URL or name
    pub project: String,
    /// New name. Fails when another unfinished project already has it
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// New short summary
    #[arg(long, value_name = "TEXT")]
    pub summary: Option<String>,
    /// Replace the body with the contents of a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear project template the new body must follow (checked by the
    /// `template-sections` rule, which requires it to replace a body)
    #[arg(long, value_name = "NAME", requires = "body_file")]
    pub template: Option<String>,
    /// Project status, by name (ignoring case), such as `In Progress` or `Completed`
    #[arg(long, value_name = "NAME")]
    pub status: Option<String>,
    /// Target date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub target_date: Option<NaiveDate>,
    /// Put the project under this initiative too (id, slug id, URL or name).
    /// Only adds: it never removes the project from another initiative
    #[arg(long, value_name = "INITIATIVE")]
    pub initiative: Option<String>,
    /// New lead: `me`, an email or a name
    #[arg(long, value_name = "WHO")]
    pub lead: Option<String>,
}

/// What `update` prints: the project as it is now, and which fields were written.
#[derive(Serialize)]
struct Updated<'a> {
    #[serde(flatten)]
    project: ProjectOut<'a>,
    /// The fields this run changed (empty: everything already was as asked).
    changed: Vec<&'static str>,
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.name.is_none()
        && cmd.summary.is_none()
        && cmd.body_file.is_none()
        && cmd.status.is_none()
        && cmd.target_date.is_none()
        && cmd.initiative.is_none()
        && cmd.lead.is_none()
    {
        return Err(CliError::usage(
            "nothing to change: pass --name, --summary, --body-file, --status, --target-date, \
             --initiative or --lead",
        ));
    }
    let name = non_blank(cmd.name.as_deref(), "the name")?;
    let summary = non_blank(cmd.summary.as_deref(), "the summary")?;
    let body = read_body(cmd.body_file.as_deref())?;

    let ws = ctx.write_session()?;
    let found = resolve_project(&ws.client, &cmd.project)?;
    let view: ProjectView = ws.client.execute(&read::project_view(found.id.inner()))?;
    let (project, detail) = (&view.project, &view.detail);

    // Resolve names to ids. An unknown name stops here, before anything is written.
    let status = match cmd.status.as_deref() {
        Some(reference) => {
            let all: ProjectStatuses = ws.client.execute(&pw::project_statuses())?;
            Some(pw::match_project_status(&all.project_statuses, reference)?.clone())
        }
        None => None,
    };
    let lead = cmd
        .lead
        .as_deref()
        .map(|who| resolve::user_id(&ws, who))
        .transpose()?;
    let initiative = cmd
        .initiative
        .as_deref()
        .map(|r| initiative(&ws, r))
        .transpose()?;
    if let Some(n) = name.filter(|n| *n != project.name) {
        let same: UnfinishedNamed = ws.client.execute(&pw::unfinished_named(n))?;
        if let Some(clash) = same.projects.nodes.iter().find(|p| p.id != project.id) {
            return Err(CliError::usage(format!(
                "another unfinished project is already named {n:?}: {}",
                clash.url
            )));
        }
    }

    // Guard, then validators.
    ws.guard(&Write::update_project(project), false)?;
    if cmd.template.is_some()
        && !ws
            .rules
            .applies(Rule::TemplateSections, Operation::ProjectUpdate)
    {
        ws.note(
            "--template is ignored: the template-sections rule is not enabled for this workspace",
        );
    }
    let mut draft = Draft::new(Operation::ProjectUpdate);
    if let Some(t) = &cmd.template {
        draft = draft.template(t);
    }
    if let Some(b) = &body {
        draft = draft.body(b);
    }
    ws.validate(&draft)?;

    // What to write, and what puts it back: only the fields that actually differ.
    let mut input = ProjectUpdateInput::default();
    let mut restore = ProjectUpdateInput::default();
    let mut changed: Vec<&'static str> = Vec::new();
    if let Some(n) = name.filter(|n| *n != project.name) {
        input.name = Some(n.to_owned());
        restore.name = Some(project.name.clone());
        changed.push("name");
    }
    if let Some(s) = summary.filter(|s| *s != detail.description) {
        input.description = Some(s.to_owned());
        restore.description = Some(detail.description.clone());
        changed.push("summary");
    }
    if let Some(b) = &body {
        let b = b.trim_end();
        if detail.content.as_deref().map(str::trim_end) != Some(b) {
            input.content = Patch::Set(b.to_owned());
            restore.content = match &detail.content {
                Some(old) => Patch::Set(old.clone()),
                None => Patch::Clear,
            };
            changed.push("body");
        }
    }
    if let Some(s) = status.filter(|s| s.id != project.status.id) {
        input.status_id = Some(s.id.inner().to_owned());
        restore.status_id = Some(project.status.id.inner().to_owned());
        changed.push("status");
    }
    if let Some(d) = cmd.target_date.filter(|d| Some(*d) != project.target_date) {
        input.target_date = Patch::Set(d);
        restore.target_date = match project.target_date {
            Some(old) => Patch::Set(old),
            None => Patch::Clear,
        };
        changed.push("targetDate");
    }
    if let Some(l) =
        lead.filter(|l| Some(l.as_str()) != project.lead.as_ref().map(|u| u.id.inner()))
    {
        input.lead_id = Patch::Set(l);
        restore.lead_id = match &project.lead {
            Some(old) => Patch::Set(old.id.inner().to_owned()),
            None => Patch::Clear,
        };
        changed.push("lead");
    }
    let link = initiative
        .as_ref()
        .filter(|i| !project.initiatives.iter().any(|linked| linked.id == i.id));
    if link.is_some() {
        changed.push("initiative");
    }

    // Mutate. A failed link puts the other fields back.
    let id = project.id.inner().to_owned();
    let id = id.as_str();
    let mut rollback = Rollback::new();
    if !input.is_empty() {
        let data: ProjectUpdate = ws.client.execute(&pw::project_update(id, input))?;
        if !data.project_update.success {
            return Err(CliError::general(format!(
                "Linear could not update {}",
                project.name
            )));
        }
        let client = &ws.client;
        rollback.on_failure("restored the project's earlier values", move || {
            let r: ProjectUpdate = client.execute(&pw::project_update(id, restore))?;
            if r.project_update.success {
                Ok(())
            } else {
                Err(CliError::general("Linear refused to restore them"))
            }
        });
    }
    if let Some(initiative) = link {
        if let Err(cause) = link_initiative(&ws, initiative, id) {
            return Err(rollback.fail(cause));
        }
    }

    // Show the project as it is now.
    let now: ProjectView = if changed.is_empty() {
        view
    } else {
        ws.client.execute(&read::project_view(id))?
    };
    let value = Updated {
        project: project_out(&ws.workspace, &now.project),
        changed: changed.clone(),
    };
    let p = &now.project;
    ctx.out.emit(
        &value,
        || {
            let verb = if changed.is_empty() {
                "already as asked, nothing changed".to_owned()
            } else {
                format!("updated {}", changed.join(", "))
            };
            format!(
                "{}  ({})\n{}  ({verb}; {}, lead {})",
                p.name,
                p.slug_id,
                p.url,
                p.status.name,
                person(&p.lead)
            )
        },
        || p.slug_id.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- status update

#[derive(Debug, Args)]
pub struct StatusUpdateCmd {
    /// Project id, slug id, URL or name
    pub project: String,
    /// How the project is doing
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
    update: &'a StatusUpdate,
}

pub fn status_update(ctx: &Ctx, cmd: &StatusUpdateCmd) -> Result<()> {
    let body = read_text(&cmd.body_file)?;
    if body.trim().is_empty() {
        return Err(CliError::usage(
            "the status update is empty: say where the project stands, what comes next and what it waits for",
        ));
    }
    let health_type = match cmd.health.as_str() {
        "onTrack" => ProjectUpdateHealthType::OnTrack,
        "atRisk" => ProjectUpdateHealthType::AtRisk,
        _ => ProjectUpdateHealthType::OffTrack,
    };

    let ws = ctx.write_session()?;
    let project = resolve::project(&ws, &cmd.project)?;
    // A status update is a write to the project: it follows the same ownership as changing it.
    ws.guard(
        &Write::ProjectUpdate {
            lead: project.lead.as_ref().map(|u| u.id.inner()),
        },
        false,
    )?;
    ws.validate(&Draft::new(Operation::ProjectUpdate))?;

    let input = StatusUpdateCreateInput {
        project_id: project.id.inner().to_owned(),
        health: health_type,
        body: body.trim().to_owned(),
    };
    let data: ProjectUpdateCreate = ws.client.execute(&pw::project_update_create(input))?;
    if !data.project_update_create.success {
        return Err(CliError::general(
            "Linear could not write the status update",
        ));
    }
    let update = &data.project_update_create.project_update;
    let value = StatusUpdated {
        workspace: &ws.workspace,
        update,
    };
    ctx.out.emit(
        &value,
        || {
            format!(
                "{}  {}  {}",
                update.project.name,
                health(&update.health),
                update.url
            )
        },
        || update.url.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- reorder

#[derive(Debug, Args)]
pub struct ReorderCmd {
    /// The projects in the order they should end up, top first (ids, slug
    /// ids, URLs or names; space- or comma-separated)
    #[arg(required = true, num_args = 1.., value_delimiter = ',', value_name = "PROJECT")]
    pub projects: Vec<String>,
}

/// What `reorder` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reordered<'a> {
    workspace: &'a str,
    /// The projects (slug ids) that were written, in the requested order.
    updated: Vec<&'a str>,
    /// `true` when they already sat in the requested order.
    unchanged: bool,
}

pub fn reorder(ctx: &Ctx, cmd: &ReorderCmd) -> Result<()> {
    // Checked here, not by clap, so that `A,B` (one comma-separated value) counts as two.
    if cmd.projects.len() < 2 {
        return Err(CliError::usage("reordering needs at least two projects"));
    }
    let ws = ctx.write_session()?;

    let mut projects = Vec::new();
    for reference in &cmd.projects {
        let found = resolve_project(&ws.client, reference.trim())?;
        let data: ProjectOrderQuery = ws.client.execute(&pw::project_order(found.id.inner()))?;
        projects.push(data.project);
    }

    // Every project must be writable before the first one is written.
    for p in &projects {
        ws.guard(
            &Write::ProjectUpdate {
                lead: p.lead.as_ref().map(|u| u.id.inner()),
            },
            false,
        )?;
    }
    ws.validate(&Draft::new(Operation::ProjectUpdate))?;

    let rows: Vec<OrderRow> = projects
        .iter()
        .map(|p| OrderRow {
            identifier: p.slug_id.clone(),
            sort_order: p.sort_order,
            priority_sort_order: p.priority_sort_order,
        })
        .collect();
    let wanted: Vec<String> = rows.iter().map(|r| r.identifier.clone()).collect();
    let plan = reorder::plan(&rows, &wanted)?;

    // Each write is undone (back to the old values) if a later one fails.
    let mut rollback = Rollback::new();
    for change in &plan {
        let project = projects
            .iter()
            .find(|p| p.slug_id == change.identifier)
            .expect("the plan only names the projects given");
        let input = ProjectUpdateInput {
            sort_order: change.sort_order,
            priority_sort_order: change.priority_sort_order,
            ..Default::default()
        };
        let step = ws
            .client
            .execute::<_, _, ProjectUpdate>(&pw::project_update(project.id.inner(), input))
            .and_then(|d| {
                if d.project_update.success {
                    Ok(())
                } else {
                    Err(CliError::general(format!(
                        "Linear could not reorder {}",
                        project.name
                    )))
                }
            });
        if let Err(cause) = step {
            return Err(rollback.fail(cause));
        }
        let client = &ws.client;
        let restore = ProjectUpdateInput {
            sort_order: change.sort_order.map(|_| project.sort_order),
            priority_sort_order: change
                .priority_sort_order
                .map(|_| project.priority_sort_order),
            ..Default::default()
        };
        rollback.on_failure(format!("restored {}", project.name), move || {
            client
                .execute::<_, _, ProjectUpdate>(&pw::project_update(project.id.inner(), restore))
                .map(|_| ())
        });
    }

    let value = Reordered {
        workspace: &ws.workspace,
        updated: plan.iter().map(|c| c.identifier.as_str()).collect(),
        unchanged: plan.is_empty(),
    };
    ctx.out.emit(
        &value,
        || {
            if plan.is_empty() {
                "Already in that order; nothing changed.".to_owned()
            } else {
                format!("Reordered: {}", wanted.join(", "))
            }
        },
        || {
            plan.iter()
                .map(|c| c.identifier.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

// ---------------------------------------------------------------- shared

/// `text` with the blanks trimmed. Given but blank is a usage error naming `what`.
fn non_blank<'a>(text: Option<&'a str>, what: &str) -> Result<Option<&'a str>> {
    match text.map(str::trim) {
        Some("") => Err(CliError::usage(format!("{what} is empty"))),
        other => Ok(other),
    }
}

/// The body read from `--body-file`, which must not be blank.
fn read_body(path: Option<&std::path::Path>) -> Result<Option<String>> {
    let body = path.map(read_text).transpose()?;
    if body.as_deref().is_some_and(|b| b.trim().is_empty()) {
        return Err(CliError::usage("the body file is empty"));
    }
    Ok(body)
}

/// An initiative by id, slug id, URL or name.
fn initiative(ws: &WriteSession, reference: &str) -> Result<Initiative> {
    let all = paginate(INITIATIVE_LIST_PAGE_SIZE, None, |page| {
        let data: InitiativeList = ws
            .client
            .execute(&read::initiative_list(InitiativeListVars::new(page, None)))?;
        Ok(data.initiatives)
    })?
    .items;
    Ok(match_initiative(&all, reference)?.clone())
}

/// Put `project_id` under `initiative`. Linear sometimes needs a moment after
/// a project is created, so a failure is tried again.
fn link_initiative(ws: &WriteSession, initiative: &Initiative, project_id: &str) -> Result<()> {
    retry(ws.out, "linking the initiative", &ATTACH_WAITS, || {
        let r: pw::InitiativeToProjectCreate = ws.client.execute(
            &pw::initiative_to_project_create(initiative.id.inner(), project_id),
        )?;
        if r.initiative_to_project_create.success {
            Ok(())
        } else {
            Err(CliError::general(format!(
                "Linear could not put the project under {}",
                initiative.name
            )))
        }
    })
}
