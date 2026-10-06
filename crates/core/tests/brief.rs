//! The brief of unfinished projects, against a fixture of six projects:
//!
//! - in progress with a recent update, milestones (3 lines of 4 shown, a list marker removed)
//! - in progress with no update ("no update" mark, last)
//! - paused with a stale update by someone else (blank lines, numbered lines, a long line)
//! - backlog with no update (not shown)
//! - planned with a fresh update (shown: somebody wrote about it)
//! - completed (not shown)
//!
//! The markdown and the JSON are compared with golden files in `tests/fixtures`;
//! run with `UPDATE_GOLDEN=1` to rewrite them after a deliberate change.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use linear_core::brief::{build, render_markdown, Brief, BriefOptions};
use linear_core::read::ProjectList;
use linear_core::wire::{parse_response, ResponseMeta};

fn read(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

fn brief_with(stale_days: u32) -> Brief {
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    let list: ProjectList =
        parse_response(&meta, &read("brief_projects.json"), now()).expect("the fixture parses");
    build(
        "example",
        &list.projects.nodes,
        now(),
        &BriefOptions {
            stale_days,
            offset: jst(),
        },
    )
}

fn golden(name: &str, actual: &str) {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    assert_eq!(
        actual,
        read(name),
        "{name} differs; run with UPDATE_GOLDEN=1 if the change is deliberate"
    );
}

#[test]
fn the_markdown_matches_the_golden_file() {
    let md = render_markdown(&brief_with(14), jst());
    golden("brief.golden.md", &format!("{md}\n"));
}

#[test]
fn the_json_matches_the_golden_file() {
    let json = serde_json::to_string_pretty(&brief_with(14)).unwrap();
    golden("brief.golden.json", &format!("{json}\n"));
}

#[test]
fn which_projects_are_shown_and_in_what_order() {
    let brief = brief_with(14);
    let slugs: Vec<&str> = brief.projects.iter().map(|p| p.slug_id.as_str()).collect();
    // Newest update first, no update last; backlog and completed are left out.
    assert_eq!(
        slugs,
        [
            "eeeeeeeeeeee",
            "aaaaaaaaaaaa",
            "cccccccccccc",
            "bbbbbbbbbbbb"
        ]
    );
}

#[test]
fn an_update_is_stale_from_the_threshold_on() {
    let age = |brief: &Brief, slug: &str| {
        let p = brief.projects.iter().find(|p| p.slug_id == slug).unwrap();
        let u = p.update.as_ref().unwrap();
        (u.age_days, u.stale)
    };
    let brief = brief_with(14);
    assert_eq!(age(&brief, "aaaaaaaaaaaa"), (13, false));
    assert_eq!(age(&brief, "cccccccccccc"), (80, true));
    // The threshold is "this many days or more".
    assert_eq!(age(&brief_with(13), "aaaaaaaaaaaa"), (13, true));
    assert_eq!(age(&brief_with(100_000), "cccccccccccc"), (80, false));
}

#[test]
fn a_project_without_an_update_carries_no_update() {
    let brief = brief_with(14);
    let p = brief
        .projects
        .iter()
        .find(|p| p.slug_id == "bbbbbbbbbbbb")
        .unwrap();
    assert!(p.update.is_none() && p.health.is_none() && p.milestones.is_none());
    let md = render_markdown(&brief, jst());
    assert!(md.contains("  - No status update"), "{md}");
}

#[test]
fn nothing_to_show_renders_as_nothing() {
    let none = Brief {
        workspace: "example".to_owned(),
        stale_days: 14,
        projects: Vec::new(),
    };
    assert_eq!(render_markdown(&none, jst()), "");
}

#[test]
fn dates_are_shown_in_the_given_offset() {
    // eeee was written 2026-10-07 00:30 UTC: still the 7th in UTC, but 09:30 in JST.
    let brief = brief_with(14);
    assert!(render_markdown(&brief, jst()).contains("2026-10-07 (13 days ago)"));
    let west = FixedOffset::west_opt(5 * 3600).unwrap();
    assert!(render_markdown(&brief, west).contains("2026-10-06 (13 days ago)"));
}
