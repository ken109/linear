//! `linear cycle list` and `linear cycle view` against a mock Linear, and the
//! guarantee that `linear cycle <DATE>` still works next to them.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

fn infos() -> Value {
    serde_json::from_str::<Value>(&fixture("cycle_infos")).unwrap()["data"].clone()
}

/// What `cycles` answers when only the cycle with this number matches.
fn only(number: u64) -> Reply {
    let mut v = infos();
    v["cycles"]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["number"] == number);
    data(v)
}

fn issues_page(has_next: bool, cursor: Option<&str>, which: &[&str]) -> Reply {
    let mut v: Value =
        serde_json::from_str::<Value>(&fixture("cycle_issues")).unwrap()["data"].clone();
    v["cycle"]["issues"]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|i| which.contains(&i["identifier"].as_str().unwrap()));
    v["cycle"]["issues"]["pageInfo"] = json!({"hasNextPage": has_next, "endCursor": cursor});
    data(v)
}

fn list_routes() -> Vec<(&'static str, Vec<Reply>)> {
    vec![("CycleInfoList", vec![data(infos())])]
}

// ------------------------------------------------------------------ cycle list

#[test]
fn list_prints_the_teams_cycles_newest_first() {
    let sb = workspace();
    let mock = Routed::start(list_routes());
    let o = run(&sb, &mock, &["cycle", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let out = stdout(&o);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("NUMBER") && lines[0].contains("STATE"),
        "{out}"
    );
    // 43 upcoming, 42 next, 41 active (named), 40 past.
    let order: Vec<&str> = lines[1..]
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(order, ["#43", "#42", "#41", "#40"], "{out}");
    assert!(
        lines[3].contains("Autumn sprint")
            && lines[3].contains("active")
            && lines[3].contains("25%")
    );
    assert!(
        lines[2].contains("next") && lines[1].contains("upcoming") && lines[4].contains("past")
    );
    assert!(lines[4].contains("75%"));

    // The workspace's default team is the only filter, and a listing writes nothing.
    assert_eq!(
        mock.of("CycleInfoList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}})
    );
    assert_eq!(mock.ops(), ["CycleInfoList"]);
}

#[test]
fn list_json_is_the_cycles_tagged_with_the_workspace() {
    let sb = workspace();
    let mock = Routed::start(list_routes());
    let o = run(&sb, &mock, &["cycle", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["workspace"], "example");
    assert_eq!(rows[0]["number"], 43);
    assert_eq!(rows[2]["isActive"], true);
    assert_eq!(rows[2]["team"]["key"], "EX");
    assert_eq!(rows[2]["description"], "Ship the fixtures.");
}

#[test]
fn list_quiet_prints_one_number_per_line() {
    let sb = workspace();
    let mock = Routed::start(list_routes());
    let o = run(&sb, &mock, &["cycle", "list", "--quiet"]);
    assert_eq!(stdout(&o), "43\n42\n41\n40\n");
}

#[test]
fn list_takes_the_team_and_the_state_as_filters() {
    let sb = workspace();
    let mock = Routed::start(list_routes());
    let o = run(
        &sb,
        &mock,
        &[
            "cycle", "list", "--team", "lt3", "--state", "active", "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("CycleInfoList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "lt3"}}, "isActive": {"eq": true}})
    );

    let o = run(&sb, &mock, &["cycle", "list", "--state", "later"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn list_limit_keeps_the_newest_and_says_it_cut_the_list() {
    let sb = workspace();
    let mock = Routed::start(list_routes());
    let o = run(&sb, &mock, &["cycle", "list", "--limit", "2", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "43\n42\n");
    assert!(
        stderr(&o).contains("showing the first 2 results"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn list_reads_every_page_of_cycles() {
    let sb = workspace();
    let all = infos();
    let page = |nodes: Vec<Value>, next: bool, cursor: Value| {
        data(
            json!({"cycles": {"nodes": nodes, "pageInfo": {"hasNextPage": next, "endCursor": cursor}}}),
        )
    };
    let nodes = all["cycles"]["nodes"].as_array().unwrap().clone();
    let mock = Routed::start(vec![(
        "CycleInfoList",
        vec![
            page(nodes[..2].to_vec(), true, json!("c1")),
            page(nodes[2..].to_vec(), false, Value::Null),
        ],
    )]);
    let o = run(&sb, &mock, &["cycle", "list", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "43\n42\n41\n40\n");
    let calls = mock.of("CycleInfoList");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1]["after"], "c1");
}

#[test]
fn a_team_without_cycles_lists_nothing_and_succeeds() {
    let sb = workspace();
    let empty = data(
        json!({"cycles": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}}),
    );
    let mock = Routed::start(vec![("CycleInfoList", vec![empty])]);
    let o = run(&sb, &mock, &["cycle", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "No cycles found for team EX.");
}

// ------------------------------------------------------------------ cycle view

fn view_routes() -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("CycleInfoList", vec![only(41)]),
        (
            "CycleIssuesQuery",
            vec![issues_page(false, None, &["EX-1", "EX-2"])],
        ),
    ]
}

#[test]
fn view_shows_the_cycle_and_its_issues() {
    let sb = workspace();
    let mock = Routed::start(view_routes());
    let o = run(&sb, &mock, &["cycle", "view", "41"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let out = stdout(&o);
    for want in [
        "Cycle #41 (Autumn sprint)  (EX)",
        "State:",
        "active",
        "Progress: 25%",
        "Ship the fixtures.",
        "Issues (2)",
        "EX-1",
        "In Progress",
        "Write the fixture issue",
        "EX-2",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }

    // The cycle is looked up by team and number, then its issues are read; nothing is written.
    assert_eq!(
        mock.of("CycleInfoList")[0]["filter"],
        json!({"team": {"key": {"eqIgnoreCase": "EX"}}, "number": {"eq": 41.0}})
    );
    assert_eq!(
        mock.of("CycleIssuesQuery")[0]["id"],
        "00000000-0000-4000-8000-000000000201"
    );
    assert_eq!(mock.ops(), ["CycleInfoList", "CycleIssuesQuery"]);
}

#[test]
fn view_json_carries_the_issues() {
    let sb = workspace();
    let mock = Routed::start(view_routes());
    let o = run(&sb, &mock, &["cycle", "view", "41", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["number"], 41);
    assert_eq!(v["id"], CYCLE_41);
    assert_eq!(v["team"]["key"], "EX");
    let issues = v["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0]["identifier"], "EX-1");
    assert_eq!(issues[0]["state"]["name"], "In Progress");

    let o = run(&sb, &mock, &["cycle", "view", "41", "--quiet"]);
    assert_eq!(stdout(&o), "41\n");
}

#[test]
fn view_follows_the_issue_pages_up_to_the_limit() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("CycleInfoList", vec![only(41)]),
        (
            "CycleIssuesQuery",
            vec![
                issues_page(true, Some("i1"), &["EX-1"]),
                issues_page(false, None, &["EX-2"]),
            ],
        ),
    ]);
    let o = run(&sb, &mock, &["cycle", "view", "41", "--all", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["issues"].as_array().unwrap().len(), 2);
    assert_eq!(mock.of("CycleIssuesQuery")[1]["after"], "i1");

    // With a limit of one, the second page is not needed and a note says so.
    let mock = Routed::start(vec![
        ("CycleInfoList", vec![only(41)]),
        (
            "CycleIssuesQuery",
            vec![issues_page(true, Some("i1"), &["EX-1"])],
        ),
    ]);
    let o = run(
        &sb,
        &mock,
        &["cycle", "view", "41", "--limit", "1", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["issues"].as_array().unwrap().len(), 1);
    assert!(
        stderr(&o).contains("showing the first 1 results"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn view_of_a_number_the_team_does_not_have_is_a_usage_error() {
    let sb = workspace();
    let empty = data(
        json!({"cycles": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}}),
    );
    let mock = Routed::start(vec![("CycleInfoList", vec![empty])]);
    let o = run(&sb, &mock, &["cycle", "view", "99"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    assert!(
        stderr(&o).contains("team EX has no cycle #99"),
        "{}",
        stderr(&o)
    );
    assert_eq!(mock.ops(), ["CycleInfoList"]);

    let o = run(&sb, &mock, &["cycle", "view", "forty"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

// ------------------------------------------------------------------ cycle <DATE> is unchanged

#[test]
fn the_date_form_still_works_beside_the_subcommands() {
    let sb = workspace();
    let mock = Routed::start(vec![("CycleList", vec![cycles()])]);
    let o = run(&sb, &mock, &["cycle", MEETING, "--team", "lt3", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["id"], CYCLE_41);
    assert_eq!(v["heldOn"], MEETING);
    assert_eq!(v["team"], "lt3");
    assert_eq!(mock.ops(), ["CycleList"]);
}

#[test]
fn cycle_without_a_day_or_a_subcommand_is_a_usage_error() {
    let sb = workspace();
    let mock = Routed::start(vec![]);
    let o = run(&sb, &mock, &["cycle"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());

    // A word that is neither a date nor a subcommand is refused, not sent.
    let o = run(&sb, &mock, &["cycle", "tomorrow"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}
