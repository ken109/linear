//! The cycle a meeting's commitments go into: "the cycle that contains the day
//! after the meeting", judged at noon Japan Standard Time.

use chrono::{NaiveDate, TimeZone, Utc};
use linear_core::cycle::{cycle_for, instant_for, label};
use linear_core::filters::cycles_of_team;
use linear_core::read::{self, CycleList, CycleListVars};
use linear_core::types::{Cycle, PageVars};
use linear_core::wire::{build_request, parse_response, ResponseMeta};
use linear_core::ErrorCode;
use serde_json::{json, Value};

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// Weekly cycles that start on Tuesday 00:00 JST (Monday 15:00 UTC), numbered from 40.
fn weekly(first_monday: &str, count: u32) -> Vec<Cycle> {
    let start = day(first_monday).and_hms_opt(15, 0, 0).unwrap().and_utc();
    (0..count)
        .map(|i| {
            let from = start + chrono::Duration::weeks(i64::from(i));
            let to = from + chrono::Duration::weeks(1);
            serde_json::from_value(json!({
                "id": format!("cycle-{}", 40 + i),
                "number": 40 + i,
                "name": null,
                "startsAt": from.to_rfc3339(),
                "endsAt": to.to_rfc3339(),
            }))
            .unwrap()
        })
        .collect()
}

#[test]
fn a_monday_meeting_goes_into_the_cycle_that_starts_the_next_morning() {
    // Cycles: #40 Tue 09-29 .. Tue 10-06, #41 Tue 10-06 .. Tue 10-13, #42 Tue 10-13 ..
    let cycles = weekly("2026-09-28", 3);
    let hit = cycle_for(day("2026-10-05"), "EX", &cycles).unwrap();
    assert_eq!(hit.id.inner(), "cycle-41");
}

#[test]
fn a_tuesday_meeting_goes_into_the_cycle_that_started_that_day() {
    let cycles = weekly("2026-09-28", 3);
    let hit = cycle_for(day("2026-10-06"), "EX", &cycles).unwrap();
    assert_eq!(hit.id.inner(), "cycle-41");
}

#[test]
fn a_wednesday_meeting_goes_into_the_same_cycle_and_no_weekday_is_special() {
    let cycles = weekly("2026-09-28", 3);
    assert_eq!(
        cycle_for(day("2026-10-07"), "EX", &cycles)
            .unwrap()
            .id
            .inner(),
        "cycle-41"
    );
    // The day before the boundary: Monday 10-12's next day is Tuesday 10-13, the next cycle.
    assert_eq!(
        cycle_for(day("2026-10-12"), "EX", &cycles)
            .unwrap()
            .id
            .inner(),
        "cycle-42"
    );
    // Sunday 10-11: the next day is Monday 10-12, still in #41.
    assert_eq!(
        cycle_for(day("2026-10-11"), "EX", &cycles)
            .unwrap()
            .id
            .inner(),
        "cycle-41"
    );
}

#[test]
fn the_day_is_judged_at_noon_japan_time_so_a_midnight_boundary_does_not_decide() {
    // 2026-10-06 12:00 JST is 03:00 UTC.
    assert_eq!(
        instant_for(day("2026-10-05")),
        Utc.with_ymd_and_hms(2026, 10, 6, 3, 0, 0).unwrap()
    );
    // A cycle that starts at 00:00 UTC on the day after (09:00 JST) still contains noon JST.
    let utc_midnight: Vec<Cycle> = vec![serde_json::from_value(json!({
        "id": "c", "number": 1, "name": null,
        "startsAt": "2026-10-06T00:00:00.000Z", "endsAt": "2026-10-13T00:00:00.000Z",
    }))
    .unwrap()];
    assert!(cycle_for(day("2026-10-05"), "EX", &utc_midnight).is_ok());
}

#[test]
fn the_end_of_a_cycle_is_exclusive_and_the_start_inclusive() {
    let ends_at_noon: Vec<Cycle> = vec![serde_json::from_value(json!({
        "id": "c", "number": 1, "name": null,
        "startsAt": "2026-10-01T00:00:00Z", "endsAt": "2026-10-06T03:00:00Z",
    }))
    .unwrap()];
    assert!(cycle_for(day("2026-10-05"), "EX", &ends_at_noon).is_err());

    let starts_at_noon: Vec<Cycle> = vec![serde_json::from_value(json!({
        "id": "c", "number": 1, "name": null,
        "startsAt": "2026-10-06T03:00:00Z", "endsAt": "2026-10-13T03:00:00Z",
    }))
    .unwrap()];
    assert!(cycle_for(day("2026-10-05"), "EX", &starts_at_noon).is_ok());
}

#[test]
fn no_cycle_is_an_error_that_lists_the_cycles_there_are() {
    let cycles = weekly("2026-09-28", 2);
    let e = cycle_for(day("2026-11-30"), "EX", &cycles).unwrap_err();
    assert_eq!(e.code(), ErrorCode::Usage);
    let m = e.to_string();
    for want in [
        "no cycle of team EX contains 2026-12-01",
        "the meeting on 2026-11-30",
        "#40 2026-09-28T15:00:00Z .. 2026-10-05T15:00:00Z",
        "#41 2026-10-05T15:00:00Z .. 2026-10-12T15:00:00Z",
    ] {
        assert!(m.contains(want), "missing {want:?} in: {m}");
    }

    let none = cycle_for(day("2026-11-30"), "EX", &[])
        .unwrap_err()
        .to_string();
    assert!(none.ends_with("Cycles known: none"), "{none}");
}

#[test]
fn a_named_cycle_is_labelled_with_its_name() {
    let mut cycles = weekly("2026-09-28", 1);
    assert_eq!(label(&cycles[0]), "#40");
    cycles[0].name = Some("Sprint 40".into());
    assert_eq!(label(&cycles[0]), "#40 (Sprint 40)");
    cycles[0].name = Some("  ".into());
    assert_eq!(label(&cycles[0]), "#40");
}

// ---------------------------------------------------------------- the query

fn meta() -> ResponseMeta {
    ResponseMeta {
        status: 200,
        ..Default::default()
    }
}

#[test]
fn the_cycles_query_asks_for_one_team_and_decodes_a_page() {
    let req = build_request(&read::cycles(CycleListVars::new(
        PageVars {
            first: 100,
            after: None,
        },
        Some(cycles_of_team("lt3")),
    )));
    assert!(req.query.contains("cycles(first: $first"), "{}", req.query);
    assert_eq!(
        req.variables["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "lt3"}}})
    );
    assert_eq!(req.variables["first"], 100);

    let body = std::fs::read_to_string(format!(
        "{}/tests/fixtures/cycles.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 7, 0, 0, 0).unwrap();
    let page: CycleList = parse_response(&meta(), &body, now).unwrap();
    assert_eq!(page.cycles.nodes.len(), 2);
    assert_eq!(page.cycles.nodes[0].number, 41.0);
    assert_eq!(
        serde_json::to_value(&page.cycles.nodes[0]).unwrap()["startsAt"],
        Value::from("2026-10-05T15:00:00Z")
    );
}
