//! Text helpers for the human-readable output of the read commands.

use chrono::{DateTime, NaiveDate, Utc};
use linear_core::types::{
    InitiativeStatus, ProjectMilestoneStatus, ProjectStatusType, ProjectUpdateHealthType, User,
};

pub fn date_time(t: &DateTime<Utc>) -> String {
    t.format("%Y-%m-%d").to_string()
}

pub fn opt_date(d: &Option<NaiveDate>) -> String {
    d.map_or_else(|| "-".to_owned(), |d| d.to_string())
}

pub fn opt_text(s: Option<&str>) -> String {
    match s {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => "-".to_owned(),
    }
}

pub fn person(u: &Option<User>) -> String {
    u.as_ref()
        .map_or_else(|| "-".to_owned(), |u| u.name.clone())
}

/// Indent every line (blank lines stay blank).
pub fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.trim_end()
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A block of `Label:  value` lines with the values aligned.
pub fn fields(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
    rows.iter()
        .map(|(k, v)| format!("{:<width$} {}", format!("{k}:"), v, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first non-empty line, shortened to `max` characters.
pub fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > max {
        let head: String = line.chars().take(max).collect();
        format!("{head}...")
    } else {
        line.to_owned()
    }
}

/// Linear's own spelling, as it appears in `--json`.
pub fn health(h: &ProjectUpdateHealthType) -> String {
    match h {
        ProjectUpdateHealthType::OnTrack => "onTrack".into(),
        ProjectUpdateHealthType::AtRisk => "atRisk".into(),
        ProjectUpdateHealthType::OffTrack => "offTrack".into(),
        ProjectUpdateHealthType::Other(s) => s.clone(),
    }
}

pub fn project_status_type(t: &ProjectStatusType) -> String {
    match t {
        ProjectStatusType::Backlog => "backlog".into(),
        ProjectStatusType::Planned => "planned".into(),
        ProjectStatusType::Started => "started".into(),
        ProjectStatusType::Paused => "paused".into(),
        ProjectStatusType::Completed => "completed".into(),
        ProjectStatusType::Canceled => "canceled".into(),
        ProjectStatusType::Other(s) => s.clone(),
    }
}

pub fn milestone_status(s: &ProjectMilestoneStatus) -> String {
    match s {
        ProjectMilestoneStatus::Done => "done".into(),
        ProjectMilestoneStatus::Next => "next".into(),
        ProjectMilestoneStatus::Overdue => "overdue".into(),
        ProjectMilestoneStatus::Unstarted => "unstarted".into(),
        ProjectMilestoneStatus::Other(s) => s.clone(),
    }
}

/// Linear reports milestone progress as a percentage (0 to 100).
pub fn percent(p: f64) -> String {
    format!("{p:.0}%")
}

pub fn initiative_status(s: &InitiativeStatus) -> String {
    match s {
        InitiativeStatus::Active => "Active".into(),
        InitiativeStatus::Canceled => "Canceled".into(),
        InitiativeStatus::Completed => "Completed".into(),
        InitiativeStatus::Planned => "Planned".into(),
        InitiativeStatus::Proposed => "Proposed".into(),
        InitiativeStatus::Other(s) => s.clone(),
    }
}
