//! `project delete|unarchive`: the lead's ownership rule and exactly one
//! mutation. Against a mock Linear that answers by operation name.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

fn routes(lead: Option<&str>) -> Vec<(&'static str, Vec<Reply>)> {
    let ok = |field: &str| data(json!({ field: { "success": true } }));
    vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        ("ProjectRefsWithArchived", vec![project_refs()]),
        ("ProjectOwnershipQuery", vec![ownership(PROJECT, lead)]),
        ("ProjectDelete", vec![ok("projectDelete")]),
        ("ProjectUnarchive", vec![ok("projectUnarchive")]),
    ]
}

fn assert_nothing_written(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter()
            .all(|o| o != "ProjectDelete" && o != "ProjectUnarchive"),
        "a mutation was sent: {ops:?}"
    );
}

#[test]
fn delete_trashes_the_project_by_name() {
    let sb = workspace();
    let mock = Routed::start(routes(Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &["project", "delete", "Fixture Project", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["id"], PROJECT);
    assert_eq!(v["slugId"], "aaaaaaaaaaaa");
    assert_eq!(v["action"], "deleted");
    assert_eq!(mock.of("ProjectDelete"), vec![json!({ "id": PROJECT })]);
    // A live project is found among the live ones.
    assert!(!mock.ops().contains(&"ProjectRefsWithArchived".to_owned()));
}

#[test]
fn unarchive_finds_the_project_among_the_archived_ones_too() {
    let sb = workspace();
    let mock = Routed::start(routes(Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &["project", "unarchive", "aaaaaaaaaaaa", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["action"], "unarchived");
    assert_eq!(mock.of("ProjectUnarchive"), vec![json!({ "id": PROJECT })]);
    assert!(mock.ops().contains(&"ProjectRefsWithArchived".to_owned()));
    assert!(!mock.ops().contains(&"ProjectRefs".to_owned()));
}

#[test]
fn text_is_one_line_and_quiet_is_the_slug_id() {
    let sb = workspace();
    let mock = Routed::start(routes(Some(ALICE)));
    for (command, action) in [("delete", "deleted"), ("unarchive", "unarchived")] {
        let o = run(&sb, &mock, &["project", command, "Fixture Project"]);
        assert_eq!(code(&o), 0, "{command}: {}", stderr(&o));
        assert_eq!(
            stdout(&o).trim(),
            format!("aaaaaaaaaaaa  Fixture Project  ({action})")
        );
        let o = run(
            &sb,
            &mock,
            &["project", command, "Fixture Project", "--quiet"],
        );
        assert_eq!(stdout(&o).trim(), "aaaaaaaaaaaa");
    }
}

#[test]
fn only_the_lead_may_delete_or_restore_a_project() {
    let sb = workspace();
    for lead in [Some(BOT), None] {
        for command in ["delete", "unarchive"] {
            let mock = Routed::start(routes(lead));
            let o = run(&sb, &mock, &["project", command, "Fixture Project"]);
            assert_eq!(code(&o), 4, "{command} {lead:?}: {}", stderr(&o));
            assert_nothing_written(&mock);
        }
    }
}

#[test]
fn an_unknown_project_is_a_usage_error_and_nothing_is_written() {
    let sb = workspace();
    let mock = Routed::start(routes(Some(ALICE)));
    let o = run(&sb, &mock, &["project", "delete", "No Such Project"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_nothing_written(&mock);
}

#[test]
fn a_refusal_from_linear_is_an_error() {
    let sb = workspace();
    let mut r = routes(Some(ALICE));
    r.retain(|(o, _)| *o != "ProjectDelete");
    r.push((
        "ProjectDelete",
        vec![data(json!({ "projectDelete": { "success": false } }))],
    ));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["project", "delete", "Fixture Project"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
}
