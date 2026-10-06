//! `linear brief`: where each unfinished project stands, as markdown.
//!
//! The projects that are in progress or have a status update, with health, the
//! start of the latest update and a mark when there is none or it is old (the
//! logic is `linear_core::brief`). It asks Linear; it does not read the cache,
//! which holds only the projects of your issues In Progress.
//!
//! `--session` is the SessionStart-hook form. It must never get in the way of
//! a session starting: it prints nothing in CI, prints nothing and exits 0 on
//! any failure (no credentials, offline, a bad key, a Linear error), and gives
//! up after [`SESSION_BUDGET`] in total, whatever is still running.

use super::listing::paginate;
use super::Ctx;
use crate::error::{CliError, Result};
use crate::store::Dirs;
use chrono::{DateTime, FixedOffset, Local, Offset, Utc};
use clap::Args;
use linear_core::audit::AuditConfig;
use linear_core::brief::{self, Brief, BriefOptions};
use linear_core::filters::ProjectQuery;
use linear_core::queries::PROJECTS_PAGE_SIZE;
use linear_core::read::{self, ProjectList, ProjectListVars};
use std::sync::mpsc;
use std::time::Duration;

/// Test-only: an RFC 3339 time (`2026-10-20T21:00:00+09:00`) that replaces the clock and the
/// time zone, so that the ages and dates in the output do not depend on when and where the
/// tests run. Read by `brief` only, and not documented for users.
pub const NOW_ENV: &str = "LINEAR_NOW";

/// How long `--session` may take in all, from reading the configuration to
/// the last response.
pub const SESSION_BUDGET: Duration = Duration::from_secs(4);

#[derive(Debug, Args)]
pub struct BriefArgs {
    /// For a SessionStart hook: print nothing and exit 0 when anything fails or when the
    /// CI environment variable is set, and give up after 4 seconds
    #[arg(long)]
    pub session: bool,
    /// A status update this many days old (or more) is marked stale
    /// [default: the workspace's audit.status_update_days, 14]
    #[arg(long, value_name = "DAYS")]
    pub stale_days: Option<u32>,
}

pub fn run(ctx: &Ctx, args: &BriefArgs) -> Result<()> {
    if args.session {
        session(ctx, args);
        return Ok(());
    }
    let (brief, offset) = fetch(ctx, args.stale_days)?;
    print(
        ctx,
        &brief,
        offset,
        "No project to show: none is in progress and none has a status update.",
    );
    Ok(())
}

/// The brief of the resolved workspace, asked of Linear.
fn fetch(ctx: &Ctx, stale_days: Option<u32>) -> Result<(Brief, FixedOffset)> {
    let (now, offset) = clock()?;
    let session = ctx.session()?;
    let filter = ProjectQuery {
        open: true,
        ..ProjectQuery::default()
    }
    .filter();
    let listing = paginate(PROJECTS_PAGE_SIZE, None, |page| {
        let vars = ProjectListVars::new(page, filter.clone());
        let data: ProjectList = session.client.execute(&read::project_list(vars))?;
        Ok(data.projects)
    })?;
    let stale_days = stale_days
        .unwrap_or_else(|| AuditConfig::from_workspace(&session.config).status_update_days);
    let brief = brief::build(
        &session.workspace,
        &listing.items,
        now,
        &BriefOptions { stale_days, offset },
    );
    Ok((brief, offset))
}

/// The current time and the offset dates are shown in: the machine's, unless
/// [`NOW_ENV`] sets both.
fn clock() -> Result<(DateTime<Utc>, FixedOffset)> {
    match std::env::var(NOW_ENV) {
        Ok(v) if !v.is_empty() => {
            let t = DateTime::parse_from_rfc3339(&v)
                .map_err(|e| CliError::usage(format!("{NOW_ENV} is not an RFC 3339 time ({e})")))?;
            Ok((t.with_timezone(&Utc), *t.offset()))
        }
        _ => Ok((Utc::now(), Local::now().offset().fix())),
    }
}

fn print(ctx: &Ctx, brief: &Brief, offset: FixedOffset, when_empty: &str) {
    ctx.out.emit(
        brief,
        || {
            let text = brief::render_markdown(brief, offset);
            if text.is_empty() {
                when_empty.to_owned()
            } else {
                text
            }
        },
        || {
            brief
                .projects
                .iter()
                .map(|p| p.slug_id.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
}

/// `--session`: the work runs on its own thread, so that the deadline holds
/// even while a request is stuck. A result that comes late, or never, or as an
/// error, is dropped without a word; returning from here ends the process and
/// the thread with it.
fn session(ctx: &Ctx, args: &BriefArgs) {
    if std::env::var("CI").is_ok_and(|v| !v.is_empty()) {
        return;
    }
    let workspace_flag = ctx.workspace_flag.clone();
    let out = ctx.out;
    let stale_days = args.stale_days;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = Dirs::from_env().and_then(|dirs| {
            let ctx = Ctx {
                dirs,
                out,
                workspace_flag,
            };
            fetch(&ctx, stale_days)
        });
        let _ = tx.send(result);
    });
    if let Ok(Ok((brief, offset))) = rx.recv_timeout(SESSION_BUDGET) {
        print(ctx, &brief, offset, "");
    }
}
