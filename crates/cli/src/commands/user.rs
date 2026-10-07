//! `linear user list|view`.

use super::format::fields;
use super::listing::{paginate, warn_truncated, ListArgs, Listing, Session};
use super::Ctx;
use crate::error::{CliError, Result};
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::matching::match_user;
use linear_core::read::{self, UserListVars, Users, USERS_PAGE_SIZE};
use linear_core::types::User;
use linear_core::InWorkspace;

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// List users
    List(ListCmd),
    /// Show one user
    View(ViewCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Include disabled (suspended) users
    #[arg(long)]
    pub include_disabled: bool,
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// `me`, an email, a name, a display name or an id
    pub user: String,
}

pub fn run(ctx: &Ctx, cmd: &UserCommand) -> Result<()> {
    match cmd {
        UserCommand::List(args) => list(ctx, args),
        UserCommand::View(args) => view(ctx, args),
    }
}

fn fetch(session: &Session, include_disabled: bool, limit: Option<usize>) -> Result<Listing<User>> {
    paginate(USERS_PAGE_SIZE, limit, |page| {
        let vars = UserListVars::new(page, include_disabled);
        let data: Users = session.client.execute(&read::users(vars))?;
        Ok(data.users)
    })
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let listing = fetch(&session, args.include_disabled, args.page.limit())?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    let tagged = InWorkspace::tag_all(&session.workspace, listing.items.clone());
    ctx.out.emit_selectable(
        &tagged,
        || {
            if listing.items.is_empty() {
                return "No users found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|u| {
                    vec![
                        format!("{}{}", u.name, if u.is_me { " (me)" } else { "" }),
                        u.display_name.clone(),
                        u.email.clone(),
                        if u.active { "yes" } else { "no" }.to_owned(),
                    ]
                })
                .collect();
            table(&["NAME", "DISPLAY NAME", "EMAIL", "ACTIVE"], &body)
        },
        || {
            listing
                .items
                .iter()
                .map(|u| u.email.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    // Disabled users can be looked up too: a lead or an assignee may have left.
    let all = fetch(&session, true, None)?.items;
    let u = if args.user.eq_ignore_ascii_case("me") {
        all.iter()
            .find(|u| u.is_me)
            .ok_or_else(|| CliError::general("Linear did not report which user is you"))?
    } else {
        match_user(&all, &args.user)?
    };
    ctx.out.emit_selectable(
        &InWorkspace::new(&session.workspace, u),
        || {
            fields(&[
                ("Name", u.name.clone()),
                ("Display name", u.display_name.clone()),
                ("Email", u.email.clone()),
                ("Active", if u.active { "yes" } else { "no" }.to_owned()),
                ("Me", if u.is_me { "yes" } else { "no" }.to_owned()),
                ("Id", u.id.inner().to_owned()),
            ])
        },
        || u.email.clone(),
    )?;
    Ok(())
}
