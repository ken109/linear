//! What the read commands share: opening a session, paging with a limit, and
//! resolving references (a project given by name, slug or URL).

use super::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use clap::Args;
use linear_core::config::WorkspaceConfig;
use linear_core::matching::match_project;
use linear_core::read::{self, ProjectRefs, PROJECT_REFS_PAGE_SIZE};
use linear_core::types::{PageVars, ProjectRef};
use linear_core::wire::{Page, Pager};

/// A resolved workspace and a client that talks to it.
pub struct Session {
    pub workspace: String,
    pub client: Client,
}

impl Ctx {
    /// Resolve the workspace and open a client with its credentials.
    pub fn session(&self) -> Result<Session> {
        let config = crate::store::read_config(&self.dirs)?;
        let resolved = self.resolve(&config)?;
        self.session_for(&resolved.name, resolved.config)
    }

    /// Open a client with the credentials of a named workspace.
    pub fn session_for(&self, name: &str, workspace: &WorkspaceConfig) -> Result<Session> {
        let (credential, _) = self.credential(name)?;
        if credential.method() != workspace.auth {
            return Err(CliError::auth(format!(
                "workspace {name:?} is configured for {} but the stored credentials are {}",
                workspace.auth,
                credential.method()
            )));
        }
        Ok(Session {
            workspace: name.to_owned(),
            client: Client::new(credential),
        })
    }
}

/// Items from a paged listing and whether more were left behind.
pub struct Listing<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

/// How many results a list command returns.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Maximum number of results
    #[arg(long, value_name = "N", default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..), conflicts_with = "all")]
    pub limit: u32,
    /// Fetch every page instead of stopping at --limit
    #[arg(long)]
    pub all: bool,
}

impl ListArgs {
    /// `None` means every page.
    pub fn limit(&self) -> Option<usize> {
        (!self.all).then_some(self.limit as usize)
    }
}

/// Fetch pages with `fetch` until the listing ends or `limit` items are in hand.
pub fn paginate<P: Page>(
    page_size: i32,
    limit: Option<usize>,
    mut fetch: impl FnMut(PageVars) -> Result<P>,
) -> Result<Listing<P::Item>> {
    let size = match limit {
        Some(l) => page_size.min(i32::try_from(l).unwrap_or(i32::MAX)).max(1),
        None => page_size,
    };
    let mut pager = Pager::new(size);
    let mut items = Vec::new();
    let mut truncated = false;
    while let Some(vars) = pager.next_vars() {
        items.extend(pager.accept(fetch(vars)?)?);
        if let Some(l) = limit {
            if items.len() >= l {
                truncated = items.len() > l || pager.next_vars().is_some();
                items.truncate(l);
                break;
            }
        }
    }
    Ok(Listing { items, truncated })
}

/// Say on stderr that a listing was cut short. Stderr even with `--json` or
/// `--quiet`: a script that reads a partial list should be able to notice.
pub fn warn_truncated(listing_len: usize) {
    eprintln!(
        "note: showing the first {listing_len} results; more exist (use --limit <N> or --all)"
    );
}

/// Every project, reduced to what is needed to resolve a reference.
fn all_project_refs(client: &Client) -> Result<Vec<ProjectRef>> {
    Ok(paginate(PROJECT_REFS_PAGE_SIZE, None, |vars| {
        let data: ProjectRefs = client.execute(&read::project_refs(vars))?;
        Ok(data.projects)
    })?
    .items)
}

/// A project by id, slug id, URL or name. Fails with the candidates when the
/// reference matches nothing or more than one project.
pub fn resolve_project(client: &Client, reference: &str) -> Result<ProjectRef> {
    let rows = all_project_refs(client)?;
    Ok(match_project(&rows, reference)?.clone())
}
