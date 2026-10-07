//! The cycle list and view queries: the filters they send, the fragments they
//! parse and the states they report.

use linear_core::cycle_read::{
    self, CycleInfo, CycleInfoList, CycleInfoListVars, CycleIssuesQuery, CycleIssuesVars,
    CycleStatus,
};
use linear_core::filters::{cycle_numbered, cycles_in_state, CycleState};
use linear_core::types::PageVars;
use linear_core::wire::{build_request, parse_response, ResponseMeta};
use serde_json::{json, Value};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    parse_response(&meta, &fixture(name), chrono::Utc::now()).unwrap()
}

fn page() -> PageVars {
    PageVars {
        first: 100,
        after: None,
    }
}

fn filter_json(filter: linear_core::filters::CycleFilter) -> Value {
    let req = build_request(&cycle_read::cycle_infos(CycleInfoListVars::new(
        page(),
        Some(filter),
    )));
    req.variables["filter"].clone()
}

#[test]
fn a_state_narrows_the_team_filter_with_one_comparator() {
    assert_eq!(
        filter_json(cycles_in_state("EX", None)),
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
    assert_eq!(
        filter_json(cycles_in_state("EX", Some(CycleState::Active))),
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}, "isActive": {"eq": true}})
    );
    assert_eq!(
        filter_json(cycles_in_state("EX", Some(CycleState::Upcoming))),
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}, "isFuture": {"eq": true}})
    );
    assert_eq!(
        filter_json(cycles_in_state("EX", Some(CycleState::Past))),
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}, "isPast": {"eq": true}})
    );
}

#[test]
fn a_cycle_is_found_by_its_team_and_number() {
    assert_eq!(
        filter_json(cycle_numbered("EX", 41)),
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}, "number": {"eq": 41.0}})
    );
}

#[test]
fn the_listing_parses_and_reports_where_each_cycle_is_in_time() {
    let data: CycleInfoList = parse("cycle_infos");
    let cycles = &data.cycles.nodes;
    let status: Vec<CycleStatus> = cycles.iter().map(CycleInfo::status).collect();
    assert_eq!(
        status,
        [
            CycleStatus::Past,
            CycleStatus::Active,
            CycleStatus::Next,
            CycleStatus::Upcoming
        ]
    );
    assert_eq!(cycles[1].label(), "#41 (Autumn sprint)");
    assert_eq!(cycles[0].label(), "#40");
    assert_eq!(cycles[1].number_whole(), 41);
    assert_eq!(cycles[1].team.key, "EX");
    assert!(cycles[0].completed_at.is_some());
}

#[test]
fn a_cycle_serializes_its_number_as_an_integer() {
    let data: CycleInfoList = parse("cycle_infos");
    let v = serde_json::to_value(&data.cycles.nodes[1]).unwrap();
    assert_eq!(v["number"], 41);
    assert_eq!(v["isActive"], true);
    assert_eq!(v["progress"], 0.25);
}

#[test]
fn the_issues_of_a_cycle_parse_and_the_query_names_the_cycle() {
    let data: CycleIssuesQuery = parse("cycle_issues");
    let ids: Vec<&str> = data
        .cycle
        .issues
        .nodes
        .iter()
        .map(|i| i.identifier.as_str())
        .collect();
    assert_eq!(ids, ["EX-1", "EX-2"]);

    let req = build_request(&cycle_read::cycle_issues(CycleIssuesVars::new(
        "cycle-id",
        page(),
    )));
    assert_eq!(req.operation_name.as_deref(), Some("CycleIssuesQuery"));
    assert_eq!(req.variables["id"], "cycle-id");
    assert_eq!(req.variables["first"], 100);
}
