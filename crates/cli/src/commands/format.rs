//! Text helpers for the human-readable output of the read commands.

use chrono::{DateTime, NaiveDate, Utc};
use linear_core::types::User;

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
