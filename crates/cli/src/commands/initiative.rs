//! `linear initiative list|view|create` (the write lives in `write::initiative`).

use super::format::{fields, initiative_status, opt_date, person, project_status_type};
use super::listing::{paginate, warn_truncated, ListArgs, Session};
use super::{write, Ctx};
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::filters::InitiativeQuery;
use linear_core::matching::match_initiative;
use linear_core::read::{
    self, InitiativeDetail, InitiativeList, InitiativeListVars, InitiativeView,
    INITIATIVE_LIST_PAGE_SIZE,
};
use linear_core::types::Initiative;
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
            if let Some(desc) = d.description.as_deref().filter(|s| !s.trim().is_empty()) {
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
