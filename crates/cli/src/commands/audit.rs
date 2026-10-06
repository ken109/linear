//! `linear audit`: find Linear data that has drifted.
//!
//! The rules are pure functions in `linear_core::audit`. This file fetches
//! what they look at, builds each workspace's configuration, prints the
//! findings and turns `--fail-on` into an exit code.

use super::cache::{in_parallel, selected, Target};
use super::listing::{paginate, Session};
use super::{verify, Ctx};
use crate::cache::{CacheDir, Loaded};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::store;
use chrono::{DateTime, Duration, Utc};
use clap::{Args, ValueEnum};
use linear_core::audit::{audit_scoped, AuditConfig, AuditOptions, Finding, Severity, Snapshot};
use linear_core::cache::{Freshness, DEFAULT_TTL_SECS};
use linear_core::config::Rule;
use linear_core::filters;
use linear_core::queries::{self, IssueById, Projects, Templates, PROJECTS_PAGE_SIZE};
use linear_core::read::{self, IssueList, IssueListVars, ISSUE_LIST_PAGE_SIZE};
use linear_core::types::Issue;
use linear_core::ErrorCode;
use serde::Serialize;

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Audit only these issues (comma-separated identifiers, such as KK-1,KK-2) and the
    /// projects they belong to
    #[arg(long, value_name = "ISSUES", value_delimiter = ',')]
    pub issues: Option<Vec<String>>,
    /// With --issues: report each of them that was not updated at or after this time
    /// (RFC 3339, such as 2026-10-06T12:00:00Z)
    #[arg(long, value_name = "TIME", requires = "issues", value_parser = parse_time)]
    pub since: Option<DateTime<Utc>>,
    /// Exit with code 6 when there are findings of this kind
    #[arg(long, value_enum, value_name = "KIND")]
    pub fail_on: Option<FailOn>,
    /// Read the last `linear cache refresh` instead of asking Linear. Fails (exit 1) when a
    /// workspace has no entry within the TTL: stale data is unknown, not clean
    #[arg(long, conflicts_with_all = ["issues", "since"])]
    pub cached: bool,
    /// With --cached: how old an entry may be
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_TTL_SECS, requires = "cached")]
    pub ttl: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    /// Findings about things you own
    Actionable,
}

fn parse_time(s: &str) -> std::result::Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("{e}; expected RFC 3339, such as 2026-10-06T12:00:00Z"))
}

#[derive(Debug, Serialize)]
struct FailedWorkspace {
    workspace: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct CachedFrom {
    workspace: String,
    fetched_at: DateTime<Utc>,
    age_secs: u64,
}

#[derive(Debug, Serialize)]
struct AuditOut {
    findings: Vec<Finding>,
    /// Issues named with --issues that no audited workspace has: they could
    /// not be checked, which is not the same as passing.
    unresolved_issues: Vec<String>,
    /// Workspaces that could not be audited (the command then exits 1).
    failed_workspaces: Vec<FailedWorkspace>,
    /// With --cached: when each snapshot was taken.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cached: Vec<CachedFrom>,
}

pub fn run(ctx: &Ctx, args: &AuditArgs) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let targets = selected(ctx, &config)?;
    let now = Utc::now();

    let out = if args.cached {
        from_cache(&targets, args.ttl, now)?
    } else {
        live(ctx, &targets, args, now)?
    };

    ctx.out.emit(&out, || text(&out), || quiet(&out));
    if !out.unresolved_issues.is_empty() && !ctx.out.json && !ctx.out.quiet {
        eprintln!(
            "note: not found in any audited workspace, so not checked: {}",
            out.unresolved_issues.join(", ")
        );
    }

    let actionable = out.findings.iter().filter(|f| f.actionable).count();
    let unreachable = failure_text(&out.failed_workspaces);
    if args.fail_on == Some(FailOn::Actionable) && actionable > 0 {
        let more = unreachable
            .map(|u| format!("; also {u}"))
            .unwrap_or_default();
        return Err(CliError::new(
            ErrorCode::AuditFindings,
            format!("{actionable} actionable finding(s){more}"),
        ));
    }
    match unreachable {
        Some(u) => Err(CliError::general(u)),
        None => Ok(()),
    }
}

fn failure_text(failed: &[FailedWorkspace]) -> Option<String> {
    if failed.is_empty() {
        return None;
    }
    let list: Vec<String> = failed
        .iter()
        .map(|f| format!("{} ({})", f.workspace, f.message))
        .collect();
    Some(format!("could not audit: {}", list.join(", ")))
}

// -------------------------------------------------------------------- live

fn live(ctx: &Ctx, targets: &[Target], args: &AuditArgs, now: DateTime<Utc>) -> Result<AuditOut> {
    let issues = match &args.issues {
        None => None,
        Some(list) => {
            let named: Vec<String> = list
                .iter()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if named.is_empty() {
                return Err(CliError::usage(
                    "--issues needs at least one identifier, such as KK-1,KK-2",
                ));
            }
            Some(named)
        }
    };
    let options = AuditOptions {
        issues,
        since: args.since,
    };

    let results = in_parallel(targets, |t| {
        ctx.session_for(&t.name, t.config)
            .and_then(|session| audit_workspace(&session, t, &options, now))
    });

    let mut out = AuditOut {
        findings: Vec::new(),
        unresolved_issues: Vec::new(),
        failed_workspaces: Vec::new(),
        cached: Vec::new(),
    };
    // An identifier is unresolved only if no workspace that was audited has it.
    let mut unresolved: Option<Vec<String>> = None;
    for (t, result) in targets.iter().zip(results) {
        match result {
            Ok(report) => {
                out.findings.extend(report.findings);
                unresolved = Some(match unresolved {
                    None => report.unresolved_issues,
                    Some(so_far) => so_far
                        .into_iter()
                        .filter(|id| report.unresolved_issues.contains(id))
                        .collect(),
                });
            }
            Err(e) => out.failed_workspaces.push(FailedWorkspace {
                workspace: t.name.clone(),
                message: e.message,
            }),
        }
    }
    out.unresolved_issues = unresolved.unwrap_or_default();
    Ok(out)
}

fn audit_workspace(
    session: &Session,
    target: &Target,
    options: &AuditOptions,
    now: DateTime<Utc>,
) -> Result<linear_core::audit::AuditReport> {
    verify(&session.client, &target.name, &target.config.url_key)?;
    let config = AuditConfig::from_workspace(target.config);
    let named = options.issues.clone().unwrap_or_default();
    let snapshot = collect(&session.client, &target.name, &config, &named, now)?;
    Ok(audit_scoped(&snapshot, &config, options, now)?)
}

// ------------------------------------------------------------------ cached

/// The findings of the last refresh, for every workspace, or an error naming
/// the workspaces whose entry cannot be trusted.
fn from_cache(targets: &[Target], ttl: u64, now: DateTime<Utc>) -> Result<AuditOut> {
    let dir = CacheDir::from_env()?;
    let mut out = AuditOut {
        findings: Vec::new(),
        unresolved_issues: Vec::new(),
        failed_workspaces: Vec::new(),
        cached: Vec::new(),
    };
    let mut unknown = Vec::new();
    for t in targets {
        let entry = match dir.load(&t.name)? {
            Loaded::Found(entry) => entry,
            Loaded::Missing => {
                unknown.push(format!("{}: nothing cached", t.name));
                continue;
            }
            Loaded::Unusable(why) => {
                unknown.push(format!("{}: unusable cache file ({why})", t.name));
                continue;
            }
        };
        match (entry.known(now, ttl), entry.freshness(now, ttl)) {
            (Some(mine), Freshness::Fresh { age_secs }) => {
                out.findings.extend(mine.findings.iter().cloned());
                out.cached.push(CachedFrom {
                    workspace: t.name.clone(),
                    fetched_at: entry.fetched_at.unwrap_or(now),
                    age_secs,
                });
            }
            (_, freshness) => {
                let why = match (freshness, &entry.failure) {
                    (Freshness::Expired { age_secs }, Some(f)) => format!(
                        "snapshot is {age_secs}s old and the last refresh failed: {}",
                        f.message
                    ),
                    (Freshness::Expired { age_secs }, None) => {
                        format!("snapshot is {age_secs}s old, past the {ttl}s TTL")
                    }
                    (_, Some(f)) => format!("never fetched; the refresh failed: {}", f.message),
                    (_, None) => "never fetched".to_owned(),
                };
                unknown.push(format!("{}: {why}", t.name));
            }
        }
    }
    if !unknown.is_empty() {
        return Err(CliError::general(format!(
            "no fresh cache, so the result is unknown ({}); run `linear cache refresh`",
            unknown.join("; ")
        )));
    }
    Ok(out)
}

// ------------------------------------------------------------------ output

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Warn => "warn",
        Severity::Info => "info",
    }
}

fn text(out: &AuditOut) -> String {
    let mut blocks: Vec<String> = out
        .findings
        .iter()
        .map(|f| {
            format!(
                "{}  {}  {}  {}{}\n  {}\n  {}\n  fix: {}",
                f.workspace,
                severity(f.severity),
                f.rule.as_str(),
                f.target.identifier,
                if f.actionable { "  (actionable)" } else { "" },
                f.message,
                f.target.url,
                f.fix
            )
        })
        .collect();
    if out.findings.is_empty() {
        blocks.push(if out.failed_workspaces.is_empty() {
            "No findings.".to_owned()
        } else {
            "No findings in the workspaces that could be audited.".to_owned()
        });
    } else {
        let actionable = out.findings.iter().filter(|f| f.actionable).count();
        blocks.push(format!(
            "{} findings, {actionable} actionable",
            out.findings.len()
        ));
    }
    blocks.join("\n\n")
}

fn quiet(out: &AuditOut) -> String {
    out.findings
        .iter()
        .map(|f| {
            format!(
                "{} {} {}",
                f.workspace,
                f.rule.as_str(),
                f.target.identifier
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------------- fetch

/// Fetch a workspace's issues, projects and (when `template-sections` is
/// enabled) templates.
///
/// A project lists only its first 100 issues, so the issues are fetched on
/// their own, from the issue side: every open issue, and every issue updated
/// recently enough to matter. A state change updates the issue, and a state
/// change only matters to a project's status update while that update is
/// younger than `status_update_days`, so a day beyond that is enough.
///
/// `named` are issue identifiers that must be in the snapshot even when they
/// are old and closed (`--issues`). Those that do not exist in this workspace
/// are left out; the audit reports them as unresolved.
pub fn collect(
    client: &Client,
    workspace: &str,
    config: &AuditConfig,
    named: &[String],
    now: DateTime<Utc>,
) -> Result<Snapshot> {
    let since = now - Duration::days(i64::from(config.status_update_days) + 1);
    let filter = filters::audit_issues(since);
    let mut issues = paginate(ISSUE_LIST_PAGE_SIZE, None, |page| {
        let vars = IssueListVars::new(page, Some(filter.clone()));
        let data: IssueList = client.execute(&read::issue_list(vars))?;
        Ok(data.issues)
    })?
    .items;

    for identifier in named {
        if issues
            .iter()
            .any(|i| i.identifier.eq_ignore_ascii_case(identifier.trim()))
        {
            continue;
        }
        if let Some(issue) = fetch_named(client, identifier)? {
            if !issues.iter().any(|i| i.id == issue.id) {
                issues.push(issue);
            }
        }
    }

    let projects = paginate(PROJECTS_PAGE_SIZE, None, |vars| {
        let data: Projects = client.execute(&queries::projects(vars))?;
        Ok(data.projects)
    })?
    .items;

    let templates = if config.validators.contains(&Rule::TemplateSections) {
        let data: Templates = client.execute(&queries::templates())?;
        data.templates
    } else {
        Vec::new()
    };

    Ok(Snapshot {
        workspace: workspace.to_owned(),
        issues,
        projects,
        templates,
    })
}

/// One issue by identifier, or `None` when this workspace has no such issue.
fn fetch_named(client: &Client, identifier: &str) -> Result<Option<Issue>> {
    match client.execute::<_, _, IssueById>(&queries::issue(identifier.trim())) {
        Ok(data) => Ok(Some(data.issue)),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Linear answers an unknown issue with an "Entity not found" API error.
fn is_not_found(e: &CliError) -> bool {
    e.message.to_ascii_lowercase().contains("not found")
}
