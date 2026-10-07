//! Which cycle the commitments of a recurring meeting go into.
//!
//! The rule is one sentence: **the cycle that contains the day after the
//! meeting.** A weekly meeting declares what is done in the week that follows
//! it, and that week's cycle is the one that is running the next day. The rule
//! does not look at weekdays: with cycles that start on Tuesday, a Monday
//! meeting lands in the cycle that starts the next morning, and a meeting that
//! slipped to Tuesday lands in the cycle that started that same day.
//!
//! The day is judged at noon in Japan Standard Time (UTC+9), so a cycle
//! boundary at midnight, give or take a rounding, never decides the answer.
//!
//! A day that no cycle contains is an error, never "no cycle": a missing
//! cycle means the team's cycles are not set up that far ahead, and silently
//! leaving the issue out would put this week's commitments outside the week.

use crate::error::{Error, Result};
use crate::types::Cycle;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};

/// The zone the day after the meeting is judged in (UTC+9, Japan Standard Time).
const JUDGED_IN_SECS: i32 = 9 * 3600;

/// The day the rule looks at: the day after `held_on`.
pub fn day_after(held_on: NaiveDate) -> NaiveDate {
    held_on.succ_opt().unwrap_or(held_on)
}

/// The instant the rule looks at: noon, in Japan Standard Time, on the day after `held_on`.
pub fn instant_for(held_on: NaiveDate) -> DateTime<Utc> {
    let zone = FixedOffset::east_opt(JUDGED_IN_SECS).expect("nine hours is a valid offset");
    let noon = NaiveTime::from_hms_opt(12, 0, 0).expect("noon exists");
    zone.from_local_datetime(&day_after(held_on).and_time(noon))
        .single()
        .expect("a fixed offset has no ambiguous times")
        .with_timezone(&Utc)
}

/// The cycle of `cycles` (the cycles of team `team`) that contains the day
/// after `held_on`.
///
/// The error is a usage error and lists the cycles that do exist, so the
/// fix (set up the team's cycles) is clear from the message alone.
pub fn cycle_for<'a>(held_on: NaiveDate, team: &str, cycles: &'a [Cycle]) -> Result<&'a Cycle> {
    let at = instant_for(held_on);
    // Cycles of one team do not overlap; if they ever did, the earlier start wins.
    cycles
        .iter()
        .filter(|c| c.starts_at <= at && at < c.ends_at)
        .min_by_key(|c| c.starts_at)
        .ok_or_else(|| {
            Error::Usage(format!(
                "no cycle of team {team} contains {} (the day after the meeting on {held_on}); \
                 check the team's cycle settings in Linear. Cycles known: {}",
                day_after(held_on),
                known(cycles)
            ))
        })
}

/// The cycle of `cycles` (the cycles of team `team`) that has this number, as `--cycle 12` asks for.
///
/// A number no cycle has is a usage error that lists the cycles that do exist.
pub fn cycle_numbered<'a>(number: u32, team: &str, cycles: &'a [Cycle]) -> Result<&'a Cycle> {
    cycles
        .iter()
        .find(|c| c.number == f64::from(number))
        .ok_or_else(|| {
            Error::Usage(format!(
                "team {team} has no cycle #{number}. Cycles known: {}",
                known(cycles)
            ))
        })
}

/// A cycle as a person types it, `12` or `#12`: its number.
pub fn parse_number(spec: &str) -> std::result::Result<u32, String> {
    spec.trim().trim_start_matches('#').parse().map_err(|_| {
        format!("{spec:?} is not a cycle number; expected a number such as 42, or none")
    })
}

/// `#12 2026-10-05T15:00:00Z .. 2026-10-12T15:00:00Z`, oldest first; `none` when empty.
fn known(cycles: &[Cycle]) -> String {
    if cycles.is_empty() {
        return "none".to_owned();
    }
    let mut sorted: Vec<&Cycle> = cycles.iter().collect();
    sorted.sort_by_key(|c| c.starts_at);
    sorted
        .iter()
        .map(|c| {
            format!(
                "{} {} .. {}",
                label(c),
                stamp(c.starts_at),
                stamp(c.ends_at)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A timestamp as `2026-10-05T15:00:00Z`.
pub fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `#12`, with the cycle's name when it has one: `#12 (Sprint)`.
pub fn label(c: &Cycle) -> String {
    let number = format!("#{}", c.number);
    match c.name.as_deref().filter(|n| !n.trim().is_empty()) {
        Some(name) => format!("{number} ({name})"),
        None => number,
    }
}
