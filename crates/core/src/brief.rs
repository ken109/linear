//! The brief: where each unfinished project stands, as markdown or JSON.
//!
//! This is what a session start wants to read first: for every project that
//! is in progress (or that has a status update), its health, the start of its
//! latest status update, how far its milestones are, and a mark when there is
//! no update or the update is old. It is built from the same `Project` rows
//! as `project list`; the staleness threshold is the audit's
//! `status_update_days`.
//!
//! Pure: `now` and the UTC offset used to show dates and count days come from the caller.

use crate::types::{
    Milestone, Project, ProjectMilestoneStatus, ProjectStatusType, ProjectUpdateHealthType,
};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

/// How many lines of a status update are shown. A status update has three
/// parts (where it stands, the next step, what it waits for).
pub const PREVIEW_LINES: usize = 3;
/// Longer lines are cut to this many characters.
pub const PREVIEW_WIDTH: usize = 120;

/// What the brief is built with.
#[derive(Debug, Clone, Copy)]
pub struct BriefOptions {
    /// A status update this many days old (or more) is stale. The audit's
    /// `status_update_days`, 14 by default.
    pub stale_days: u32,
    /// The offset dates are shown and days are counted in (the reader's local time).
    pub offset: FixedOffset,
}

/// The brief of one workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Brief {
    pub workspace: String,
    pub stale_days: u32,
    /// Newest status update first; projects without one come last.
    pub projects: Vec<BriefProject>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefProject {
    pub slug_id: String,
    pub name: String,
    pub url: String,
    /// The project's status as Linear names it (`In Progress`).
    pub status: String,
    /// The first initiative the project belongs to.
    pub initiative: Option<String>,
    /// The health of the latest status update (`onTrack`, `atRisk`, `offTrack`).
    pub health: Option<String>,
    /// `None`: the project has no status update.
    pub update: Option<BriefUpdate>,
    /// `None`: the project has no milestones.
    pub milestones: Option<MilestoneSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefUpdate {
    pub url: String,
    pub created_at: DateTime<Utc>,
    /// Calendar days since it was written, in the offset of [`BriefOptions`].
    pub age_days: i64,
    /// `age_days` has reached the threshold.
    pub stale: bool,
    pub author: String,
    /// The first non-empty lines, list markers removed (see [`PREVIEW_LINES`]).
    pub preview: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneSummary {
    pub total: usize,
    pub done: usize,
    /// The first milestone, in screen order, that is not done.
    pub next: Option<NextMilestone>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextMilestone {
    pub name: String,
    pub target_date: Option<NaiveDate>,
    /// Linear's percentage, 0 to 100.
    pub progress: f64,
    pub overdue: bool,
}

/// Whether a project belongs in the brief: it is not finished, and it is in
/// progress or has a status update. An in-progress project without an update
/// is shown on purpose (so a forgotten update is noticed); one that is not in
/// progress (backlog, planned, paused) is shown only once somebody wrote an
/// update, since it is evidently being worked on.
pub fn should_show(p: &Project) -> bool {
    if matches!(
        p.status.type_,
        ProjectStatusType::Completed | ProjectStatusType::Canceled
    ) {
        return false;
    }
    p.last_update.is_some() || p.status.type_ == ProjectStatusType::Started
}

/// Build the brief from the projects of a workspace (in any order).
pub fn build(
    workspace: &str,
    projects: &[Project],
    now: DateTime<Utc>,
    options: &BriefOptions,
) -> Brief {
    let mut rows: Vec<&Project> = projects.iter().filter(|p| should_show(p)).collect();
    // Newest first; no update last. Stable, so equal rows keep Linear's order.
    rows.sort_by_key(|p| std::cmp::Reverse(p.last_update.as_ref().map(|u| u.created_at)));
    Brief {
        workspace: workspace.to_owned(),
        stale_days: options.stale_days,
        projects: rows.into_iter().map(|p| entry(p, now, options)).collect(),
    }
}

fn entry(p: &Project, now: DateTime<Utc>, options: &BriefOptions) -> BriefProject {
    let update = p.last_update.as_ref().map(|u| {
        // Calendar days in the reader's time zone, so that an update written at 23:00
        // is "1 day ago" the next morning and the age agrees with the date shown.
        let day = |t: DateTime<Utc>| t.with_timezone(&options.offset).date_naive();
        let age_days = (day(now) - day(u.created_at)).num_days();
        BriefUpdate {
            url: u.url.clone(),
            created_at: u.created_at,
            age_days,
            stale: age_days >= i64::from(options.stale_days),
            author: u.user.name.clone(),
            preview: preview(&u.body),
        }
    });
    BriefProject {
        slug_id: p.slug_id.clone(),
        name: p.name.clone(),
        url: p.url.clone(),
        status: p.status.name.clone(),
        initiative: p.initiatives.first().map(|i| i.name.clone()),
        health: p.last_update.as_ref().map(|u| health_word(&u.health)),
        update,
        milestones: milestone_summary(&p.project_milestones),
    }
}

/// The first lines of a status update body: blank lines dropped, a leading
/// list marker (`-`, `*`, `+`, `1.`) removed so `- Stage: ...` reads as text,
/// long lines cut.
pub fn preview(body: &str) -> Vec<String> {
    body.lines()
        .map(|l| strip_marker(l.trim()).trim())
        .filter(|l| !l.is_empty())
        .take(PREVIEW_LINES)
        .map(|l| {
            if l.chars().count() > PREVIEW_WIDTH {
                let head: String = l.chars().take(PREVIEW_WIDTH).collect();
                format!("{head}…")
            } else {
                l.to_owned()
            }
        })
        .collect()
}

/// `- text`, `* text`, `+ text` and `12. text` lose their marker; `-text` and
/// `2026-10-06` do not (a marker is followed by whitespace).
fn strip_marker(line: &str) -> &str {
    let after = |marker_len: usize| {
        let rest = &line[marker_len..];
        rest.starts_with(char::is_whitespace)
            .then(|| rest.trim_start())
    };
    if line.starts_with(['-', '*', '+']) {
        return after(1).unwrap_or(line);
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with('.') {
        return after(digits + 1).unwrap_or(line);
    }
    line
}

fn milestone_summary(milestones: &[Milestone]) -> Option<MilestoneSummary> {
    if milestones.is_empty() {
        return None;
    }
    let mut sorted: Vec<&Milestone> = milestones.iter().collect();
    sorted.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
    let is_done = |m: &Milestone| m.status == ProjectMilestoneStatus::Done;
    let done = sorted.iter().filter(|m| is_done(m)).count();
    let next = sorted.iter().find(|m| !is_done(m)).map(|m| NextMilestone {
        name: m.name.clone(),
        target_date: m.target_date,
        progress: m.progress,
        overdue: m.status == ProjectMilestoneStatus::Overdue,
    });
    Some(MilestoneSummary {
        total: sorted.len(),
        done,
        next,
    })
}

/// Linear's own spelling, as in `--json`.
fn health_word(h: &ProjectUpdateHealthType) -> String {
    match h {
        ProjectUpdateHealthType::OnTrack => "onTrack".to_owned(),
        ProjectUpdateHealthType::AtRisk => "atRisk".to_owned(),
        ProjectUpdateHealthType::OffTrack => "offTrack".to_owned(),
        ProjectUpdateHealthType::Other(s) => s.clone(),
    }
}

/// The health as it reads in a sentence.
fn health_label(word: &str) -> &str {
    match word {
        "onTrack" => "on track",
        "atRisk" => "at risk",
        "offTrack" => "off track",
        other => other,
    }
}

fn ago(days: i64) -> String {
    match days {
        ..=0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        n => format!("{n} days ago"),
    }
}

/// The brief as markdown, for a session start. Empty when there is nothing to
/// show, so the caller can print nothing at all.
pub fn render_markdown(brief: &Brief, offset: FixedOffset) -> String {
    if brief.projects.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        format!(
            "## Linear project status ({}: projects in progress)",
            brief.workspace
        ),
        String::new(),
    ];
    for p in &brief.projects {
        lines.push(format!(
            "- **{}**{} `{}`",
            p.name,
            p.initiative
                .as_ref()
                .map_or_else(String::new, |i| format!(" ({i})")),
            p.slug_id
        ));
        match &p.update {
            None => lines.push("  - No status update".to_owned()),
            Some(u) => {
                let date = u.created_at.with_timezone(&offset).date_naive();
                let when = if u.stale {
                    format!("{date} ({}, **stale**)", ago(u.age_days))
                } else {
                    format!("{date} ({})", ago(u.age_days))
                };
                let health = p.health.as_deref().map_or("", health_label);
                lines.push(format!("  - {when} {health} · {}", u.author));
                lines.extend(u.preview.iter().map(|l| format!("    > {l}")));
            }
        }
        if let Some(m) = &p.milestones {
            lines.push(format!("  - {}", milestone_line(m)));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Details: `linear project view <slug>`. **A status update {} days old or more is stale: \
         check it before relying on it.**",
        brief.stale_days
    ));
    lines.join("\n")
}

fn milestone_line(m: &MilestoneSummary) -> String {
    let head = format!("Milestones {}/{} done", m.done, m.total);
    let Some(next) = &m.next else {
        return head;
    };
    let mut detail = Vec::new();
    if let Some(d) = next.target_date {
        detail.push(d.to_string());
    }
    detail.push(format!("{:.0}%", next.progress));
    if next.overdue {
        detail.push("overdue".to_owned());
    }
    format!("{head} · next: {} ({})", next.name, detail.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_drops_blank_lines_and_list_markers() {
        let body = "\n- Stage: build\n\n* Next: ship\n1. Waiting: review\n- fourth\n";
        assert_eq!(
            preview(body),
            ["Stage: build", "Next: ship", "Waiting: review"]
        );
    }

    #[test]
    fn only_a_marker_followed_by_a_space_is_removed() {
        assert_eq!(preview("-5 degrees"), ["-5 degrees"]);
        assert_eq!(preview("2026-10-06 shipped"), ["2026-10-06 shipped"]);
        assert_eq!(preview("3.5 percent"), ["3.5 percent"]);
        assert_eq!(preview("-"), ["-"]);
        assert_eq!(preview("  - indented"), ["indented"]);
    }

    #[test]
    fn a_long_line_is_cut_at_the_width() {
        let long = "x".repeat(PREVIEW_WIDTH + 30);
        let p = preview(&long);
        assert_eq!(p[0].chars().count(), PREVIEW_WIDTH + 1);
        assert!(p[0].ends_with('…'));
        let exact = "y".repeat(PREVIEW_WIDTH);
        assert_eq!(preview(&exact), [exact]);
    }

    #[test]
    fn a_non_ascii_line_is_cut_on_a_character_boundary() {
        let long = "あ".repeat(PREVIEW_WIDTH + 5);
        let p = preview(&long);
        assert_eq!(p[0].chars().count(), PREVIEW_WIDTH + 1);
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(0), "today");
        assert_eq!(ago(-1), "today");
        assert_eq!(ago(1), "1 day ago");
        assert_eq!(ago(20), "20 days ago");
    }
}
