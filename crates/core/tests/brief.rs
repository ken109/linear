//! The brief of unfinished projects, against a fixture of six projects:
//!
//! - in progress with an update exactly on the stale threshold, milestones (3 lines of 4
//!   shown, a list marker removed)
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
    brief_in(stale_days, jst())
}

fn brief_in(stale_days: u32, offset: FixedOffset) -> Brief {
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
        &BriefOptions { stale_days, offset },
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
    // eeee: 2026-10-07 in JST; aaaa: 2026-10-06 in JST (22:34); now is 2026-10-20 (21:00).
    assert_eq!(age(&brief, "eeeeeeeeeeee"), (13, false));
    assert_eq!(age(&brief, "aaaaaaaaaaaa"), (14, true));
    assert_eq!(age(&brief, "cccccccccccc"), (80, true));
    // The threshold is "this many days or more".
    assert_eq!(age(&brief_with(13), "eeeeeeeeeeee"), (13, true));
    assert_eq!(age(&brief_with(15), "aaaaaaaaaaaa"), (14, false));
    assert_eq!(age(&brief_with(100_000), "cccccccccccc"), (80, false));
}

#[test]
fn days_are_calendar_days_of_the_offset_not_elapsed_hours() {
    // aaaa was written 2026-10-06 13:34 UTC, 13 days and 22 hours before now (the 20th, 12:00
    // UTC). Elapsed time says 13 days; the calendar says 14, unless the offset puts the update
    // just after midnight and now just before it (UTC+11: the 7th 00:34, and the 20th 23:00).
    let aaaa = |hours: i32| {
        let brief = brief_in(14, FixedOffset::east_opt(hours * 3600).unwrap());
        let p = brief.projects.iter().find(|p| p.slug_id == "aaaaaaaaaaaa");
        p.unwrap().update.as_ref().unwrap().age_days
    };
    assert_eq!(aaaa(9), 14);
    assert_eq!(aaaa(0), 14);
    assert_eq!(aaaa(11), 13);
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
    // eeee was written 2026-10-07 00:30 UTC: the 7th in UTC and in JST (09:30), the 6th at UTC-5.
    let line = |offset| {
        let md = render_markdown(&brief_in(14, offset), offset);
        md.lines()
            .find(|l| l.ends_with("off track · Alice Example"))
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        line(jst()),
        "  - 2026-10-07 (13 days ago) off track · Alice Example"
    );
    let west = FixedOffset::west_opt(5 * 3600).unwrap();
    assert_eq!(
        line(west),
        "  - 2026-10-06 (14 days ago, **stale**) off track · Alice Example"
    );
}
