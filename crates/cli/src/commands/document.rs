//! `linear document list|view|create|update` (the writes live in `write::document`).

use super::format::{date_time, fields, opt_text, person};
use super::listing::{paginate, resolve_project, warn_truncated, ListArgs, Session};
use super::{write, Ctx};
use crate::error::Result;
use crate::http::Client;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::docs::{
    self, Doc, DocDetail, DocList, DocListVars, DocQuery, DocView, DOC_LIST_PAGE_SIZE,
};
use linear_core::matching::{match_initiative, slug_from_url};
use linear_core::read::{self, InitiativeList, InitiativeListVars, INITIATIVE_LIST_PAGE_SIZE};
use linear_core::types::Initiative;
use linear_core::InWorkspace;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum DocumentCommand {
    /// List documents, of a project or an initiative or all of them
    List(ListCmd),
    /// Show one document with its body
    View(ViewCmd),
    /// Add a document to a project you lead or an initiative you own (the same title returns that one)
    Create(write::document::CreateCmd),
    /// Change a document's title or body (of a project you lead or an initiative you own)
    Update(write::document::UpdateCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Only the documents of this project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT", conflicts_with = "initiative")]
    pub project: Option<String>,
    /// Only the documents of this initiative: id, slug id, URL or name
    #[arg(long, value_name = "INITIATIVE")]
    pub initiative: Option<String>,
    /// Only documents with this title (ignoring case)
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// The document: id, slug id, or the URL (or its `title-slugid` part)
    pub document: String,
}

pub fn run(ctx: &Ctx, cmd: &DocumentCommand) -> Result<()> {
    match cmd {
        DocumentCommand::List(args) => list(ctx, args),
        DocumentCommand::View(args) => view(ctx, args),
        DocumentCommand::Create(args) => write::document::create(ctx, args),
        DocumentCommand::Update(args) => write::document::update(ctx, args),
    }
}

/// An initiative by id, slug id, URL or name.
pub(super) fn find_initiative(client: &Client, reference: &str) -> Result<Initiative> {
    let all = paginate(INITIATIVE_LIST_PAGE_SIZE, None, |page| {
        let vars = InitiativeListVars::new(page, None);
        let data: InitiativeList = client.execute(&read::initiative_list(vars))?;
        Ok(data.initiatives)
    })?
    .items;
    Ok(match_initiative(&all, reference)?.clone())
}

/// What the API takes for a document given as an id, slug id or URL.
pub(super) fn key_of(reference: &str) -> String {
    slug_from_url(reference, "document")
}

fn fetch(
    session: &Session,
    query: &DocQuery,
    limit: Option<usize>,
) -> Result<super::listing::Listing<Doc>> {
    let filter = query.filter();
    paginate(DOC_LIST_PAGE_SIZE, limit, |page| {
        let vars = DocListVars::new(page, filter.clone());
        let data: DocList = session.client.execute(&docs::doc_list(vars))?;
        Ok(data.documents)
    })
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let query = DocQuery {
        project_id: args
            .project
            .as_deref()
            .map(|p| resolve_project(&session.client, p))
            .transpose()?
            .map(|p| p.id.inner().to_owned()),
        initiative_id: args
            .initiative
            .as_deref()
            .map(|i| find_initiative(&session.client, i))
            .transpose()?
            .map(|i| i.id.inner().to_owned()),
        title: args.title.clone(),
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
                return "No documents found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|d| {
                    vec![
                        d.slug_id.clone(),
                        d.title.clone(),
                        d.parent_label(),
                        date_time(&d.updated_at),
                    ]
                })
                .collect();
            table(&["SLUG", "TITLE", "IN", "UPDATED"], &body)
        },
        || {
            listing
                .items
                .iter()
                .map(|d| d.slug_id.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

#[derive(Serialize)]
struct DocViewOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    document: &'a Doc,
    #[serde(flatten)]
    detail: &'a DocDetail,
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let data: DocView = session
        .client
        .execute(&docs::doc_view(key_of(&args.document)))?;
    let (d, detail) = (&data.document, &data.detail);

    let value = DocViewOut {
        workspace: &session.workspace,
        document: d,
        detail,
    };
    ctx.out.emit(
        &value,
        || {
            let mut rows = vec![("In", d.parent_label())];
            rows.push(("Creator", person(&d.creator)));
            rows.push(("Updated", date_time(&d.updated_at)));
            rows.push(("Url", d.url.clone()));
            let mut text = format!("{}\n\n{}", d.title, fields(&rows));
            match detail.content.as_deref().filter(|c| !c.trim().is_empty()) {
                Some(body) => text.push_str(&format!("\n\n{}", body.trim_end())),
                None => text.push_str(&format!("\n\n{}", opt_text(None))),
            }
            text
        },
        || d.slug_id.clone(),
    );
    Ok(())
}
