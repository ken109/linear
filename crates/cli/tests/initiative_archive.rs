//! `initiative archive|unarchive|delete`: found by reference, one mutation, one
//! line. An initiative belongs to the workspace, so no ownership rule applies
//! (as for `create`). Against a mock Linear that answers by operation name.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const FIRST: &str = "00000000-0000-4000-8000-000000000100";
const SECOND: &str = "00000000-0000-4000-8000-000000000101";

fn initiatives(names: &[&str]) -> Reply {
    let nodes: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            json!({
                "id": format!("00000000-0000-4000-8000-0000000001{n:02}"),
                "slugId": format!("slug{n}"), "name": name, "description": null,
                "url": format!("https://linear.app/example/initiative/{name}"),
                "status": "Active", "targetDate": null, "owner": null
            })
        })
        .collect();
    data(
        json!({ "initiatives": { "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } }),
    )
}

fn routes() -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives(&["Alpha", "Beta"])]),
        (
            "InitiativeListWithArchived",
            vec![initiatives(&["Alpha", "Beta"])],
        ),
        (
            "InitiativeArchive",
            vec![data(json!({ "initiativeArchive": { "success": true } }))],
        ),
        (
            "InitiativeUnarchive",
            vec![data(json!({ "initiativeUnarchive": { "success": true } }))],
        ),
        (
            "InitiativeDelete",
            vec![data(json!({ "initiativeDelete": { "success": true } }))],
        ),
    ]
}

/// (command, the mutation it sends, what it reports, the list it reads).
const CASES: [(&str, &str, &str, &str); 3] = [
    ("archive", "InitiativeArchive", "archived", "InitiativeList"),
    (
        "unarchive",
        "InitiativeUnarchive",
        "unarchived",
        "InitiativeListWithArchived",
    ),
    ("delete", "InitiativeDelete", "deleted", "InitiativeList"),
];

#[test]
fn each_command_sends_its_one_mutation_with_the_initiative_id() {
    let sb = workspace();
    for (command, mutation, action, list) in CASES {
        let mock = Routed::start(routes());
        let o = run(&sb, &mock, &["initiative", command, "Beta", "--json"]);
        assert_eq!(code(&o), 0, "{command}: {}", stderr(&o));
        let v = stdout_json(&o);
        assert_eq!(v["workspace"], "example");
        assert_eq!(v["id"], SECOND);
        assert_eq!(v["slugId"], "slug1");
        assert_eq!(v["name"], "Beta");
        assert_eq!(v["action"], action);
        assert_eq!(
            mock.of(mutation),
            vec![json!({ "id": SECOND })],
            "{command}"
        );
        // Only `unarchive` has to look among the archived initiatives.
        let ops = mock.ops();
        assert!(ops.contains(&list.to_owned()), "{command}: {ops:?}");
        assert_eq!(
            ops.contains(&"InitiativeListWithArchived".to_owned()),
            command == "unarchive",
            "{command}: {ops:?}"
        );
    }
}

#[test]
fn an_initiative_is_found_by_slug_id_too() {
    let sb = workspace();
    let mock = Routed::start(routes());
    let o = run(&sb, &mock, &["initiative", "archive", "slug0"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("InitiativeArchive"), vec![json!({ "id": FIRST })]);
}

#[test]
fn text_is_one_line_and_quiet_is_the_slug_id() {
    let sb = workspace();
    for (command, _, action, _) in CASES {
        let mock = Routed::start(routes());
        let o = run(&sb, &mock, &["initiative", command, "Beta"]);
        assert_eq!(code(&o), 0, "{command}: {}", stderr(&o));
        assert_eq!(stdout(&o).trim(), format!("slug1  Beta  ({action})"));
        let o = run(&sb, &mock, &["initiative", command, "Beta", "--quiet"]);
        assert_eq!(stdout(&o).trim(), "slug1");
    }
}

#[test]
fn an_unknown_initiative_is_a_usage_error_and_nothing_is_written() {
    let sb = workspace();
    for (command, mutation, _, _) in CASES {
        let mock = Routed::start(routes());
        let o = run(&sb, &mock, &["initiative", command, "Gamma"]);
        assert_eq!(code(&o), 2, "{command}: {}", stderr(&o));
        assert!(mock.of(mutation).is_empty());
    }
}

#[test]
fn a_refusal_from_linear_is_an_error() {
    let sb = workspace();
    let mut r = routes();
    r.retain(|(o, _)| *o != "InitiativeDelete");
    r.push((
        "InitiativeDelete",
        vec![data(json!({ "initiativeDelete": { "success": false } }))],
    ));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["initiative", "delete", "Beta"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
}

#[test]
fn a_dry_run_of_each_command_plans_its_one_mutation() {
    let sb = workspace();
    for (command, mutation, _, list) in CASES {
        let mock = Routed::start(routes());
        let v = plan(
            &run(
                &sb,
                &mock,
                &["initiative", command, "Beta", "--dry-run", "--json"],
            ),
            &mock,
        );
        assert_eq!(v["command"], format!("initiative {command}"));
        assert_eq!(planned(&v), [mutation], "{command}");
        assert_eq!(v["mutations"][0]["variables"], json!({ "id": SECOND }));
        assert!(mock.ops().contains(&list.to_owned()), "{command}");
    }
    // An unknown initiative is a usage error and nothing is planned.
    let mock = Routed::start(routes());
    let o = run(&sb, &mock, &["initiative", "delete", "Nope", "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}
