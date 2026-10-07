//! `linear team list|view`.

use super::format::fields;
use super::listing::{paginate, warn_truncated, ListArgs, Listing, Session};
use super::Ctx;
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::matching::match_team;
use linear_core::read::{self, Teams, TEAMS_PAGE_SIZE};
use linear_core::types::Team;
use linear_core::InWorkspace;

#[derive(Debug, Subcommand)]
pub enum TeamCommand {
    /// List teams
    List(ListCmd),
    /// Show one team
    View(ViewCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Team key (ignoring case), name or id
    pub team: String,
}

pub fn run(ctx: &Ctx, cmd: &TeamCommand) -> Result<()> {
    match cmd {
        TeamCommand::List(args) => list(ctx, args),
        TeamCommand::View(args) => view(ctx, args),
    }
}

fn fetch(session: &Session, limit: Option<usize>) -> Result<Listing<Team>> {
    paginate(TEAMS_PAGE_SIZE, limit, |page| {
        let data: Teams = session.client.execute(&read::teams(page))?;
        Ok(data.teams)
    })
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let listing = fetch(&session, args.page.limit())?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    let tagged = InWorkspace::tag_all(&session.workspace, listing.items.clone());
    ctx.out.emit_selectable(
        &tagged,
        || {
            if listing.items.is_empty() {
                return "No teams found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|t| vec![t.key.clone(), t.name.clone()])
                .collect();
            table(&["KEY", "NAME"], &body)
        },
        || {
            listing
                .items
                .iter()
                .map(|t| t.key.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let all = fetch(&session, None)?.items;
    let t = match_team(&all, &args.team)?;
    ctx.out.emit_selectable(
        &InWorkspace::new(&session.workspace, t),
        || {
            fields(&[
                ("Key", t.key.clone()),
                ("Name", t.name.clone()),
                ("Id", t.id.inner().to_owned()),
            ])
        },
        || t.key.clone(),
    )?;
    Ok(())
}
