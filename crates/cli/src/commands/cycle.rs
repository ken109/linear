//! `linear cycle <YYYY-MM-DD>`: the cycle a meeting's commitments go into, and
//! `linear cycle list|view`: the team's cycles and the issues in one.
//!
//! Read-only. The rule (the cycle that contains the day after the meeting)
//! lives in `linear_core::cycle`; `issue create --held-on` uses the same lookup.
//! `cycle <DATE>` is the command's own positional argument and `list` / `view`
//! are its subcommands (clap tells them apart: a date is never a subcommand name).

use super::format::{date_time, fields, opt_text};
use super::listing::{paginate, warn_truncated, ListArgs, Session};
use super::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::table;
use chrono::NaiveDate;
use clap::{Args, Subcommand, ValueEnum};
use linear_core::cycle::{cycle_for, cycle_numbered as find_in, day_after, label, stamp};
use linear_core::cycle_read::{
    self, CycleInfo, CycleInfoList, CycleInfoListVars, CycleIssuesQuery, CycleIssuesVars,
    CYCLE_INFO_PAGE_SIZE, CYCLE_ISSUES_PAGE_SIZE,
};
use linear_core::filters::{cycle_numbered, cycles_in_state, cycles_of_team, CycleState};
use linear_core::read::{self, CycleList, CycleListVars, IssueBrief, CYCLE_LIST_PAGE_SIZE};
use linear_core::types::Cycle;
use linear_core::InWorkspace;
use serde::Serialize;

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct CycleArgs {
    /// The day of the meeting (YYYY-MM-DD). The cycle that contains the day after it is
    /// printed; when there is none the command fails (exit 2) and lists the cycles that exist
    #[arg(value_name = "DATE", required = true)]
    pub held_on: Option<NaiveDate>,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    #[command(subcommand)]
    pub command: Option<CycleCommand>,
}

#[derive(Debug, Subcommand)]
pub enum CycleCommand {
    /// Show how to use these commands, briefly, for an AI agent
    Usage,
    /// List a team's cycles, newest first
    List(ListCmd),
    /// Show one cycle by its number, with the issues in it
    View(ViewCmd),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StateArg {
    /// The cycle that is running now
    Active,
    /// Cycles that have not started
    Upcoming,
    /// Cycles that have ended
    Past,
}

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Only cycles in this state
    #[arg(long, value_enum)]
    pub state: Option<StateArg>,
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// The cycle's number (the `#41` of a listing; numbers are per team)
    #[arg(value_name = "NUMBER")]
    pub number: u32,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    #[command(flatten)]
    pub page: ListArgs,
}

/// Every cycle of team `team`.
fn of_team(client: &Client, team: &str) -> Result<Vec<Cycle>> {
    let filter = Some(cycles_of_team(team));
    Ok(paginate(CYCLE_LIST_PAGE_SIZE, None, |page| {
        let vars = CycleListVars::new(page, filter.clone());
        let data: CycleList = client.execute(&read::cycles(vars))?;
        Ok(data.cycles)
    })?
    .items)
}

/// The cycle of team `team` that contains the day after `held_on`, from Linear.
///
/// Fails with a usage error that lists the cycles there are when none does.
pub(super) fn find(client: &Client, team: &str, held_on: NaiveDate) -> Result<Cycle> {
    let all = of_team(client, team)?;
    Ok(cycle_for(held_on, team, &all)?.clone())
}

/// The cycle of team `team` that has this number (`--cycle 42`), from Linear.
///
/// Fails with a usage error that lists the cycles there are when none does.
pub(super) fn find_with_number(client: &Client, team: &str, number: u32) -> Result<Cycle> {
    let all = of_team(client, team)?;
    Ok(find_in(number, team, &all)?.clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CycleOut<'a> {
    workspace: &'a str,
    team: &'a str,
    held_on: NaiveDate,
    #[serde(flatten)]
    cycle: &'a Cycle,
}

pub fn run(ctx: &Ctx, args: &CycleArgs) -> Result<()> {
    match (&args.command, args.held_on) {
        (Some(CycleCommand::Usage), _) => unreachable!("handled before the context is built"),
        (Some(CycleCommand::List(cmd)), _) => list(ctx, cmd),
        (Some(CycleCommand::View(cmd)), _) => view(ctx, cmd),
        (None, Some(held_on)) => by_date(ctx, args.team.as_deref(), held_on),
        (None, None) => Err(CliError::usage(
            "pass the day of the meeting (`linear cycle 2026-10-05`) or a subcommand (list, view)",
        )),
    }
}

/// The team named by the flag, else the workspace's `default_team`.
fn team_of<'a>(session: &'a Session, flag: Option<&'a str>) -> Result<&'a str> {
    flag.or(session.config.default_team.as_deref())
        .ok_or_else(|| {
            CliError::usage(
            "no team: pass --team <KEY> or set default_team for this workspace in workspaces.toml",
        )
        })
}

fn by_date(ctx: &Ctx, team: Option<&str>, held_on: NaiveDate) -> Result<()> {
    let session = ctx.session()?;
    let team = team_of(&session, team)?;
    let cycle = find(&session.client, team, held_on)?;
    let value = CycleOut {
        workspace: &session.workspace,
        team,
        held_on,
        cycle: &cycle,
    };
    ctx.out.emit_selectable(
        &value,
        || {
            format!(
                "Cycle {}  ({team})\n\n{}",
                label(&cycle),
                fields(&[
                    ("Starts", stamp(cycle.starts_at)),
                    ("Ends", stamp(cycle.ends_at)),
                    (
                        "Holds",
                        format!("{}, the day after {held_on}", day_after(held_on))
                    ),
                    ("Id", cycle.id.inner().to_owned()),
                ])
            )
        },
        || cycle.id.inner().to_owned(),
    )?;
    Ok(())
}

// ---------------------------------------------------------------- list

fn state_of(arg: StateArg) -> CycleState {
    match arg {
        StateArg::Active => CycleState::Active,
        StateArg::Upcoming => CycleState::Upcoming,
        StateArg::Past => CycleState::Past,
    }
}

fn percent_of(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

fn list(ctx: &Ctx, cmd: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let team = team_of(&session, cmd.team.as_deref())?;
    let filter = Some(cycles_in_state(team, cmd.state.map(state_of)));
    // Linear cannot sort cycles, and the ones that matter (the running and the next)
    // are far from either end of its order, so every page is fetched and `--limit`
    // keeps the newest of them.
    let mut all = paginate(CYCLE_INFO_PAGE_SIZE, None, |page| {
        let vars = CycleInfoListVars::new(page, filter.clone());
        let data: CycleInfoList = session.client.execute(&cycle_read::cycle_infos(vars))?;
        Ok(data.cycles)
    })?
    .items;
    all.sort_by_key(|c| std::cmp::Reverse(c.starts_at));
    let truncated = cmd.page.limit().is_some_and(|l| all.len() > l);
    if let Some(l) = cmd.page.limit() {
        all.truncate(l);
    }
    if truncated {
        warn_truncated(all.len());
    }

    let tagged = InWorkspace::tag_all(&session.workspace, all.clone());
    ctx.out.emit_selectable(
        &tagged,
        || {
            if all.is_empty() {
                return format!("No cycles found for team {team}.");
            }
            let body: Vec<Vec<String>> = all
                .iter()
                .map(|c| {
                    vec![
                        format!("#{}", c.number_whole()),
                        opt_text(c.name.as_deref()),
                        c.status().as_str().to_owned(),
                        date_time(&c.starts_at),
                        date_time(&c.ends_at),
                        percent_of(c.progress),
                    ]
                })
                .collect();
            table(
                &["NUMBER", "NAME", "STATE", "STARTS", "ENDS", "PROGRESS"],
                &body,
            )
        },
        || {
            all.iter()
                .map(|c| c.number_whole().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------- view

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CycleViewOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    cycle: &'a CycleInfo,
    issues: &'a [IssueBrief],
}

/// The cycle of `team` with this number, from Linear. A usage error when there is none.
fn find_numbered(client: &Client, team: &str, number: u32) -> Result<CycleInfo> {
    let filter = Some(cycle_numbered(team, number));
    let vars = CycleInfoListVars::new(
        linear_core::types::PageVars {
            first: 1,
            after: None,
        },
        filter,
    );
    let data: CycleInfoList = client.execute(&cycle_read::cycle_infos(vars))?;
    data.cycles
        .nodes
        .into_iter()
        .next()
        .ok_or_else(|| CliError::usage(format!("team {team} has no cycle #{number}")))
}

fn view(ctx: &Ctx, cmd: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let team = team_of(&session, cmd.team.as_deref())?;
    let cycle = find_numbered(&session.client, team, cmd.number)?;
    let listing = paginate(CYCLE_ISSUES_PAGE_SIZE, cmd.page.limit(), |page| {
        let vars = CycleIssuesVars::new(cycle.id.inner(), page);
        let data: CycleIssuesQuery = session.client.execute(&cycle_read::cycle_issues(vars))?;
        Ok(data.cycle.issues)
    })?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    let issues = &listing.items;

    let value = CycleViewOut {
        workspace: &session.workspace,
        cycle: &cycle,
        issues,
    };
    ctx.out.emit_selectable(
        &value,
        || {
            let mut text = format!(
                "Cycle {}  ({team})\n\n{}",
                cycle.label(),
                fields(&[
                    ("State", cycle.status().as_str().to_owned()),
                    ("Starts", stamp(cycle.starts_at)),
                    ("Ends", stamp(cycle.ends_at)),
                    ("Progress", percent_of(cycle.progress)),
                    ("Id", cycle.id.inner().to_owned()),
                ])
            );
            if let Some(desc) = cycle
                .description
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                text.push_str(&format!("\n\n{}", desc.trim_end()));
            }
            text.push_str(&format!("\n\nIssues ({})", issues.len()));
            if !issues.is_empty() {
                let body: Vec<Vec<String>> = issues
                    .iter()
                    .map(|i| {
                        vec![
                            i.identifier.clone(),
                            i.state.name.clone(),
                            opt_text(i.assignee.as_ref().map(|u| u.name.as_str())),
                            i.title.clone(),
                        ]
                    })
                    .collect();
                text.push('\n');
                text.push_str(&table(&["ID", "STATE", "ASSIGNEE", "TITLE"], &body));
            }
            text
        },
        || cycle.number_whole().to_string(),
    )?;
    Ok(())
}
