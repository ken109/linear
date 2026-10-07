//! `linear status`: the cache in one line, for a statusline or a hook.
//!
//! It reads the cache and nothing else: no credentials, no request, so it
//! returns at once. A snapshot older than the TTL, or no snapshot, is reported
//! as unknown and carries no numbers: the line never shows old figures as if
//! they were current. Starting a refresh (`linear cache refresh`, in the
//! background) is up to the caller.

use super::cache::{age_text, selected};
use super::cached::{judge, Kind, Verdict};
use super::Ctx;
use crate::cache::CacheDir;
use crate::error::Result;
use crate::store;
use chrono::{DateTime, Utc};
use clap::Args;
use linear_core::cache::DEFAULT_TTL_SECS;
use serde::Serialize;

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// How old a snapshot may be before it counts as unknown
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_TTL_SECS)]
    pub ttl: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum State {
    /// A snapshot within the TTL: the numbers are current.
    Fresh,
    /// A snapshot older than the TTL.
    Expired,
    /// No snapshot.
    Missing,
    /// A file that cannot be used (another schema version, not valid).
    Unusable,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Expired => "expired",
            Self::Missing => "missing",
            Self::Unusable => "unusable",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    workspace: String,
    state: State,
    ttl_secs: u64,
    /// When the snapshot was taken (a fresh one only).
    fetched_at: Option<DateTime<Utc>>,
    age_secs: Option<u64>,
    /// The issues assigned to you that are In Progress. `null` unless fresh.
    in_progress: Option<usize>,
    /// All findings of the audit. `null` unless fresh.
    findings: Option<usize>,
    /// The findings about things you own. `null` unless fresh.
    actionable: Option<usize>,
    /// The findings that were not in the previous snapshot. `null` unless fresh.
    new_findings: Option<usize>,
    /// The last refresh failed (the snapshot, still within the TTL, is from before it).
    refresh_failed: bool,
    /// Why there are no numbers.
    reason: Option<String>,
    /// The line `linear status` prints for this workspace.
    line: String,
}

pub fn run(ctx: &Ctx, args: &StatusArgs) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let targets = selected(ctx, &config)?;
    let dir = CacheDir::from_env()?;
    let now = Utc::now();

    let mut rows = Vec::new();
    for t in &targets {
        rows.push(row(judge(&dir, &t.name, args.ttl, now)?, &t.name, args.ttl));
    }

    ctx.out.emit(
        &rows,
        || {
            rows.iter()
                .map(|r| r.line.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        },
        || {
            rows.iter()
                .map(|r| format!("{} {}", r.workspace, r.state.word()))
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

fn row(verdict: Verdict, workspace: &str, ttl_secs: u64) -> Row {
    match verdict {
        Verdict::Fresh(hit) => {
            let mine = &hit.mine;
            let actionable = mine.findings.iter().filter(|f| f.actionable).count();
            let refresh_failed = hit.failure.is_some();
            let mut line = format!(
                "{workspace}: {} in progress, {actionable} actionable",
                mine.issues.len()
            );
            if refresh_failed {
                line.push_str(" (last refresh failed)");
            }
            Row {
                workspace: workspace.to_owned(),
                state: State::Fresh,
                ttl_secs,
                fetched_at: Some(hit.fetched_at),
                age_secs: Some(hit.age_secs),
                in_progress: Some(mine.issues.len()),
                findings: Some(mine.findings.len()),
                actionable: Some(actionable),
                new_findings: Some(mine.new_findings.len()),
                refresh_failed,
                reason: hit.failure.as_ref().map(|f| f.message.clone()),
                line,
            }
        }
        Verdict::Unknown(u) => {
            let (state, short) = match u.kind {
                Kind::Missing => (State::Missing, "nothing cached".to_owned()),
                Kind::Unusable => (State::Unusable, "unusable cache file".to_owned()),
                Kind::Expired => (
                    State::Expired,
                    format!("snapshot {} old", age_text(u.age_secs.unwrap_or(0))),
                ),
            };
            Row {
                workspace: workspace.to_owned(),
                state,
                ttl_secs,
                fetched_at: None,
                age_secs: u.age_secs,
                in_progress: None,
                findings: None,
                actionable: None,
                new_findings: None,
                refresh_failed: false,
                reason: Some(u.why),
                line: format!("{workspace}: unknown ({short})"),
            }
        }
    }
}
