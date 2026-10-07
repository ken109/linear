//! `linear initiative list|view|status-updates|create|update|archive|unarchive|delete|add-project|remove-project|status-update` (the writes live in `write::initiative`).

use super::format::{
    date_time, fields, first_line, initiative_status, opt_date, person, project_status_type,
};
use super::listing::{paginate, warn_truncated, ListArgs, Session};
use super::{write, Ctx};
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::filters::InitiativeQuery;
use linear_core::initiative_write::{self, InitiativeStatusUpdatesQuery};
use linear_core::matching::match_initiative;
use linear_core::read::{
    self, InitiativeDetail, InitiativeList, InitiativeListVars, InitiativeView,
    INITIATIVE_LIST_PAGE_SIZE,
};
use linear_core::types::{Initiative, InitiativeStatusUpdate};
use linear_core::InWorkspace;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum InitiativeCommand {
    /// List initiatives
    List(ListCmd),
    /// Show one initiative with its projects
    View(ViewCmd),
    /// Create an initiative (the same name returns the existing one instead)
    Create(write::initiative::CreateCmd),
    /// Change an initiative's name, description, status, target date or owner
    ///
    /// Only the fields that differ from now are sent; a run that changes nothing sends
    /// nothing (`changed` is empty with --json). An initiative belongs to the workspace, so no
    /// ownership rule applies (as for `create`).
    Update(write::initiative::UpdateCmd),
    /// Put a project under an initiative
    ///
    /// A project already under it is left alone (nothing is sent). Follows the ownership rules
    /// of changing the project (you may only change a project you lead), as `project update
    /// --initiative` does; the initiative itself has no ownership rule.
    AddProject(write::initiative::ProjectLinkCmd),
    /// Take a project out of an initiative
    ///
    /// Only the link goes: the project and the initiative stay, and `add-project` puts it back.
    /// A project that is not under it is left alone (nothing is sent). Follows the ownership
    /// rules of changing the project.
    RemoveProject(write::initiative::ProjectLinkCmd),
    /// Write a status update on an initiative
    ///
    /// The same shape as `project status-update`: `--health` and the text of the update from
    /// `--body-file`. An initiative belongs to the workspace, so no ownership rule applies.
    StatusUpdate(write::initiative::StatusUpdateCmd),
    /// List the status updates of an initiative, newest first
    StatusUpdates(ViewCmd),
    /// Archive an initiative (restore it with `initiative unarchive`)
    ///
    /// An initiative belongs to the workspace, so no ownership rule applies (as for `create`).
    Archive(write::initiative::InitiativeTargetCmd),
    /// Restore an archived initiative
    ///
    /// Finds the initiative among the archived ones too. No ownership rule applies.
    Unarchive(write::initiative::InitiativeTargetCmd),
    /// Move an initiative to the trash
    ///
    /// Linear keeps a deleted initiative for a while before removing it for good. No ownership
    /// rule applies.
    Delete(write::initiative::InitiativeTargetCmd),
}

/// Linear's spelling of initiative statuses, keyed by what a person types.
const STATUSES: [(&str, &str); 5] = [
    ("active", "Active"),
    ("completed", "Completed"),
    ("canceled", "Canceled"),
    ("planned", "Planned"),
    ("proposed", "Proposed"),
];

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Status (repeatable or comma-separated): active, planned, proposed, completed, canceled
    #[arg(long, value_name = "STATUS", value_delimiter = ',', value_parser = ["active", "completed", "canceled", "planned", "proposed"])]
    pub status: Vec<String>,
    /// Owner: `me`, `none`, an email, or a name
    #[arg(long, value_name = "WHO")]
    pub owner: Option<String>,
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Initiative id, slug id, URL or name
    pub initiative: String,
}

pub fn run(ctx: &Ctx, cmd: &InitiativeCommand) -> Result<()> {
    match cmd {
        InitiativeCommand::List(args) => list(ctx, args),
        InitiativeCommand::View(args) => view(ctx, args),
        InitiativeCommand::Create(args) => write::initiative::create(ctx, args),
        InitiativeCommand::Update(args) => write::initiative::update(ctx, args),
        InitiativeCommand::AddProject(args) => write::initiative::add_project(ctx, args),
        InitiativeCommand::RemoveProject(args) => write::initiative::remove_project(ctx, args),
        InitiativeCommand::StatusUpdate(args) => write::initiative::status_update(ctx, args),
        InitiativeCommand::StatusUpdates(args) => status_updates(ctx, args),
        InitiativeCommand::Archive(args) => write::initiative::archive(ctx, args),
        InitiativeCommand::Unarchive(args) => write::initiative::unarchive(ctx, args),
        InitiativeCommand::Delete(args) => write::initiative::delete(ctx, args),
    }
}

fn fetch(
    session: &Session,
    query: &InitiativeQuery,
    limit: Option<usize>,
) -> Result<super::listing::Listing<Initiative>> {
    let filter = query.filter();
    paginate(INITIATIVE_LIST_PAGE_SIZE, limit, |page| {
        let vars = InitiativeListVars::new(page, filter.clone());
        let data: InitiativeList = session.client.execute(&read::initiative_list(vars))?;
        Ok(data.initiatives)
    })
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let statuses = args
        .status
        .iter()
        .filter_map(|s| {
            STATUSES
                .iter()
                .find(|(k, _)| k == s)
                .map(|(_, v)| (*v).to_owned())
        })
        .collect();
    let query = InitiativeQuery {
        statuses,
        owner: args.owner.clone(),
    };
    let listing = fetch(&session, &query, args.page.limit())?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }

    let tagged = InWorkspace::tag_all(&session.workspace, listing.items.clone());
    ctx.out.emit(
        &tagged,
        || {
            if listing.items.is_empty() {
                return "No initiatives found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|i| {
                    vec![
                        i.slug_id.clone(),
                        i.name.clone(),
                        initiative_status(&i.status),
                        person(&i.owner),
                        opt_date(&i.target_date),
                    ]
                })
                .collect();
            table(&["SLUG", "NAME", "STATUS", "OWNER", "TARGET"], &body)
        },
        || {
            listing
                .items
                .iter()
                .map(|i| i.slug_id.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

#[derive(Serialize)]
struct InitiativeViewOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    initiative: &'a Initiative,
    #[serde(flatten)]
    detail: &'a InitiativeDetail,
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let all = fetch(&session, &InitiativeQuery::default(), None)?.items;
    let found = match_initiative(&all, &args.initiative)?;
    let data: InitiativeView = session
        .client
        .execute(&read::initiative_view(found.id.inner()))?;
    let (i, d) = (&data.initiative, &data.detail);

    let value = InitiativeViewOut {
        workspace: &session.workspace,
        initiative: i,
        detail: d,
    };
    ctx.out.emit(
        &value,
        || {
            let mut text = format!(
                "{}  ({})\n{}\n\n{}",
                i.name,
                i.slug_id,
                i.url,
                fields(&[
                    ("Status", initiative_status(&i.status)),
                    ("Owner", person(&i.owner)),
                    ("Target", opt_date(&i.target_date)),
                ])
            );
            if let Some(desc) = i.description.as_deref().filter(|s| !s.trim().is_empty()) {
                text.push_str(&format!("\n\n{}", desc.trim_end()));
            }
            text.push_str(&format!("\n\nProjects ({})", d.projects.len()));
            if !d.projects.is_empty() {
                let body: Vec<Vec<String>> = d
                    .projects
                    .iter()
                    .map(|p| {
                        vec![
                            p.slug_id.clone(),
                            p.name.clone(),
                            format!(
                                "{} ({})",
                                p.status.name,
                                project_status_type(&p.status.type_)
                            ),
                            opt_date(&p.target_date),
                        ]
                    })
                    .collect();
                text.push('\n');
                text.push_str(&table(&["SLUG", "NAME", "STATUS", "TARGET"], &body));
            }
            text
        },
        || i.slug_id.clone(),
    );
    Ok(())
}

fn status_updates(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let all = fetch(&session, &InitiativeQuery::default(), None)?.items;
    let found = match_initiative(&all, &args.initiative)?;
    let data: InitiativeStatusUpdatesQuery =
        session
            .client
            .execute(&initiative_write::initiative_status_updates(
                found.id.inner(),
            ))?;
    // Newest first, whatever order Linear answers in.
    let mut updates: Vec<&InitiativeStatusUpdate> =
        data.initiative.initiative_updates.iter().collect();
    updates.sort_by_key(|u| std::cmp::Reverse(u.created_at));

    #[derive(Serialize)]
    struct Out<'a> {
        workspace: &'a str,
        updates: &'a [&'a InitiativeStatusUpdate],
    }
    let value = Out {
        workspace: &session.workspace,
        updates: &updates,
    };
    ctx.out.emit(
        &value,
        || {
            if updates.is_empty() {
                return "No status updates.".to_owned();
            }
            let body: Vec<Vec<String>> = updates
                .iter()
                .map(|u| {
                    vec![
                        date_time(&u.created_at),
                        write::initiative::health_word(&u.health),
                        u.user.name.clone(),
                        first_line(&u.body, 60),
                    ]
                })
                .collect();
            table(&["DATE", "HEALTH", "AUTHOR", "UPDATE"], &body)
        },
        || {
            updates
                .iter()
                .map(|u| u.url.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}
