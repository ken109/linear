//! `linear cycle` against a mock Linear.
//!
//! The cycles come from `crates/core/tests/fixtures/cycles.json` (#41 starts
//! 2026-10-05T15:00Z, #42 a week later), so a meeting on 2026-10-05 goes into
//! #41 and one on 2026-10-30 has no cycle.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const NO_CYCLE_DAY: &str = "2026-10-30";

// ------------------------------------------------------------------ linear cycle

fn cycle_routes() -> Vec<(&'static str, Vec<Reply>)> {
    vec![("CycleList", vec![cycles()])]
}

#[test]
fn cycle_prints_the_cycle_that_contains_the_day_after_the_meeting() {
    let sb = workspace();
    let mock = Routed::start(cycle_routes());
    let o = run(&sb, &mock, &["cycle", MEETING, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["team"], "EX");
    assert_eq!(v["heldOn"], MEETING);
    assert_eq!(v["id"], CYCLE_41);
    assert_eq!(v["number"], 41);
    assert_eq!(v["startsAt"], "2026-10-05T15:00:00Z");
    assert_eq!(v["endsAt"], "2026-10-12T15:00:00Z");

    // The team is the workspace's default, and only that team's cycles are asked for.
    assert_eq!(
        mock.of("CycleList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
    // Read-only: a lookup is the only request.
    assert_eq!(mock.ops(), ["CycleList"]);
}

#[test]
fn cycle_text_and_quiet_output() {
    let sb = workspace();
    let mock = Routed::start(cycle_routes());
    let o = run(&sb, &mock, &["cycle", "2026-10-12"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Cycle #42  (EX)",
        "2026-10-12T15:00:00Z",
        "2026-10-19T15:00:00Z",
        "2026-10-13, the day after 2026-10-12",
        CYCLE_42,
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }

    let o = run(&sb, &mock, &["cycle", "2026-10-12", "--quiet"]);
    assert_eq!(stdout(&o).trim(), CYCLE_42);
}

#[test]
fn cycle_takes_the_team_from_the_flag() {
    let sb = workspace();
    let mock = Routed::start(cycle_routes());
    let o = run(&sb, &mock, &["cycle", MEETING, "--team", "lt3", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("CycleList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "lt3"}}})
    );
}

#[test]
fn a_day_without_a_cycle_fails_and_lists_the_cycles_that_exist() {
    let sb = workspace();
    let mock = Routed::start(cycle_routes());
    let o = run(&sb, &mock, &["cycle", NO_CYCLE_DAY]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "nothing is printed on failure");
    let err = stderr(&o);
    for want in [
        "no cycle of team EX contains 2026-10-31",
        "the meeting on 2026-10-30",
        "#41 2026-10-05T15:00:00Z .. 2026-10-12T15:00:00Z",
        "#42 2026-10-12T15:00:00Z .. 2026-10-19T15:00:00Z",
    ] {
        assert!(err.contains(want), "missing {want:?} in:\n{err}");
    }

    // With --json the error is the usual object.
    let o = run(&sb, &mock, &["cycle", NO_CYCLE_DAY, "--json"]);
    assert_eq!(code(&o), 2);
    let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(e["error"]["code"], "usage");
}

#[test]
fn a_team_without_cycles_says_none_exist() {
    let sb = workspace();
    let empty =
        r#"{"data":{"cycles":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Routed::start(vec![("CycleList", vec![ok(empty)])]);
    let o = run(&sb, &mock, &["cycle", MEETING]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("Cycles known: none"), "{}", stderr(&o));
}

#[test]
fn cycle_reads_every_page_of_cycles() {
    let sb = workspace();
    let page = |has_next: bool, cursor: &str, which: usize| {
        let mut v: Value = serde_json::from_str(&fixture("cycles")).unwrap();
        let nodes = v["data"]["cycles"]["nodes"].as_array().unwrap().clone();
        v["data"]["cycles"]["nodes"] = json!([nodes[which].clone()]);
        v["data"]["cycles"]["pageInfo"] = json!({"hasNextPage": has_next, "endCursor": cursor});
        ok(&v.to_string())
    };
    let mock = Routed::start(vec![(
        "CycleList",
        vec![page(true, "c1", 0), page(false, "c2", 1)],
    )]);
    // The second page holds #42, which only a follow-up request can find.
    let o = run(&sb, &mock, &["cycle", "2026-10-12", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), CYCLE_42);
    let asked = mock.of("CycleList");
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1]["after"], "c1");
}

#[test]
fn cycle_dates_are_checked_strictly_before_any_request() {
    let sb = workspace();
    for bad in ["2026-02-30", "2026-13-01", "nope", "10-05"] {
        let mock = Routed::start(cycle_routes());
        let o = run(&sb, &mock, &["cycle", bad]);
        assert_eq!(code(&o), 2, "{bad}: {}", stderr(&o));
        assert!(mock.ops().is_empty(), "{bad}: a request was sent");
    }
}

#[test]
fn cycle_needs_a_team_from_the_flag_or_the_workspace() {
    let sb = Sandbox::new();
    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "example"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let mock = Routed::start(cycle_routes());
    let o = run(&sb, &mock, &["cycle", MEETING]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("no team"), "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}
