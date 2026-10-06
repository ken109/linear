//! `linear label list|view`.

use super::listing::{paginate, warn_truncated, ListArgs, Listing, Session};
use super::Ctx;
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::matching::{label_path, match_labels};
use linear_core::read::{self, Labels, LABELS_PAGE_SIZE};
use linear_core::types::Label;
use linear_core::InWorkspace;

#[derive(Debug, Subcommand)]
pub enum LabelCommand {
    /// List labels, with the group each one belongs to
    List(ListCmd),
    /// Show the label(s) with a name or id
    View(ViewCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Label name (ignoring case, or `group/name`) or id
    pub label: String,
}

pub fn run(ctx: &Ctx, cmd: &LabelCommand) -> Result<()> {
    match cmd {
        LabelCommand::List(args) => list(ctx, args),
        LabelCommand::View(args) => view(ctx, args),
    }
}

fn fetch(session: &Session, limit: Option<usize>) -> Result<Listing<Label>> {
    let mut listing = paginate(LABELS_PAGE_SIZE, limit, |page| {
        let data: Labels = session.client.execute(&read::labels(page))?;
        Ok(data.issue_labels)
    })?;
    // Groups first, each followed by its own labels.
    listing.items.sort_by_key(|l| {
        let group = l
            .parent
            .as_ref()
            .map_or_else(|| l.name.clone(), |g| g.name.clone());
        (group.to_lowercase(), !l.is_group, l.name.to_lowercase())
    });
    Ok(listing)
}

fn table_of(labels: &[Label]) -> String {
    if labels.is_empty() {
        return "No labels found.".to_owned();
    }
    let body: Vec<Vec<String>> = labels
        .iter()
        .map(|l| {
            vec![
                label_path(l),
                if l.is_group { "group" } else { "label" }.to_owned(),
                l.color.clone(),
            ]
        })
        .collect();
    table(&["NAME", "KIND", "COLOR"], &body)
}

fn paths(labels: &[Label]) -> String {
    labels.iter().map(label_path).collect::<Vec<_>>().join("\n")
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let listing = fetch(&session, args.page.limit())?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    let tagged = InWorkspace::tag_all(&session.workspace, listing.items.clone());
    ctx.out.emit(
        &tagged,
        || table_of(&listing.items),
        || paths(&listing.items),
    );
    Ok(())
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let all = fetch(&session, None)?.items;
    // `group/name` addresses a child by its path.
    let by_path: Vec<Label> = all
        .iter()
        .filter(|l| label_path(l).eq_ignore_ascii_case(&args.label))
        .cloned()
        .collect();
    let found: Vec<Label> = if by_path.is_empty() {
        match_labels(&all, &args.label)?
            .into_iter()
            .cloned()
            .collect()
    } else {
        by_path
    };
    let tagged = InWorkspace::tag_all(&session.workspace, found.clone());
    ctx.out.emit(&tagged, || table_of(&found), || paths(&found));
    Ok(())
}
