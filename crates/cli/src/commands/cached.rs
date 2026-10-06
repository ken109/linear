//! `--cached`: answering a read command from the last `linear cache refresh`.
//!
//! A command that takes `--cached` never falls back to Linear and never
//! serves a snapshot it cannot vouch for: no entry, an unreadable one, one
//! for another workspace or schema version, or one past the TTL all fail with
//! exit code 1 and say why. Unknown is not "nothing there".

use super::cache::age_text;
use super::Ctx;
use crate::cache::{CacheDir, Loaded};
use crate::error::{CliError, Result};
use crate::store;
use chrono::{DateTime, Utc};
use clap::Args;
use linear_core::cache::{Failure, Freshness, Mine, DEFAULT_TTL_SECS};
use serde::Serialize;

/// The flags a read command takes to be answered from the cache.
#[derive(Debug, Args)]
pub struct CachedArgs {
    /// Read the last `linear cache refresh` instead of asking Linear. Only what the cache holds
    /// (your issues In Progress and their projects) can be read, so filters outside that are a
    /// usage error. Fails (exit 1) when there is no entry within the TTL: stale data is unknown
    #[arg(long)]
    pub cached: bool,
    /// With --cached: how old the snapshot may be
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_TTL_SECS, requires = "cached")]
    pub ttl: u64,
}

/// A snapshot that may be trusted, and where it came from.
#[derive(Debug)]
pub struct Hit {
    pub workspace: String,
    pub mine: Mine,
    pub fetched_at: DateTime<Utc>,
    pub age_secs: u64,
    /// The last refresh failed, though the snapshot it left is still within the TTL.
    pub failure: Option<Failure>,
}

/// Why an entry cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// There is no snapshot: no file, or a refresh that never succeeded.
    Missing,
    /// There is a file this build cannot use (another schema version or
    /// workspace, not valid JSON).
    Unusable,
    /// There is a snapshot, but it is older than the TTL.
    Expired,
}

#[derive(Debug)]
pub struct Unknown {
    pub kind: Kind,
    /// For people: what is wrong.
    pub why: String,
    /// The age of an expired snapshot.
    pub age_secs: Option<u64>,
}

/// Whether one workspace's entry can be used.
#[derive(Debug)]
pub enum Verdict {
    Fresh(Box<Hit>),
    Unknown(Unknown),
}

/// An entry that cannot be used for a reason other than its age.
fn unknown(kind: Kind, why: impl Into<String>) -> Verdict {
    Verdict::Unknown(Unknown {
        kind,
        why: why.into(),
        age_secs: None,
    })
}

/// Decide whether the entry of `workspace` is usable at `now` with this TTL.
pub fn judge(dir: &CacheDir, workspace: &str, ttl: u64, now: DateTime<Utc>) -> Result<Verdict> {
    let mut entry = match dir.load(workspace)? {
        Loaded::Found(entry) => entry,
        Loaded::Missing => return Ok(unknown(Kind::Missing, "nothing cached")),
        Loaded::Unusable(why) => {
            return Ok(unknown(
                Kind::Unusable,
                format!("unusable cache file ({why})"),
            ))
        }
    };
    if entry.workspace != workspace {
        return Ok(unknown(
            Kind::Unusable,
            format!(
                "unusable cache file (it holds workspace {:?})",
                entry.workspace
            ),
        ));
    }
    let freshness = entry.freshness(now, ttl);
    let Freshness::Fresh { age_secs } = freshness else {
        return Ok(match (freshness, &entry.failure) {
            (Freshness::Expired { age_secs }, failure) => {
                let why = match failure {
                    Some(f) => format!(
                        "snapshot is {age_secs}s old and the last refresh failed: {}",
                        f.message
                    ),
                    None => format!("snapshot is {age_secs}s old, past the {ttl}s TTL"),
                };
                Verdict::Unknown(Unknown {
                    kind: Kind::Expired,
                    why,
                    age_secs: Some(age_secs),
                })
            }
            (_, Some(f)) => unknown(
                Kind::Missing,
                format!("never fetched; the refresh failed: {}", f.message),
            ),
            (_, None) => unknown(Kind::Missing, "never fetched"),
        });
    };
    match (entry.data.take(), entry.fetched_at) {
        (Some(mine), Some(fetched_at)) => Ok(Verdict::Fresh(Box::new(Hit {
            workspace: workspace.to_owned(),
            mine,
            fetched_at,
            age_secs,
            failure: entry.failure.take(),
        }))),
        _ => Ok(unknown(Kind::Missing, "never fetched")),
    }
}

/// The error for entries that cannot be trusted. `reasons` are
/// `<workspace>: <why>` lines.
pub fn unknown_error(reasons: &[String]) -> CliError {
    CliError::general(format!(
        "no fresh cache, so the result is unknown ({}); run `linear cache refresh`",
        reasons.join("; ")
    ))
}

/// The snapshot of the workspace this command works on, or the reason there
/// is none. Reads no credentials and makes no request.
pub fn read(ctx: &Ctx, args: &CachedArgs) -> Result<Hit> {
    let config = store::read_config(&ctx.dirs)?;
    let name = ctx.resolve(&config)?.name;
    let dir = CacheDir::from_env()?;
    match judge(&dir, &name, args.ttl, Utc::now())? {
        Verdict::Fresh(hit) => Ok(*hit),
        Verdict::Unknown(u) => Err(unknown_error(&[format!("{name}: {}", u.why)])),
    }
}

/// Say on stderr that the answer is from the cache and how old it is. Also
/// with `--json`, so that a script reading the same shape as a live answer can
/// still notice; not with `--quiet`.
pub fn announce(ctx: &Ctx, hit: &Hit) {
    if !ctx.out.quiet {
        eprintln!(
            "note: from the cache of {}, fetched {} ago ({}), not from Linear",
            hit.workspace,
            age_text(hit.age_secs),
            hit.fetched_at.format("%Y-%m-%d %H:%M:%SZ")
        );
    }
}

/// Fail with a usage error naming the flags `--cached` cannot be combined
/// with, since the cache does not hold what they would select.
pub fn refuse_outside_cache(what: &str, flags: &[String]) -> Result<()> {
    if flags.is_empty() {
        return Ok(());
    }
    Err(CliError::usage(format!(
        "--cached reads {what}, so it cannot be combined with {}; \
         drop --cached to ask Linear",
        flags.join(", ")
    )))
}
