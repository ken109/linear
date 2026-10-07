//! `linear project list|view|create|update|reorder|status-update|delete|unarchive` (the writes live in `write::project`).

use super::cached::{self, CachedArgs};
use super::format::{
    date_time, fields, first_line, health, indent, milestone_status, opt_date, percent, person,
    project_status_type,
};
use super::listing::{paginate, resolve_project, warn_truncated, ListArgs};
use super::{write, Ctx};
use crate::error::{CliError, Result};
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::filters::ProjectQuery;
use linear_core::matching::match_project;
use linear_core::queries::PROJECTS_PAGE_SIZE;
use linear_core::read::{
    self, ProjectDetail, ProjectList, ProjectListVars, ProjectView, PROJECT_VIEW_UPDATES,
};
use linear_core::types::{IssueCounts, Project, ProjectRef};
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects with lead, status, target date and the latest status update
    List(ListCmd),
    /// Show one project: milestones, issue counts and status updates
    View(ViewCmd),
    /// Create a project (same name, unfinished: returns the existing one instead)
    Create(write::project::CreateCmd),
    /// Change a project's name, summary, body, status, target date, initiative or lead
    Update(write::project::UpdateCmd),
    /// Put projects in a given order
    Reorder(write::project::ReorderCmd),
    /// Write a status update (health and body) on a project
    StatusUpdate(write::project::StatusUpdateCmd),
    /// Move a project to the trash (restore it with `project unarchive`)
    ///
    /// Linear keeps a deleted project for a while before removing it for good. Only a project
    /// you lead (the ownership rule of `project update`). There is no `project archive`: Linear
    /// has deprecated its archive mutation in favour of this one.
    Delete(write::project::ProjectTargetCmd),
    /// Bring back a deleted (trashed) or archived project
    ///
    /// Finds the project among the deleted ones too. Only a project you lead.
    Unarchive(write::project::ProjectTargetCmd),
}

const STATUS_TYPES: [&str; 6] = [
    "backlog",
    "planned",
    "started",
    "paused",
    "completed",
    "canceled",
];

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Lead: `me`, `none`, an email, or a name
    #[arg(long, value_name = "WHO")]
    pub lead: Option<String>,
    /// Status type (repeatable or comma-separated)
    #[arg(long, value_name = "TYPE", value_delimiter = ',', value_parser = STATUS_TYPES)]
    pub status_type: Vec<String>,
    /// Only projects that are not completed or canceled
    #[arg(long, conflicts_with = "status_type")]
    pub open: bool,
    /// Initiative name, ignoring case
    #[arg(long, value_name = "NAME")]
    pub initiative: Option<String>,
    #[command(flatten)]
    pub page: ListArgs,
    #[command(flatten)]
    pub cache: CachedArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Project id, slug id, URL or name
    pub project: String,
    /// Also print the project's content document (not with --cached)
    #[arg(long, conflicts_with = "cached")]
    pub content: bool,
    #[command(flatten)]
    pub cache: CachedArgs,
}

pub fn run(ctx: &Ctx, cmd: &ProjectCommand) -> Result<()> {
    match cmd {
        ProjectCommand::List(args) => list(ctx, args),
        ProjectCommand::View(args) => view(ctx, args),
        ProjectCommand::Create(args) => write::project::create(ctx, args),
        ProjectCommand::Update(args) => write::project::update(ctx, args),
        ProjectCommand::Reorder(args) => write::project::reorder(ctx, args),
        ProjectCommand::StatusUpdate(args) => write::project::status_update(ctx, args),
        ProjectCommand::Delete(args) => write::project::delete(ctx, args),
        ProjectCommand::Unarchive(args) => write::project::unarchive(ctx, args),
    }
}

/// A project as `--json` prints it: Linear's fields, the workspace, and the
/// issue counts by state type.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    project: &'a Project,
    issue_counts: IssueCounts,
}

pub(crate) fn out<'a>(workspace: &'a str, project: &'a Project) -> ProjectOut<'a> {
    ProjectOut {
        workspace,
        project,
        issue_counts: project.issue_counts(),
    }
}

/// `done/total`, with a `+` when the project has more issues than were counted.
fn issues_done(c: &IssueCounts) -> String {
    format!(
        "{}/{}{}",
        c.completed,
        c.total(),
        if c.complete { "" } else { "+" }
    )
}

fn latest_update(p: &Project) -> String {
    match &p.last_update {
        Some(u) => format!("{} {}", date_time(&u.created_at), health(&u.health)),
        None => "-".to_owned(),
    }
}

/// The cache holds the projects of your issues In Progress, not the projects
/// you lead or any other selection, so no filter can be applied to it.
fn check_cached_filters(args: &ListCmd) -> Result<()> {
    let mut flags = Vec::new();
    if args.lead.is_some() {
        flags.push("--lead".to_owned());
    }
    if !args.status_type.is_empty() {
        flags.push("--status-type".to_owned());
    }
    if args.open {
        flags.push("--open".to_owned());
    }
    if args.initiative.is_some() {
        flags.push("--initiative".to_owned());
    }
    cached::refuse_outside_cache(
        "the projects of the issues assigned to you that are In Progress",
        &flags,
    )
}

/// The projects to list and the workspace they are from.
fn fetch_list(ctx: &Ctx, args: &ListCmd) -> Result<(String, Vec<Project>)> {
    if args.cache.cached {
        check_cached_filters(args)?;
        let hit = cached::read(ctx, &args.cache)?;
        cached::announce(ctx, &hit);
        let mut items = hit.mine.projects;
        if let Some(limit) = args.page.limit() {
            if items.len() > limit {
                items.truncate(limit);
                warn_truncated(items.len());
            }
        }
        return Ok((hit.workspace, items));
    }

    let session = ctx.session()?;
    let filter = ProjectQuery {
        lead: args.lead.clone(),
        status_types: args.status_type.clone(),
        open: args.open,
        initiative: args.initiative.clone(),
    }
    .filter();

    let listing = paginate(PROJECTS_PAGE_SIZE, args.page.limit(), |page| {
        let vars = ProjectListVars::new(page, filter.clone());
        let data: ProjectList = session.client.execute(&read::project_list(vars))?;
        Ok(data.projects)
    })?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    Ok((session.workspace, listing.items))
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let (workspace, items) = fetch_list(ctx, args)?;
    let rows: Vec<ProjectOut> = items.iter().map(|p| out(&workspace, p)).collect();
    ctx.out.emit_selectable(
        &rows,
        || {
            if items.is_empty() {
                return "No projects found.".to_owned();
            }
            let body: Vec<Vec<String>> = items
                .iter()
                .map(|p| {
                    vec![
                        p.slug_id.clone(),
                        p.name.clone(),
                        p.status.name.clone(),
                        person(&p.lead),
                        opt_date(&p.target_date),
                        issues_done(&p.issue_counts()),
                        latest_update(p),
                    ]
                })
                .collect();
            table(
                &[
                    "SLUG",
                    "NAME",
                    "STATUS",
                    "LEAD",
                    "TARGET",
                    "ISSUES",
                    "LATEST UPDATE",
                ],
                &body,
            )
        },
        || {
            items
                .iter()
                .map(|p| p.slug_id.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

#[derive(Serialize)]
struct ProjectViewOut<'a> {
    #[serde(flatten)]
    base: ProjectOut<'a>,
    /// Absent from the cache, which keeps the list fields only.
    #[serde(flatten)]
    detail: Option<&'a ProjectDetail>,
}

/// A project from the cache, matched the way a live reference is: id, slug id,
/// URL or name.
fn find_cached(projects: Vec<Project>, reference: &str) -> Result<Project> {
    let refs: Vec<ProjectRef> = projects
        .iter()
        .map(|p| ProjectRef {
            id: p.id.clone(),
            slug_id: p.slug_id.clone(),
            name: p.name.clone(),
            url: p.url.clone(),
        })
        .collect();
    let id = match_project(&refs, reference)
        .map_err(|e| {
            CliError::general(format!(
                "{e}; the cache holds only the projects of the issues assigned to you that are \
                 In Progress, so drop --cached to ask Linear"
            ))
        })?
        .id
        .clone();
    Ok(projects
        .into_iter()
        .find(|p| p.id == id)
        .expect("the match came from this list"))
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    if args.cache.cached {
        let hit = cached::read(ctx, &args.cache)?;
        let project = find_cached(hit.mine.projects.clone(), &args.project)?;
        cached::announce(ctx, &hit);
        return show(ctx, &hit.workspace, &project, None, false);
    }
    let session = ctx.session()?;
    let found = resolve_project(&session.client, &args.project)?;
    let data: ProjectView = session
        .client
        .execute(&read::project_view(found.id.inner()))?;
    show(
        ctx,
        &session.workspace,
        &data.project,
        Some(&data.detail),
        args.content,
    )
}

fn show(
    ctx: &Ctx,
    workspace: &str,
    p: &Project,
    d: Option<&ProjectDetail>,
    with_content: bool,
) -> Result<()> {
    let value = ProjectViewOut {
        base: out(workspace, p),
        detail: d,
    };
    ctx.out
        .emit_selectable(&value, || render(p, d, with_content), || p.slug_id.clone())?;
    Ok(())
}

fn render(p: &Project, d: Option<&ProjectDetail>, with_content: bool) -> String {
    let c = p.issue_counts();
    let initiatives = if p.initiatives.is_empty() {
        "-".to_owned()
    } else {
        p.initiatives
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let count_text = format!(
        "{} total ({} started, {} unstarted, {} backlog, {} triage, {} completed, {} canceled){}",
        c.total(),
        c.started,
        c.unstarted,
        c.backlog,
        c.triage,
        c.completed,
        c.canceled,
        if c.complete {
            ""
        } else {
            "; the project has more issues than were counted"
        }
    );
    let mut text = format!(
        "{}  ({})\n{}\n\n{}",
        p.name,
        p.slug_id,
        p.url,
        fields(&[
            (
                "Status",
                format!(
                    "{} ({})",
                    p.status.name,
                    project_status_type(&p.status.type_)
                )
            ),
            ("Health", p.health.as_ref().map_or("-".to_owned(), health)),
            ("Lead", person(&p.lead)),
            ("Start", opt_date(&p.start_date)),
            ("Target", opt_date(&p.target_date)),
            ("Initiatives", initiatives),
            ("Issues", count_text),
            ("Updated", date_time(&p.updated_at)),
        ])
    );
    if let Some(d) = d.filter(|d| !d.description.trim().is_empty()) {
        text.push_str(&format!("\n\n{}", d.description.trim_end()));
    }

    if !p.project_milestones.is_empty() {
        let mut ms: Vec<_> = p.project_milestones.iter().collect();
        ms.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
        text.push_str("\n\nMilestones");
        for m in ms {
            text.push_str(&format!(
                "\n  [{}] {} ({}, {}, {})",
                if milestone_status(&m.status) == "done" {
                    "x"
                } else {
                    " "
                },
                m.name,
                opt_date(&m.target_date),
                percent(m.progress),
                milestone_status(&m.status)
            ));
        }
    }

    // `lastUpdate` is Linear's own "latest"; the others come from the update
    // list, newest first.
    match &p.last_update {
        None => text.push_str("\n\nLatest status update: none"),
        Some(u) => {
            text.push_str(&format!(
                "\n\nLatest status update ({}, {}, {})\n{}\n  {}",
                date_time(&u.created_at),
                health(&u.health),
                u.user.name,
                indent(u.body.trim(), 2),
                u.url
            ));
        }
    }
    let latest_id = p.last_update.as_ref().map(|u| u.id.inner());
    let mut earlier: Vec<_> = d
        .map(|d| d.project_updates.iter())
        .into_iter()
        .flatten()
        .filter(|u| Some(u.id.inner()) != latest_id)
        .collect();
    earlier.sort_by_key(|u| std::cmp::Reverse(u.created_at));
    if !earlier.is_empty() {
        text.push_str(&format!(
            "\n\nEarlier status updates (up to {} shown)",
            PROJECT_VIEW_UPDATES
        ));
        for u in earlier {
            text.push_str(&format!(
                "\n  {} {}: {}",
                date_time(&u.created_at),
                health(&u.health),
                first_line(&u.body, 100)
            ));
        }
    }

    if with_content {
        if let Some(content) = d
            .and_then(|d| d.content.as_deref())
            .filter(|s| !s.trim().is_empty())
        {
            text.push_str(&format!("\n\nContent\n{}", indent(content, 2)));
        }
    }
    text
}
