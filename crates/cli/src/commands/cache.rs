//! `linear cache refresh|show|clear`.
//!
//! The cache is for readers that must not wait for Linear (a hook, a
//! statusline). Ordinary commands never read it; `--cached` is the way in.

use super::audit::collect;
use super::{verify, Ctx};
use crate::cache::{CacheDir, Loaded};
use crate::error::{CliError, Result};
use crate::output::table;
use crate::store;
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use linear_core::audit::{audit, AuditConfig, Finding};
use linear_core::cache::{
    Fetched, Freshness, Mine, RefreshStatus, WorkspaceCache, DEFAULT_TTL_SECS,
};
use linear_core::config::{Config, WorkspaceConfig};
use linear_core::types::{Issue, StateType};
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Fetch what the viewer has in progress and the audit result, and store them
    ///
    /// Without --workspace (or LINEAR_WORKSPACE, or a .linear.toml) every
    /// configured workspace is refreshed. A workspace that cannot be reached
    /// keeps its old snapshot, with the failure recorded beside it; the command
    /// then exits with code 1.
    Refresh,
    /// Show what is cached, how old it is and whether it can be trusted
    Show(ShowArgs),
    /// Remove cached entries (every one, unless a workspace is selected)
    Clear,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// How old an entry may be before it counts as unknown
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_TTL_SECS)]
    pub ttl: u64,
}

pub fn run(ctx: &Ctx, cmd: &CacheCommand) -> Result<()> {
    match cmd {
        CacheCommand::Refresh => refresh(ctx),
        CacheCommand::Show(args) => show(ctx, args),
        CacheCommand::Clear => clear(ctx),
    }
}

/// A workspace a batch command works on.
pub struct Target<'a> {
    pub name: String,
    pub config: &'a WorkspaceConfig,
}

/// The workspaces a command that covers several works on: the one selected
/// with `--workspace`, `LINEAR_WORKSPACE` or a `.linear.toml`, otherwise every
/// configured workspace (the configured default only matters to commands that
/// work on one).
pub fn selected<'a>(ctx: &Ctx, config: &'a Config) -> Result<Vec<Target<'a>>> {
    if is_selected(ctx)? {
        let resolved = ctx.resolve(config)?;
        return Ok(vec![Target {
            name: resolved.name,
            config: resolved.config,
        }]);
    }
    if config.workspaces.is_empty() {
        return Err(CliError::usage(
            "no workspace is configured; run `linear workspace add <name> --url-key <key>`",
        ));
    }
    Ok(config
        .workspaces
        .iter()
        .map(|(name, config)| Target {
            name: name.clone(),
            config,
        })
        .collect())
}

/// Did the person choose a workspace (as opposed to leaving it to defaults)?
fn is_selected(ctx: &Ctx) -> Result<bool> {
    let set = |s: Option<&str>| s.is_some_and(|s| !s.trim().is_empty());
    let env = std::env::var("LINEAR_WORKSPACE").ok();
    let repo = store::find_repo_file(&std::env::current_dir()?);
    Ok(set(ctx.workspace_flag.as_deref()) || set(env.as_deref()) || repo.is_some())
}

// ------------------------------------------------------------------ refresh

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum RowStatus {
    Ok,
    Failed,
    /// Another refresh of the workspace is running; this one did nothing.
    Busy,
}

#[derive(Debug, Serialize)]
struct RefreshRow {
    workspace: String,
    status: RowStatus,
    fetched_at: Option<DateTime<Utc>>,
    findings: usize,
    actionable: usize,
    /// Findings that were not in the previous snapshot.
    new_findings: Vec<Finding>,
    message: Option<String>,
    /// When the snapshot kept through a failure is from.
    kept_from: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct RefreshOut {
    workspaces: Vec<RefreshRow>,
    /// Workspaces that could not be reached (their old snapshot, if any, was kept).
    unreachable: Vec<String>,
}

fn refresh(ctx: &Ctx) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let targets = selected(ctx, &config)?;
    let dir = CacheDir::from_env()?;
    let now = Utc::now();

    let rows: Vec<RefreshRow> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|t| {
                let session = ctx.session_for(&t.name, t.config);
                let dir = &dir;
                scope.spawn(move || refresh_workspace(dir, session, &t.name, t.config, now))
            })
            .collect();
        handles
            .into_iter()
            .zip(&targets)
            .map(|(h, t)| {
                h.join().unwrap_or_else(|_| {
                    failed_row(&t.name, "the refresh stopped unexpectedly".to_owned())
                })
            })
            .collect()
    });

    let unreachable: Vec<String> = rows
        .iter()
        .filter(|r| r.status == RowStatus::Failed)
        .map(|r| r.workspace.clone())
        .collect();
    let out = RefreshOut {
        workspaces: rows,
        unreachable,
    };
    ctx.out.emit(
        &out,
        || {
            out.workspaces
                .iter()
                .map(refresh_text)
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            out.workspaces
                .iter()
                .flat_map(|r| {
                    r.new_findings.iter().map(|f| {
                        format!(
                            "{} {} {}",
                            f.workspace,
                            f.rule.as_str(),
                            f.target.identifier
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    );

    if out.unreachable.is_empty() {
        return Ok(());
    }
    let reasons: Vec<String> = out
        .workspaces
        .iter()
        .filter(|r| r.status == RowStatus::Failed)
        .map(|r| {
            format!(
                "{} ({})",
                r.workspace,
                r.message.as_deref().unwrap_or("failed")
            )
        })
        .collect();
    Err(CliError::general(format!(
        "could not refresh: {}",
        reasons.join(", ")
    )))
}

fn failed_row(workspace: &str, message: String) -> RefreshRow {
    RefreshRow {
        workspace: workspace.to_owned(),
        status: RowStatus::Failed,
        fetched_at: None,
        findings: 0,
        actionable: 0,
        new_findings: Vec::new(),
        message: Some(message),
        kept_from: None,
    }
}

fn refresh_text(r: &RefreshRow) -> String {
    match r.status {
        RowStatus::Ok => {
            let mut text = format!(
                "{}: refreshed ({} findings, {} actionable, {} new)",
                r.workspace,
                r.findings,
                r.actionable,
                r.new_findings.len()
            );
            for f in &r.new_findings {
                text.push_str(&format!(
                    "\n  new: [{}] {} {}",
                    f.rule.as_str(),
                    f.target.identifier,
                    f.message
                ));
            }
            text
        }
        RowStatus::Busy => format!("{}: another refresh is running; skipped", r.workspace),
        RowStatus::Failed => {
            let kept = r.kept_from.map_or_else(
                || "nothing cached".to_owned(),
                |t| format!("kept the snapshot from {}", t.format("%Y-%m-%d %H:%M:%SZ")),
            );
            format!(
                "{}: failed: {} ({kept})",
                r.workspace,
                r.message.as_deref().unwrap_or("unknown error")
            )
        }
    }
}

/// Refresh one workspace and store the result, whatever it is.
fn refresh_workspace(
    dir: &CacheDir,
    session: Result<super::listing::Session>,
    name: &str,
    config: &WorkspaceConfig,
    now: DateTime<Utc>,
) -> RefreshRow {
    let _lock = match dir.lock_refresh(name) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return RefreshRow {
                status: RowStatus::Busy,
                message: None,
                ..failed_row(name, String::new())
            }
        }
        Err(e) => return failed_row(name, e.message),
    };
    // A file that cannot be read is replaced, not kept: there is nothing in it to keep.
    let previous = dir.load(name).ok().and_then(Loaded::into_entry);

    let entry = match session.and_then(|s| fetch(&s.client, name, config, now)) {
        Ok(fetched) => WorkspaceCache::refreshed(previous.as_ref(), name, fetched, now),
        Err(e) => WorkspaceCache::failed(previous.as_ref(), name, e.message, now),
    };
    if let Err(e) = dir.save(&entry) {
        return failed_row(name, e.message);
    }
    row_of(&entry)
}

fn row_of(entry: &WorkspaceCache) -> RefreshRow {
    let data = entry.data.as_ref();
    RefreshRow {
        workspace: entry.workspace.clone(),
        status: match entry.status {
            RefreshStatus::Ok => RowStatus::Ok,
            RefreshStatus::Failed => RowStatus::Failed,
        },
        fetched_at: if entry.status == RefreshStatus::Ok {
            entry.fetched_at
        } else {
            None
        },
        findings: data.map_or(0, |d| d.findings.len()),
        actionable: data.map_or(0, |d| d.findings.iter().filter(|f| f.actionable).count()),
        new_findings: if entry.status == RefreshStatus::Ok {
            data.map(|d| d.new_findings.clone()).unwrap_or_default()
        } else {
            Vec::new()
        },
        message: entry.failure.as_ref().map(|f| f.message.clone()),
        kept_from: if entry.status == RefreshStatus::Failed {
            entry.fetched_at
        } else {
            None
        },
    }
}

/// Everything a snapshot needs from Linear, or the first thing that went wrong.
fn fetch(
    client: &crate::http::Client,
    name: &str,
    config: &WorkspaceConfig,
    now: DateTime<Utc>,
) -> Result<Fetched> {
    let who = verify(client, name, &config.url_key)?;
    let audit_config = AuditConfig::from_workspace(config);
    let snapshot = collect(client, name, &audit_config, &[], now)?;
    let report = audit(&snapshot, &audit_config, now);

    let issues: Vec<Issue> = snapshot
        .issues
        .iter()
        .filter(|i| {
            i.assignee.as_ref().is_some_and(|u| u.is_me)
                && i.state.state_type() == StateType::Started
        })
        .cloned()
        .collect();
    let projects = snapshot
        .projects
        .iter()
        .filter(|p| {
            issues
                .iter()
                .any(|i| i.project.as_ref().is_some_and(|r| r.id == p.id))
        })
        .cloned()
        .collect();
    Ok(Fetched {
        viewer: who.viewer,
        issues,
        projects,
        findings: report.findings,
    })
}

// --------------------------------------------------------------------- show

#[derive(Serialize)]
struct ShowRow {
    workspace: String,
    freshness: Freshness,
    ttl_secs: u64,
    /// Why a file that exists cannot be used.
    problem: Option<String>,
    entry: Option<WorkspaceCache>,
}

fn show(ctx: &Ctx, args: &ShowArgs) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let targets = selected(ctx, &config)?;
    let dir = CacheDir::from_env()?;
    let now = Utc::now();

    let mut rows = Vec::new();
    for t in &targets {
        let (entry, problem) = match dir.load(&t.name)? {
            Loaded::Missing => (None, None),
            Loaded::Unusable(why) => (None, Some(why)),
            Loaded::Found(e) => (Some(*e), None),
        };
        let freshness = entry
            .as_ref()
            .map_or(Freshness::Missing, |e| e.freshness(now, args.ttl));
        rows.push(ShowRow {
            workspace: t.name.clone(),
            freshness,
            ttl_secs: args.ttl,
            problem,
            entry,
        });
    }

    ctx.out.emit(
        &rows,
        || show_text(&rows, now),
        || {
            rows.iter()
                .map(|r| format!("{} {}", r.workspace, state_word(&r.freshness)))
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

fn state_word(f: &Freshness) -> &'static str {
    match f {
        Freshness::Fresh { .. } => "fresh",
        Freshness::Expired { .. } => "expired",
        Freshness::Missing => "missing",
    }
}

fn show_text(rows: &[ShowRow], now: DateTime<Utc>) -> String {
    let body: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let data: Option<&Mine> = r.entry.as_ref().and_then(|e| e.data.as_ref());
            let age = match r.freshness {
                Freshness::Fresh { age_secs } | Freshness::Expired { age_secs } => {
                    age_text(age_secs)
                }
                Freshness::Missing => "-".to_owned(),
            };
            let state = match (&r.freshness, r.problem.is_some()) {
                (_, true) => "unusable",
                (Freshness::Expired { .. }, _) => "unknown (expired)",
                (f, _) => state_word(f),
            };
            let failure = r
                .entry
                .as_ref()
                .and_then(|e| e.failure.as_ref())
                .map_or_else(
                    || "-".to_owned(),
                    |f| {
                        format!(
                            "{} ({})",
                            age_text((now - f.at).num_seconds().max(0) as u64),
                            f.message
                        )
                    },
                );
            vec![
                r.workspace.clone(),
                state.to_owned(),
                age,
                data.map_or("-".to_owned(), |d| d.issues.len().to_string()),
                data.map_or("-".to_owned(), |d| {
                    format!(
                        "{}/{}",
                        d.findings.iter().filter(|f| f.actionable).count(),
                        d.findings.len()
                    )
                }),
                data.map_or("-".to_owned(), |d| d.new_findings.len().to_string()),
                failure,
            ]
        })
        .collect();
    let mut text = table(
        &[
            "WORKSPACE",
            "STATE",
            "AGE",
            "IN PROGRESS",
            "FINDINGS (ACTIONABLE/ALL)",
            "NEW",
            "LAST FAILURE",
        ],
        &body,
    );
    for r in rows {
        if let Some(why) = &r.problem {
            text.push_str(&format!("\n{}: unusable cache file: {why}", r.workspace));
        }
    }
    text
}

fn age_text(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

// -------------------------------------------------------------------- clear

#[derive(Serialize)]
struct ClearOut {
    cleared: Vec<String>,
}

fn clear(ctx: &Ctx) -> Result<()> {
    let dir = CacheDir::from_env()?;
    let cleared = if is_selected(ctx)? {
        let config = store::read_config(&ctx.dirs)?;
        let mut names = Vec::new();
        for t in selected(ctx, &config)? {
            if dir.clear(&t.name)? {
                names.push(t.name);
            }
        }
        names
    } else {
        dir.clear_all()?
    };
    let out = ClearOut { cleared };
    ctx.out.emit(
        &out,
        || {
            if out.cleared.is_empty() {
                "Nothing was cached.".to_owned()
            } else {
                format!("Cleared the cache of: {}", out.cleared.join(", "))
            }
        },
        || out.cleared.join("\n"),
    );
    Ok(())
}
