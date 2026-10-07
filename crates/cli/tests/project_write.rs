//! `linear project create|update|reorder|status-update` against a mock Linear
//! that answers by operation name: the path every write takes (guard,
//! validators, mutation, rollback), checked from outside.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const TEMPLATE_RULE: [&str; 1] = ["template-sections"];
const STATUS_COMPLETED: &str = "00000000-0000-4000-8000-000000000013";
const STATUS_STARTED: &str = "00000000-0000-4000-8000-000000000012";
const INITIATIVE_EXAMPLE: &str = "00000000-0000-4000-8000-000000000101";
/// The initiative `Fixture Project` is already under (see `project_view.json`).
const INITIATIVE_LINKED: &str = "00000000-0000-4000-8000-000000000021";
const TEAM: &str = "00000000-0000-4000-8000-000000000004";

const PROJECT_WRITES: [&str; 5] = [
    "ProjectCreate",
    "ProjectUpdate",
    "ProjectDelete",
    "ProjectUpdateCreate",
    "InitiativeToProjectCreate",
];

fn assert_no_project_write(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter().all(|o| !PROJECT_WRITES.contains(&o.as_str())),
        "a mutation was sent: {ops:?}"
    );
}

fn view_data() -> Value {
    serde_json::from_str::<Value>(&fixture("project_view")).unwrap()["data"].clone()
}

/// What `project_view` returns, after `edit` has changed it.
fn project_view_with(edit: impl FnOnce(&mut Value)) -> Reply {
    let mut v = view_data();
    edit(&mut v);
    data(v)
}

fn unfinished(names: &[(&str, &str)]) -> Reply {
    let nodes: Vec<Value> = names
        .iter()
        .map(|(id, name)| {
            json!({
                "id": id, "slugId": "bbbbbbbbbbbb", "name": name,
                "url": "https://linear.app/example/project/twin-bbbbbbbbbbbb"
            })
        })
        .collect();
    data(json!({ "projects": { "nodes": nodes } }))
}

fn statuses() -> Reply {
    data(json!({ "projectStatuses": { "nodes": [
        { "id": STATUS_STARTED, "name": "In Progress", "type": "started" },
        { "id": STATUS_COMPLETED, "name": "Completed", "type": "completed" },
    ]}}))
}

fn initiatives() -> Reply {
    let mut v: Value = serde_json::from_str(&fixture("initiatives")).unwrap();
    // `Fixture Initiative` is the one `project_view.json` already links.
    let mut linked = v["data"]["initiatives"]["nodes"][0].clone();
    linked["id"] = json!(INITIATIVE_LINKED);
    linked["name"] = json!("Fixture Initiative");
    linked["slugId"] = json!("dddddddddddd");
    v["data"]["initiatives"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(linked);
    ok(&v.to_string())
}

fn project_payload(field: &str) -> Reply {
    data(json!({ field: { "success": true, "project": view_data()["project"] } }))
}

fn link_ok() -> Reply {
    data(json!({ "initiativeToProjectCreate": { "success": true } }))
}

fn delete_project_ok() -> Reply {
    data(json!({ "projectDelete": { "success": true } }))
}

fn status_update_payload() -> Reply {
    let mut update = view_data()["project"]["lastUpdate"].clone();
    update["body"] = json!("Where it stands.");
    update["health"] = json!("atRisk");
    data(json!({ "projectUpdateCreate": { "success": true, "projectUpdate": update } }))
}

/// A `Project Template` whose body has one section.
fn templates_with_project_template() -> Reply {
    let mut v: Value = serde_json::from_str(&fixture("templates_sections")).unwrap();
    let doc = json!({ "descriptionData": { "type": "doc", "content": [
        { "type": "heading", "attrs": { "level": 2 },
          "content": [{ "type": "text", "text": "完了の定義" }] },
        { "type": "paragraph", "content": [{ "type": "text", "text": "What done looks like." }] },
    ]}});
    v["data"]["templates"].as_array_mut().unwrap().push(json!({
        "id": "00000000-0000-4000-8000-000000000052",
        "name": "Project Template",
        "description": null,
        "type": "project",
        "team": null,
        "templateData": doc.to_string(),
        "updatedAt": "2026-10-06T13:35:53.246Z",
    }));
    ok(&v.to_string())
}

fn create_routes(extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes: Vec<(&str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("Users", vec![users()]),
        ("InitiativeList", vec![initiatives()]),
        ("UnfinishedNamed", vec![unfinished(&[])]),
        ("Templates", vec![templates_with_project_template()]),
        ("ProjectCreate", vec![project_payload("projectCreate")]),
        ("InitiativeToProjectCreate", vec![link_ok()]),
        ("ProjectDelete", vec![delete_project_ok()]),
    ];
    override_routes(&mut routes, extra);
    routes
}

fn update_routes(extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes: Vec<(&str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        ("ProjectView", vec![project_view_with(|_| {})]),
        ("ProjectStatuses", vec![statuses()]),
        ("Users", vec![users()]),
        ("InitiativeList", vec![initiatives()]),
        ("UnfinishedNamed", vec![unfinished(&[])]),
        ("Templates", vec![templates_with_project_template()]),
        ("ProjectUpdate", vec![project_payload("projectUpdate")]),
        ("InitiativeToProjectCreate", vec![link_ok()]),
    ];
    override_routes(&mut routes, extra);
    routes
}

fn override_routes<'a>(routes: &mut Vec<(&'a str, Vec<Reply>)>, extra: Vec<(&'a str, Vec<Reply>)>) {
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
}

const GOOD_PROJECT_BODY: &str = "## Goal\n\nShip it.\n\n## 完了の定義\n\n- shipped\n";

// ------------------------------------------------------------------ create

#[test]
fn create_resolves_names_looks_for_a_twin_then_creates_and_links() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", GOOD_PROJECT_BODY);
    let mock = Routed::start(create_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "New Thing",
            "--summary",
            "A short one",
            "--body-file",
            &body,
            "--target-date",
            "2026-12-01",
            "--initiative",
            "Example Initiative",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["existing"], false);
    assert_eq!(v["slugId"], "aaaaaaaaaaaa");

    // Reads first (the twin check among them), then the creation, then the link.
    let ops = mock.ops();
    let first_write = ops.iter().position(|o| o == "ProjectCreate").unwrap();
    assert!(ops[..first_write].contains(&"UnfinishedNamed".to_owned()));
    assert_eq!(
        &ops[first_write..],
        ["ProjectCreate", "InitiativeToProjectCreate"]
    );

    assert_eq!(
        mock.of("UnfinishedNamed")[0]["filter"],
        json!({
            "name": { "eq": "New Thing" },
            "status": { "type": { "nin": ["completed", "canceled"] } }
        }),
        "the twin check is by exact name among unfinished projects"
    );
    assert_eq!(
        mock.of("ProjectCreate")[0]["input"],
        json!({
            "name": "New Thing",
            "teamIds": [TEAM],
            "description": "A short one",
            "content": GOOD_PROJECT_BODY.trim_end(),
            // No --lead: the viewer.
            "leadId": ALICE,
            "targetDate": "2026-12-01",
        })
    );
    assert_eq!(
        mock.of("InitiativeToProjectCreate")[0]["input"],
        json!({ "initiativeId": INITIATIVE_EXAMPLE, "projectId": PROJECT })
    );
}

#[test]
fn a_project_with_the_same_unfinished_name_is_returned_not_created() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![(
        "UnfinishedNamed",
        vec![unfinished(&[(PROJECT, "New Thing")])],
    )]));

    let args = ["project", "create", "--name", "New Thing", "--lead", "me"];
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let o = run(&sb, &mock, &with_json);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["id"], PROJECT);
    assert_no_project_write(&mock);

    let mut quiet = args.to_vec();
    quiet.push("--quiet");
    let o = run(&sb, &mock, &quiet);
    assert_eq!(stdout(&o).trim(), "bbbbbbbbbbbb");
}

#[test]
fn creating_a_project_somebody_else_leads_is_refused_with_exit_4() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "Theirs",
            "--lead",
            "linear@example.com",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("must have you as its lead"),
        "{}",
        stderr(&o)
    );
    assert_no_project_write(&mock);
    // The refusal comes before anything is looked up about the name.
    assert!(!mock.ops().contains(&"UnfinishedNamed".to_owned()));

    // `--lead me` is the same as no --lead.
    let o = run(
        &sb,
        &mock,
        &[
            "project", "create", "--name", "Mine", "--lead", "me", "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("ProjectCreate")[0]["input"]["leadId"], ALICE);
}

#[test]
fn the_project_template_rule_can_require_the_definition_of_done() {
    let sb = workspace_with_rules(&TEMPLATE_RULE);
    let thin = write_file(&sb, "thin.md", "## Goal\n\nOnly this.\n");
    let good = write_file(&sb, "good.md", GOOD_PROJECT_BODY);
    let mock = Routed::start(create_routes(vec![]));

    // No template named.
    let o = run(&sb, &mock, &["project", "create", "--name", "P"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("a template is required"),
        "{}",
        stderr(&o)
    );
    assert_no_project_write(&mock);

    // A body without the section.
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "P",
            "--template",
            "Project Template",
            "--body-file",
            &thin,
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("完了の定義"), "{}", stderr(&o));
    assert_no_project_write(&mock);

    // An issue template is not a project template.
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "P",
            "--template",
            "Sectioned Template",
            "--body-file",
            &good,
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("no project template named"),
        "{}",
        stderr(&o)
    );
    assert_no_project_write(&mock);

    // A body that fills it.
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "P",
            "--template",
            "Project Template",
            "--body-file",
            &good,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("ProjectCreate").len(), 1);
}

#[test]
fn without_rules_a_template_is_said_to_be_ignored() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &["project", "create", "--name", "P", "--template", "Whatever"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--template is ignored"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn a_failed_link_is_retried_then_takes_the_project_with_it() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![(
        "InitiativeToProjectCreate",
        vec![graphql_error("link refused")],
    )]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "New Thing",
            "--initiative",
            "Example Initiative",
        ],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "a failed create prints no result");
    let err = stderr(&o);
    assert!(err.contains("link refused"), "{err}");
    assert!(err.contains("rolled back: deleted project"), "{err}");
    assert_eq!(mock.of("InitiativeToProjectCreate").len(), 3);
    assert_eq!(mock.of("ProjectDelete"), vec![json!({ "id": PROJECT })]);
    assert_eq!(mock.ops().last().unwrap(), "ProjectDelete");
}

#[test]
fn a_rollback_that_fails_says_what_is_left_behind() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![
        (
            "InitiativeToProjectCreate",
            vec![graphql_error("link refused")],
        ),
        ("ProjectDelete", vec![graphql_error("no delete for you")]),
    ]));
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "create",
            "--name",
            "New Thing",
            "--initiative",
            "Example Initiative",
        ],
    );
    assert_eq!(code(&o), 1);
    let err = stderr(&o);
    assert!(err.contains("COULD NOT roll back"), "{err}");
    assert!(err.contains("no delete for you"), "{err}");
}

#[test]
fn a_project_without_an_initiative_needs_no_rollback() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &["project", "create", "--name", "Solo", "--quiet"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // One mutation, and nothing else to undo.
    assert_eq!(
        mock.ops()
            .iter()
            .filter(|o| PROJECT_WRITES.contains(&o.as_str()))
            .count(),
        1
    );
    // Optional fields that were not given are not sent (never `null`).
    assert_eq!(
        mock.of("ProjectCreate")[0]["input"],
        json!({ "name": "Solo", "teamIds": [TEAM], "leadId": ALICE })
    );
}

#[test]
fn unknown_names_and_blank_text_stop_a_create_before_it_starts() {
    let sb = workspace_with_rules(&[]);
    let blank = write_file(&sb, "blank.md", "  \n\n");
    let mock = Routed::start(create_routes(vec![]));
    for (args, what) in [
        (vec!["--name", "P", "--initiative", "Nope"], "initiative"),
        (vec!["--name", "P", "--lead", "nobody@example.com"], "user"),
        (vec!["--name", "P", "--team", "ZZ"], "team"),
        (vec!["--name", "  "], "name"),
        (vec!["--name", "P", "--summary", " "], "summary"),
        (vec!["--name", "P", "--body-file", blank.as_str()], "body"),
        (vec!["--name", "P", "--target-date", "soon"], "target-date"),
    ] {
        let mut full = vec!["project", "create"];
        full.extend(args.iter());
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(what), "{args:?}: {}", stderr(&o));
    }
    assert_no_project_write(&mock);
}

// ------------------------------------------------------------------ update

#[test]
fn update_writes_only_what_differs() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--status",
            "completed",
            // The same as now: not sent.
            "--target-date",
            "2026-12-31",
            "--name",
            "Fixture Project",
            "--lead",
            "me",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["changed"], json!(["status"]));
    assert_eq!(v["slugId"], "aaaaaaaaaaaa");
    assert_eq!(
        mock.of("ProjectUpdate"),
        vec![json!({ "id": PROJECT, "input": { "statusId": STATUS_COMPLETED } })],
        "an untouched field is omitted, never null"
    );
    // Nothing was looked up that no flag asked for.
    assert!(!mock.ops().contains(&"InitiativeList".to_owned()));
}

#[test]
fn update_sends_every_field_asked_for_and_nothing_else() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "## New body\n");
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "aaaaaaaaaaaa",
            "--name",
            "Renamed",
            "--summary",
            "New summary",
            "--body-file",
            &body,
            "--status",
            "Completed",
            "--target-date",
            "2027-01-31",
            "--lead",
            "linear@example.com",
            "--json",
        ],
    );
    // I lead it now, so I may change it, and I may hand the lead on.
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("ProjectUpdate")[0]["input"],
        json!({
            "name": "Renamed",
            "description": "New summary",
            "content": "## New body",
            "statusId": STATUS_COMPLETED,
            "targetDate": "2027-01-31",
            "leadId": BOT,
        })
    );
    assert_eq!(
        stdout_json(&o)["changed"],
        json!(["name", "summary", "body", "status", "targetDate", "lead"])
    );
}

#[test]
fn update_with_everything_already_as_asked_writes_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(vec![]));
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--status",
            "in progress",
            "--initiative",
            "Fixture Initiative",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!([]));
    assert_no_project_write(&mock);
}

#[test]
fn update_adds_the_initiative_only_when_it_is_not_linked() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--initiative",
            "Example Initiative",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["initiative"]));
    assert_eq!(
        mock.of("InitiativeToProjectCreate")[0]["input"],
        json!({ "initiativeId": INITIATIVE_EXAMPLE, "projectId": PROJECT })
    );
    // Only a link: the project's own fields are not written.
    assert!(mock.of("ProjectUpdate").is_empty());
}

#[test]
fn a_failed_link_puts_the_other_fields_back() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(vec![(
        "InitiativeToProjectCreate",
        vec![graphql_error("link refused")],
    )]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--status",
            "Completed",
            "--target-date",
            "2027-01-31",
            "--initiative",
            "Example Initiative",
        ],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("rolled back: restored the project's earlier values"),
        "{}",
        stderr(&o)
    );
    let writes = mock.of("ProjectUpdate");
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0]["input"],
        json!({ "statusId": STATUS_COMPLETED, "targetDate": "2027-01-31" })
    );
    assert_eq!(
        writes[1]["input"],
        json!({ "statusId": STATUS_STARTED, "targetDate": "2026-12-31" }),
        "the second write is the earlier values"
    );
}

#[test]
fn restoring_a_field_that_was_empty_clears_it() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "## New body\n");
    let mock = Routed::start(update_routes(vec![
        (
            "ProjectView",
            vec![project_view_with(|v| {
                v["project"]["targetDate"] = Value::Null;
                v["detail"]["content"] = Value::Null;
            })],
        ),
        (
            "InitiativeToProjectCreate",
            vec![graphql_error("link refused")],
        ),
    ]));
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--target-date",
            "2027-01-31",
            "--body-file",
            &body,
            "--initiative",
            "Example Initiative",
        ],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let writes = mock.of("ProjectUpdate");
    assert_eq!(
        writes[1]["input"],
        json!({ "content": null, "targetDate": null })
    );
}

#[test]
fn updating_a_project_somebody_else_leads_or_nobody_is_refused_with_exit_4() {
    let sb = workspace_with_rules(&[]);
    for lead in [
        json!({ "id": BOT, "name": "Linear", "displayName": "linear",
        "email": "linear@example.com", "active": true, "isMe": false }),
        Value::Null,
    ] {
        let mock = Routed::start(update_routes(vec![(
            "ProjectView",
            vec![project_view_with(|v| v["project"]["lead"] = lead.clone())],
        )]));
        let o = run(
            &sb,
            &mock,
            &[
                "project",
                "update",
                "Fixture Project",
                "--status",
                "Completed",
            ],
        );
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert!(stderr(&o).contains("not the lead"), "{}", stderr(&o));
        assert_no_project_write(&mock);

        // Taking it over is a write to it too.
        let o = run(
            &sb,
            &mock,
            &["project", "update", "Fixture Project", "--lead", "me"],
        );
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert_no_project_write(&mock);
    }
}

#[test]
fn update_refuses_a_name_another_unfinished_project_has() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(vec![(
        "UnfinishedNamed",
        vec![unfinished(&[(OTHER_PROJECT, "Taken")])],
    )]));
    let o = run(
        &sb,
        &mock,
        &["project", "update", "Fixture Project", "--name", "Taken"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("already named"), "{}", stderr(&o));
    assert_no_project_write(&mock);

    // The project itself carrying the name is no clash.
    let mock = Routed::start(update_routes(vec![(
        "UnfinishedNamed",
        vec![unfinished(&[(PROJECT, "Taken")])],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--name",
            "Taken",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn update_stops_before_writing_on_unknown_names_or_nothing_to_change() {
    let sb = workspace_with_rules(&[]);
    let blank = write_file(&sb, "blank.md", "\n");
    let mock = Routed::start(update_routes(vec![]));
    for (args, what) in [
        (vec!["--status", "Nope"], "project status"),
        (vec!["--initiative", "Nope"], "initiative"),
        (vec!["--lead", "nobody@example.com"], "user"),
        (vec!["--body-file", blank.as_str()], "body"),
        (vec!["--target-date", "2026-13-40"], "target-date"),
        (vec![], "nothing to change"),
    ] {
        let mut full = vec!["project", "update", "Fixture Project"];
        full.extend(args.iter());
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(what), "{args:?}: {}", stderr(&o));
    }
    assert_no_project_write(&mock);

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Missing Project",
            "--status",
            "Completed",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn update_holds_a_new_body_to_the_project_template() {
    let sb = workspace_with_rules(&TEMPLATE_RULE);
    let thin = write_file(&sb, "thin.md", "## Goal\n\nOnly this.\n");
    let good = write_file(&sb, "good.md", GOOD_PROJECT_BODY);
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--body-file",
            &thin,
            "--template",
            "Project Template",
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("完了の定義"), "{}", stderr(&o));
    assert_no_project_write(&mock);

    // Changing something other than the body is not held to a template.
    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--status",
            "Completed",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--body-file",
            &good,
            "--template",
            "Project Template",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn update_replacing_a_body_without_a_template_is_refused_when_the_rule_is_on() {
    let sb = workspace_with_rules(&TEMPLATE_RULE);
    let good = write_file(&sb, "good.md", GOOD_PROJECT_BODY);
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &["project", "update", "Fixture Project", "--body-file", &good],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("--template"), "{}", stderr(&o));
    assert_no_project_write(&mock);
}

#[test]
fn update_body_is_unchecked_when_rule_operations_leave_out_project_update() {
    let sb = workspace_with_setting(
        &TEMPLATE_RULE,
        "rule_operations = { \"template-sections\" = [\"project_create\"] }",
    );
    let body = write_file(&sb, "body.md", "Whatever.\n");
    let mock = Routed::start(update_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "update",
            "Fixture Project",
            "--body-file",
            &body,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(mock.of("Templates").is_empty());
}

// ------------------------------------------------------------------ status update

fn status_routes(extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes: Vec<(&str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        (
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(ALICE))],
        ),
        ("ProjectUpdateCreate", vec![status_update_payload()]),
    ];
    override_routes(&mut routes, extra);
    routes
}

#[test]
fn status_update_writes_health_and_body() {
    let sb = workspace_with_rules(&TEMPLATE_RULE);
    let body = write_file(&sb, "update.md", "\n- Stage: 2\n- Next: wait\n\n");
    let mock = Routed::start(status_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "project",
            "status-update",
            "Fixture Project",
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
    assert_eq!(
        mock.of("ProjectUpdateCreate")[0]["input"],
        json!({ "projectId": PROJECT, "health": "atRisk", "body": "- Stage: 2\n- Next: wait" })
    );
    // The template rule does not hold a status update to a template.
    assert!(!mock.ops().contains(&"Templates".to_owned()));
}

#[test]
fn status_update_is_refused_for_a_project_somebody_else_leads() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "update.md", "Where it stands.\n");
    for lead in [Some(BOT), None] {
        let mock = Routed::start(status_routes(vec![(
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, lead)],
        )]));
        let o = run(
            &sb,
            &mock,
            &[
                "project",
                "status-update",
                "Fixture Project",
                "--health",
                "onTrack",
                "--body-file",
                &body,
            ],
        );
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert_no_project_write(&mock);
    }
}

#[test]
fn status_update_needs_a_known_health_and_a_body() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "update.md", "Where it stands.\n");
    let blank = write_file(&sb, "blank.md", " \n");
    let mock = Routed::start(status_routes(vec![]));
    for (health, file) in [("fine", body.as_str()), ("onTrack", blank.as_str())] {
        let o = run(
            &sb,
            &mock,
            &[
                "project",
                "status-update",
                "Fixture Project",
                "--health",
                health,
                "--body-file",
                file,
            ],
        );
        assert_eq!(code(&o), 2, "{health}: {}", stderr(&o));
    }
    assert!(
        mock.ops().is_empty(),
        "both were refused before anything was asked: {:?}",
        mock.ops()
    );
}

// ------------------------------------------------------------------ reorder

const P_A: &str = "00000000-0000-4000-8000-0000000000a1";
const P_B: &str = "00000000-0000-4000-8000-0000000000a2";
const P_C: &str = "00000000-0000-4000-8000-0000000000a3";

fn three_project_refs() -> Reply {
    let node = |id: &str, slug: &str, name: &str| {
        json!({ "id": id, "slugId": slug, "name": name,
            "url": format!("https://linear.app/example/project/{slug}") })
    };
    data(json!({ "projects": {
        "nodes": [node(P_A, "aaaaaaaaaaa1", "Alpha"), node(P_B, "aaaaaaaaaaa2", "Beta"), node(P_C, "aaaaaaaaaaa3", "Gamma")],
        "pageInfo": { "hasNextPage": false, "endCursor": null },
    }}))
}

fn order_of(id: &str, slug: &str, lead: Option<&str>, sort: f64, priority: f64) -> Reply {
    let lead = match lead {
        Some(id) => json!({ "id": id, "name": "Someone", "displayName": "someone",
            "email": "someone@example.com", "active": true, "isMe": id == ALICE }),
        None => Value::Null,
    };
    data(json!({ "project": {
        "id": id, "slugId": slug, "name": slug, "lead": lead,
        "sortOrder": sort, "prioritySortOrder": priority,
    }}))
}

fn reorder_routes(orders: Vec<Reply>, extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes: Vec<(&str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![three_project_refs()]),
        ("ProjectOrderQuery", orders),
        ("ProjectUpdate", vec![project_payload("projectUpdate")]),
    ];
    override_routes(&mut routes, extra);
    routes
}

#[test]
fn reorder_hands_the_held_values_out_in_the_requested_order() {
    let sb = workspace_with_rules(&[]);
    // Top to bottom they are B, C, A; the request is A, B, C.
    let mock = Routed::start(reorder_routes(
        vec![
            order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 30.0, 3.0),
            order_of(P_B, "aaaaaaaaaaa2", Some(ALICE), 10.0, 1.0),
            order_of(P_C, "aaaaaaaaaaa3", Some(ALICE), 20.0, 2.0),
        ],
        vec![],
    ));

    let o = run(
        &sb,
        &mock,
        &["project", "reorder", "Alpha", "Beta", "Gamma", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["unchanged"], false);
    assert_eq!(
        v["updated"],
        json!(["aaaaaaaaaaa1", "aaaaaaaaaaa2", "aaaaaaaaaaa3"])
    );
    assert_eq!(
        mock.of("ProjectUpdate"),
        vec![
            json!({ "id": P_A, "input": { "sortOrder": 10.0, "prioritySortOrder": 1.0 } }),
            json!({ "id": P_B, "input": { "sortOrder": 20.0, "prioritySortOrder": 2.0 } }),
            json!({ "id": P_C, "input": { "sortOrder": 30.0, "prioritySortOrder": 3.0 } }),
        ]
    );
}

#[test]
fn reorder_in_the_order_they_already_sit_writes_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(reorder_routes(
        vec![
            order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 10.0, 1.0),
            order_of(P_B, "aaaaaaaaaaa2", Some(ALICE), 20.0, 2.0),
        ],
        vec![],
    ));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha,Beta", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["unchanged"], true);
    assert_no_project_write(&mock);
}

#[test]
fn reorder_needs_every_project_to_be_mine_before_writing_any() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(reorder_routes(
        vec![
            order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 30.0, 3.0),
            order_of(P_B, "aaaaaaaaaaa2", Some(BOT), 10.0, 1.0),
        ],
        vec![],
    ));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha", "Beta"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_no_project_write(&mock);

    let mock = Routed::start(reorder_routes(
        vec![
            order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 30.0, 3.0),
            order_of(P_B, "aaaaaaaaaaa2", None, 10.0, 1.0),
        ],
        vec![],
    ));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha", "Beta"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_no_project_write(&mock);
}

#[test]
fn a_failed_reorder_step_puts_the_earlier_ones_back() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(reorder_routes(
        vec![
            order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 30.0, 3.0),
            order_of(P_B, "aaaaaaaaaaa2", Some(ALICE), 10.0, 1.0),
        ],
        // The first write goes through, the second is refused, the third (the undo) goes through.
        vec![(
            "ProjectUpdate",
            vec![
                project_payload("projectUpdate"),
                graphql_error("refused"),
                project_payload("projectUpdate"),
            ],
        )],
    ));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha", "Beta"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("rolled back: restored aaaaaaaaaaa1"),
        "{}",
        stderr(&o)
    );
    let writes = mock.of("ProjectUpdate");
    assert_eq!(writes.len(), 3);
    assert_eq!(
        writes[0]["input"],
        json!({ "sortOrder": 10.0, "prioritySortOrder": 1.0 })
    );
    assert_eq!(writes[2]["id"], P_A);
    assert_eq!(
        writes[2]["input"],
        json!({ "sortOrder": 30.0, "prioritySortOrder": 3.0 })
    );
}

#[test]
fn reorder_needs_two_different_known_projects() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(reorder_routes(
        vec![order_of(P_A, "aaaaaaaaaaa1", Some(ALICE), 30.0, 3.0)],
        vec![],
    ));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha", "Alpha"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = run(&sb, &mock, &["project", "reorder", "Alpha", "Nope"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_no_project_write(&mock);
}

#[test]
fn credentials_for_another_workspace_never_write_a_project() {
    let sb = workspace_with_rules(&[]);
    let elsewhere = WHOAMI_OK.replace("\"urlKey\":\"example\"", "\"urlKey\":\"elsewhere\"");
    let mock = Routed::start(create_routes(vec![("Whoami", vec![ok(&elsewhere)])]));
    let o = run(&sb, &mock, &["project", "create", "--name", "P"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(mock.ops(), ["Whoami"]);
}
