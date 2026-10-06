//! `linear cycle <YYYY-MM-DD>`: the cycle a meeting's commitments go into.
//!
//! Read-only. The rule (the cycle that contains the day after the meeting)
//! lives in `linear_core::cycle`; `issue create --held-on` uses the same lookup.

use super::format::fields;
use super::listing::paginate;
use super::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use chrono::NaiveDate;
use clap::Args;
use linear_core::cycle::{cycle_for, day_after, label, stamp};
use linear_core::filters::cycles_of_team;
use linear_core::read::{self, CycleList, CycleListVars, CYCLE_LIST_PAGE_SIZE};
use linear_core::types::Cycle;
use serde::Serialize;

#[derive(Debug, Args)]
pub struct CycleArgs {
    /// The day of the meeting (YYYY-MM-DD). The cycle that contains the day after it is
    /// printed; when there is none the command fails (exit 2) and lists the cycles that exist
    #[arg(value_name = "DATE")]
    pub held_on: NaiveDate,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
}

/// The cycle of team `team` that contains the day after `held_on`, from Linear.
///
/// Fails with a usage error that lists the cycles there are when none does.
pub(super) fn find(client: &Client, team: &str, held_on: NaiveDate) -> Result<Cycle> {
    let filter = Some(cycles_of_team(team));
    let all = paginate(CYCLE_LIST_PAGE_SIZE, None, |page| {
        let vars = CycleListVars::new(page, filter.clone());
        let data: CycleList = client.execute(&read::cycles(vars))?;
        Ok(data.cycles)
    })?
    .items;
    Ok(cycle_for(held_on, team, &all)?.clone())
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
    let session = ctx.session()?;
    let team = args
        .team
        .as_deref()
        .or(session.config.default_team.as_deref())
        .ok_or_else(|| {
            CliError::usage(
                "no team: pass --team <KEY> or set default_team for this workspace in workspaces.toml",
            )
        })?;
    let cycle = find(&session.client, team, args.held_on)?;
    let value = CycleOut {
        workspace: &session.workspace,
        team,
        held_on: args.held_on,
        cycle: &cycle,
    };
    ctx.out.emit(
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
                        format!(
                            "{}, the day after {}",
                            day_after(args.held_on),
                            args.held_on
                        )
                    ),
                    ("Id", cycle.id.inner().to_owned()),
                ])
            )
        },
        || cycle.id.inner().to_owned(),
    );
    Ok(())
}
