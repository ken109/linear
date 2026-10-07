//! `initiative update|add-project|remove-project|status-update|status-updates`
//! against a mock Linear that answers by operation name. An initiative belongs
//! to the workspace, so no ownership rule applies to it (as for `create`); the
//! project of `add-project` and `remove-project` does have one.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const FIRST: &str = "00000000-0000-4000-8000-000000000100";
const SECOND: &str = "00000000-0000-4000-8000-000000000101";
const LINK: &str = "00000000-0000-4000-8000-000000000300";

fn initiative_json(n: usize, name: &str) -> Value {
    json!({
        "id": format!("00000000-0000-4000-8000-0000000001{n:02}"),
        "slugId": format!("slug{n}"), "name": name, "description": "Old text",
        "url": format!("https://linear.app/example/initiative/{name}"),
        "status": "Active", "targetDate": "2026-12-31",
        "owner": { "id": BOT, "name": "Linear", "displayName": "linear",
                   "email": "linear@example.com", "active": true, "isMe": false }
    })
}

fn initiatives() -> Reply {
    data(json!({ "initiatives": { "nodes": [
        initiative_json(0, "Alpha"), initiative_json(1, "Beta"),
    ], "pageInfo": { "hasNextPage": false, "endCursor": null } } }))
}

fn payload(field: &str, edit: impl FnOnce(&mut Value)) -> Reply {
    let mut i = initiative_json(1, "Beta");
    edit(&mut i);
    data(json!({ field: { "success": true, "initiative": i } }))
}

/// What `project(id) { initiativeToProjects }` returns: a link to each initiative id given.
fn links(initiatives: &[&str]) -> Reply {
    let nodes: Vec<Value> = initiatives
        .iter()
        .map(|i| json!({ "id": LINK, "initiative": { "id": i } }))
        .collect();
    data(json!({ "project": { "initiativeToProjects": { "nodes": nodes } } }))
}

fn routes(extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes: Vec<(&'static str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives()]),
        ("ProjectRefs", vec![project_refs()]),
        (
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(ALICE))],
        ),
        ("ProjectLinksQuery", vec![links(&[])]),
        ("Users", vec![users()]),
        (
            "InitiativeUpdate",
            vec![payload("initiativeUpdate", |_| {})],
        ),
        (
            "InitiativeToProjectCreate",
            vec![data(
                json!({ "initiativeToProjectCreate": { "success": true } }),
            )],
        ),
        (
            "InitiativeToProjectDelete",
            vec![data(
                json!({ "initiativeToProjectDelete": { "success": true } }),
            )],
        ),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

const WRITES: [&str; 4] = [
    "InitiativeUpdate",
    "InitiativeToProjectCreate",
    "InitiativeToProjectDelete",
    "InitiativeUpdateCreate",
];

fn assert_nothing_written(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter().all(|o| !WRITES.contains(&o.as_str())),
        "a mutation was sent: {ops:?}"
    );
}

// ------------------------------------------------------------------ update

#[test]
fn update_sends_only_the_fields_that_differ() {
    let sb = workspace();
    let description = write_file(&sb, "description.md", "\nNew text\n\n");
    let mock = Routed::start(routes(vec![(
        "InitiativeUpdate",
        vec![payload("initiativeUpdate", |i| {
            i["name"] = json!("Gamma");
            i["status"] = json!("Completed");
        })],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "update",
            "Beta",
            "--name",
            "Gamma",
            "--description-file",
            &description,
            "--status",
            "completed",
            // The same date as now: not sent.
            "--target-date",
            "2026-12-31",
            "--owner",
            "linear@example.com",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["name"], "Gamma");
    assert_eq!(v["changed"], json!(["name", "description", "status"]));
    assert_eq!(
        mock.of("InitiativeUpdate"),
        vec![json!({
            "id": SECOND,
            "input": { "name": "Gamma", "description": "New text", "status": "Completed" }
        })]
    );
}

#[test]
fn update_sets_the_target_date_and_the_owner() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "update",
            "slug0",
            "--target-date",
            "2027-03-31",
            "--owner",
            "me",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "slug1");
    assert_eq!(
        mock.of("InitiativeUpdate"),
        vec![json!({
            "id": FIRST,
            "input": { "targetDate": "2027-03-31", "ownerId": ALICE }
        })]
    );
}

#[test]
fn update_that_changes_nothing_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &["initiative", "update", "Beta", "--status", "active"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o).trim(),
        "slug1  Beta  Active  (already as asked, nothing changed)"
    );
    assert_nothing_written(&mock);
}

#[test]
fn update_text_names_the_changed_fields() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &["initiative", "update", "Beta", "--status", "planned"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("(updated status)"), "{}", stdout(&o));
}

#[test]
fn update_refuses_a_missing_change_an_empty_value_and_an_unknown_name() {
    let sb = workspace();
    let empty = write_file(&sb, "empty.md", " \n");
    let mock = Routed::start(routes(vec![]));
    for args in [
        vec!["initiative", "update", "Beta"],
        vec!["initiative", "update", "Beta", "--name", " "],
        vec!["initiative", "update", "Beta", "--description-file", &empty],
        vec!["initiative", "update", "Gamma", "--status", "active"],
        vec![
            "initiative",
            "update",
            "Beta",
            "--owner",
            "nobody@example.com",
        ],
        vec!["initiative", "update", "Beta", "--status", "paused"],
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
}

#[test]
fn a_refusal_from_linear_is_an_error() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "InitiativeUpdate",
        vec![data(
            json!({ "initiativeUpdate": { "success": false, "initiative": initiative_json(1, "Beta") } }),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &["initiative", "update", "Beta", "--status", "planned"],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
}

// ------------------------------------------------------------------ add-project, remove-project

#[test]
fn add_project_links_the_project_once() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "add-project",
            "Beta",
            "Fixture Project",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["action"], "added");
    assert_eq!(v["unchanged"], false);
    assert_eq!(v["initiative"]["slugId"], "slug1");
    assert_eq!(v["project"]["slugId"], "aaaaaaaaaaaa");
    assert_eq!(
        mock.of("InitiativeToProjectCreate"),
        vec![json!({ "input": { "initiativeId": SECOND, "projectId": PROJECT } })]
    );
}

#[test]
fn add_project_already_linked_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![("ProjectLinksQuery", vec![links(&[SECOND])])]));
    let o = run(
        &sb,
        &mock,
        &["initiative", "add-project", "Beta", "Fixture Project"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o).trim(),
        "slug1  Beta  <-  Fixture Project  (already under it, nothing sent)"
    );
    assert_nothing_written(&mock);
}

#[test]
fn remove_project_deletes_only_the_link() {
    let sb = workspace();
    // Linked to Beta and to Alpha: only Beta's link goes.
    let mock = Routed::start(routes(vec![(
        "ProjectLinksQuery",
        vec![data(
            json!({ "project": { "initiativeToProjects": { "nodes": [
            { "id": "00000000-0000-4000-8000-000000000301", "initiative": { "id": FIRST } },
            { "id": LINK, "initiative": { "id": SECOND } },
        ] } } }),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &["initiative", "remove-project", "Beta", "Fixture Project"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o).trim(),
        "slug1  Beta  <-  Fixture Project  (removed)"
    );
    assert_eq!(
        mock.of("InitiativeToProjectDelete"),
        vec![json!({ "id": LINK })]
    );
}

#[test]
fn remove_project_that_is_not_under_it_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![("ProjectLinksQuery", vec![links(&[FIRST])])]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "remove-project",
            "Beta",
            "Fixture Project",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["action"], "removed");
    assert_eq!(v["unchanged"], true);
    assert_nothing_written(&mock);
}

#[test]
fn a_project_somebody_else_leads_cannot_be_linked_or_unlinked() {
    let sb = workspace();
    for command in ["add-project", "remove-project"] {
        for lead in [Some(BOT), None] {
            let mock = Routed::start(routes(vec![(
                "ProjectOwnershipQuery",
                vec![ownership(PROJECT, lead)],
            )]));
            let o = run(
                &sb,
                &mock,
                &["initiative", command, "Beta", "Fixture Project"],
            );
            assert_eq!(code(&o), 4, "{command}: {}", stderr(&o));
            assert_nothing_written(&mock);
        }
    }
}

#[test]
fn an_unknown_initiative_or_project_is_a_usage_error() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    for args in [
        ["initiative", "add-project", "Gamma", "Fixture Project"],
        ["initiative", "remove-project", "Beta", "No Such Project"],
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
}

// ------------------------------------------------------------------ status update

fn status_update_json(health: &str) -> Value {
    json!({
        "id": "00000000-0000-4000-8000-000000000400",
        "url": "https://linear.app/example/initiative/beta/updates",
        "body": "Where it stands.", "health": health,
        "createdAt": "2026-10-07T01:00:00.000Z", "updatedAt": "2026-10-07T01:00:00.000Z",
        "user": { "id": ALICE, "name": "Alice Example", "displayName": "alice",
                  "email": "alice@example.com", "active": true, "isMe": true },
        "initiative": { "id": SECOND, "name": "Beta",
                        "url": "https://linear.app/example/initiative/Beta" }
    })
}

fn update_created(health: &str) -> Reply {
    data(json!({ "initiativeUpdateCreate": {
        "success": true, "initiativeUpdate": status_update_json(health)
    } }))
}

#[test]
fn status_update_writes_health_and_body() {
    let sb = workspace();
    let body = write_file(&sb, "update.md", "\n- Stage: 2\n- Next: wait\n\n");
    let mock = Routed::start(routes(vec![(
        "InitiativeUpdateCreate",
        vec![update_created("atRisk")],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "status-update",
            "Beta",
            "--health",
            "atRisk",
            "--body-file",
            &body,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["health"], "atRisk");
    assert_eq!(v["initiative"]["name"], "Beta");
    assert_eq!(
        mock.of("InitiativeUpdateCreate"),
        vec![json!({ "input": {
            "initiativeId": SECOND, "health": "atRisk", "body": "- Stage: 2\n- Next: wait"
        } })]
    );
    // The initiative has no owner rule, so even a person who is not its owner may write one.
    assert!(!mock.ops().contains(&"ProjectOwnershipQuery".to_owned()));
}

#[test]
fn status_update_text_and_quiet() {
    let sb = workspace();
    let body = write_file(&sb, "update.md", "Fine.\n");
    let mock = Routed::start(routes(vec![(
        "InitiativeUpdateCreate",
        vec![update_created("onTrack")],
    )]));
    let args = [
        "initiative",
        "status-update",
        "slug1",
        "--health",
        "onTrack",
        "--body-file",
        &body,
    ];
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o).trim(),
        "Beta  onTrack  https://linear.app/example/initiative/beta/updates"
    );
    let mut quiet = args.to_vec();
    quiet.push("--quiet");
    let o = run(&sb, &mock, &quiet);
    assert_eq!(
        stdout(&o).trim(),
        "https://linear.app/example/initiative/beta/updates"
    );
}

#[test]
fn status_update_needs_a_known_health_a_body_and_an_initiative() {
    let sb = workspace();
    let body = write_file(&sb, "update.md", "Where it stands.\n");
    let blank = write_file(&sb, "blank.md", " \n");
    let mock = Routed::start(routes(vec![]));
    for (initiative, health, file) in [
        ("Beta", "fine", body.as_str()),
        ("Beta", "onTrack", blank.as_str()),
        ("Gamma", "onTrack", body.as_str()),
    ] {
        let o = run(
            &sb,
            &mock,
            &[
                "initiative",
                "status-update",
                initiative,
                "--health",
                health,
                "--body-file",
                file,
            ],
        );
        assert_eq!(code(&o), 2, "{initiative} {health}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
}

#[test]
fn status_updates_lists_newest_first() {
    let sb = workspace();
    let mut older = status_update_json("offTrack");
    older["createdAt"] = json!("2026-09-30T01:00:00.000Z");
    older["body"] = json!("\nBlocked on review\nmore");
    older["url"] = json!("https://linear.app/example/initiative/beta/updates-old");
    let mock = Routed::start(routes(vec![(
        "InitiativeStatusUpdatesQuery",
        vec![data(json!({ "initiative": { "initiativeUpdates": {
            "nodes": [status_update_json("onTrack"), older]
        } } }))],
    )]));
    let o = run(&sb, &mock, &["initiative", "status-updates", "Beta"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("DATE"), "{text}");
    assert!(
        lines[1].contains("2026-10-07")
            && lines[1].contains("onTrack")
            && lines[1].contains("Alice Example")
            && lines[1].contains("Where it stands."),
        "{text}"
    );
    assert!(
        lines[2].contains("offTrack") && lines[2].contains("Blocked on review"),
        "{text}"
    );

    let o = run(
        &sb,
        &mock,
        &["initiative", "status-updates", "Beta", "--json"],
    );
    let v = stdout_json(&o);
    assert_eq!(v["updates"].as_array().unwrap().len(), 2);
    assert_eq!(v["updates"][1]["health"], "offTrack");

    let o = run(
        &sb,
        &mock,
        &["initiative", "status-updates", "Beta", "--quiet"],
    );
    assert_eq!(
        stdout(&o).trim(),
        "https://linear.app/example/initiative/beta/updates\nhttps://linear.app/example/initiative/beta/updates-old"
    );
}

#[test]
fn status_updates_says_when_there_are_none() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "InitiativeStatusUpdatesQuery",
        vec![data(
            json!({ "initiative": { "initiativeUpdates": { "nodes": [] } } }),
        )],
    )]));
    let o = run(&sb, &mock, &["initiative", "status-updates", "Beta"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "No status updates.");
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_of_update_lists_what_differs_and_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "update",
                "Beta",
                "--name",
                "Gamma",
                "--status",
                "completed",
                "--description-file",
                &write_file(&sb, "d.md", "Old text\n"),
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "initiative update");
    assert_eq!(v["changed"], json!(["name", "status"]));
    assert_eq!(planned(&v), ["InitiativeUpdate"]);
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "id": SECOND, "input": { "name": "Gamma", "status": "Completed" } })
    );

    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "update",
                "Beta",
                "--status",
                "active",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["mutations"], json!([]));
    // A refusal is the same as the real run's (exit 2).
    let o = run(&sb, &mock, &["initiative", "update", "Beta", "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_dry_run_of_add_and_remove_project_plans_the_link_change() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "add-project",
                "Beta",
                "Fixture Project",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "initiative add-project");
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "input": { "initiativeId": SECOND, "projectId": PROJECT } })
    );

    let mock = Routed::start(routes(vec![("ProjectLinksQuery", vec![links(&[SECOND])])]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "add-project",
                "Beta",
                "Fixture Project",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["mutations"], json!([]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "remove-project",
                "Beta",
                "Fixture Project",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(planned(&v), ["InitiativeToProjectDelete"]);
    assert_eq!(v["mutations"][0]["variables"], json!({ "id": LINK }));

    // The project's ownership still applies (exit 4).
    let mock = Routed::start(routes(vec![(
        "ProjectOwnershipQuery",
        vec![ownership(PROJECT, Some(BOT))],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "add-project",
            "Beta",
            "Fixture Project",
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_dry_run_of_status_update_plans_the_update() {
    let sb = workspace();
    let body = write_file(&sb, "update.md", "- Stage: 2\n");
    let mock = Routed::start(routes(vec![]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "initiative",
                "status-update",
                "Beta",
                "--health",
                "atRisk",
                "--body-file",
                &body,
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(planned(&v), ["InitiativeUpdateCreate"]);
    assert_eq!(
        v["mutations"][0]["variables"]["input"],
        json!({ "initiativeId": SECOND, "health": "atRisk", "body": "- Stage: 2" })
    );
}
