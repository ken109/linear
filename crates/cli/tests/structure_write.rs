//! `linear milestone create|update|delete`, `linear initiative create` and
//! `linear template create` against a mock Linear that answers by operation
//! name: the guard, the checks that are always on, the idempotent returns and
//! the mutations themselves, seen from outside.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const MILESTONE_ID: &str = "00000000-0000-4000-8000-000000000007";
const NEW_MILESTONE_ID: &str = "00000000-0000-4000-8000-0000000000a1";

/// The mutations of this file's commands; a refused or repeated write sends none of them.
const MUTATIONS: [&str; 5] = [
    "MilestoneCreate",
    "MilestoneUpdate",
    "MilestoneDelete",
    "InitiativeCreate",
    "TemplateCreate",
];

fn assert_nothing_written(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter().all(|o| !MUTATIONS.contains(&o.as_str())),
        "a mutation was sent: {ops:?}"
    );
}

// ------------------------------------------------------------------ responses

fn milestone(id: &str, name: &str, date: &str, description: Option<&str>) -> Value {
    json!({
        "id": id, "name": name, "description": description, "targetDate": date,
        "status": "unstarted", "sortOrder": 1.0, "progress": 0.0,
        "project": {
            "id": PROJECT, "slugId": "aaaaaaaaaaaa", "name": "Fixture Project",
            "url": "https://linear.app/example/project/fixture-project-aaaaaaaaaaaa"
        }
    })
}

fn milestone_payload(field: &str, m: Value) -> Reply {
    data(json!({ field: { "success": true, "projectMilestone": m } }))
}

/// What a milestone write reads first: who I am, which project, who leads it.
fn project_routes(lead: Option<&str>) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        ("ProjectOwnershipQuery", vec![ownership(PROJECT, lead)]),
    ]
}

fn with(
    mut routes: Vec<(&'static str, Vec<Reply>)>,
    extra: Vec<(&'static str, Vec<Reply>)>,
) -> Vec<(&'static str, Vec<Reply>)> {
    routes.extend(extra);
    routes
}

/// A valid create (the name is at index 5 and the date at index 7, for tests that change them).
fn create_args() -> Vec<&'static str> {
    vec![
        "milestone",
        "create",
        "--project",
        "Fixture Project",
        "--name",
        "Ship it",
        "--target-date",
        "2026-12-01",
        "--json",
    ]
}

// ------------------------------------------------------------------ milestone create

#[test]
fn a_milestone_is_created_in_a_project_the_viewer_leads() {
    let sb = workspace();
    let desc = write_file(&sb, "desc.md", "Why it matters.\n\n");
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![(
            "MilestoneCreate",
            vec![milestone_payload(
                "projectMilestoneCreate",
                milestone(
                    NEW_MILESTONE_ID,
                    "Ship it",
                    "2026-12-01",
                    Some("Why it matters."),
                ),
            )],
        )],
    ));

    let mut args = create_args();
    args.extend(["--description-file", &desc]);
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);

    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["existing"], false);
    assert_eq!(v["name"], "Ship it");
    assert_eq!(v["targetDate"], "2026-12-01");

    // Reads, then one mutation, nothing else.
    let ops = mock.ops();
    assert_eq!(
        &ops[ops.len() - 1..],
        ["MilestoneCreate"],
        "{ops:?}: the mutation is last"
    );
    assert_eq!(
        mock.of("MilestoneCreate")[0]["input"],
        json!({
            "projectId": PROJECT,
            "name": "Ship it",
            "targetDate": "2026-12-01",
            // The trailing blank lines of the file are not part of the description.
            "description": "Why it matters.",
        })
    );
}

#[test]
fn a_milestone_needs_a_target_date_before_anything_is_sent() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "create",
            "--project",
            "Fixture Project",
            "--name",
            "No date",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--target-date"), "{}", stderr(&o));

    let mut args = create_args();
    args[7] = "next friday";
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let mut args = create_args();
    args[5] = "   ";
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "nothing is read or sent");
}

#[test]
fn a_milestone_name_already_in_the_project_is_returned_not_created() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(ALICE)));
    let mut args = create_args();
    args[5] = "Milestone 1"; // the fixture project's milestone
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["id"], MILESTONE_ID);
    assert_nothing_written(&mock);

    // --quiet prints just the name, for scripts.
    args.pop();
    args.push("--quiet");
    let o = run(&sb, &mock, &args);
    assert_eq!(stdout(&o).trim(), "Milestone 1");
}

#[test]
fn a_milestone_is_refused_in_a_project_somebody_else_leads_or_nobody_does() {
    let sb = workspace();
    for lead in [Some(BOT), None] {
        let mock = Routed::start(project_routes(lead));
        let o = run(&sb, &mock, &create_args());
        assert_eq!(code(&o), 4, "{lead:?}: {}", stderr(&o));
        assert!(stderr(&o).contains("not the lead"), "{}", stderr(&o));
        assert_nothing_written(&mock);
    }
}

// ------------------------------------------------------------------ milestone update

#[test]
fn a_milestone_is_updated_by_its_name_and_only_what_differs_is_sent() {
    let sb = workspace();
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![(
            "MilestoneUpdate",
            vec![milestone_payload(
                "projectMilestoneUpdate",
                milestone(
                    MILESTONE_ID,
                    "Milestone one",
                    "2026-12-20",
                    Some("First milestone"),
                ),
            )],
        )],
    ));
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "milestone 1", // ignoring case
            "--project",
            "Fixture Project",
            "--new-name",
            "Milestone one",
            // The same as now: not sent.
            "--target-date",
            "2026-11-15",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["changed"], true);
    assert_eq!(v["name"], "Milestone one");

    let sent = mock.of("MilestoneUpdate");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["id"], MILESTONE_ID);
    assert_eq!(sent[0]["input"], json!({ "name": "Milestone one" }));
}

#[test]
fn updating_to_the_values_it_has_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "Milestone 1",
            "--project",
            "Fixture Project",
            "--target-date",
            "2026-11-15",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], false);
    assert_nothing_written(&mock);
}

#[test]
fn a_new_name_that_another_milestone_has_is_refused() {
    let sb = workspace();
    // A second milestone in the project.
    let mut project = ownership(PROJECT, Some(ALICE))
        .body
        .parse::<Value>()
        .unwrap();
    project["data"]["project"]["projectMilestones"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(milestone(NEW_MILESTONE_ID, "Taken", "2026-12-01", None));
    let mut routes = project_routes(Some(ALICE));
    routes.retain(|(op, _)| *op != "ProjectOwnershipQuery");
    routes.push(("ProjectOwnershipQuery", vec![ok(&project.to_string())]));
    let mock = Routed::start(routes);
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "Milestone 1",
            "--project",
            "Fixture Project",
            "--new-name",
            "Taken",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("already has a milestone"),
        "{}",
        stderr(&o)
    );
    assert_nothing_written(&mock);
}

#[test]
fn update_needs_something_to_change_and_a_known_milestone() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(ALICE)));
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "Milestone 1",
            "--project",
            "Fixture Project",
        ],
    );
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("nothing to change"));
    assert!(mock.ops().is_empty());

    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "No such milestone",
            "--project",
            "Fixture Project",
            "--target-date",
            "2026-12-01",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Milestone 1"),
        "the candidates are listed"
    );
    assert_nothing_written(&mock);
}

#[test]
fn updating_in_a_project_somebody_else_leads_is_refused() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(BOT)));
    let o = run(
        &sb,
        &mock,
        &[
            "milestone",
            "update",
            "Milestone 1",
            "--project",
            "Fixture Project",
            "--target-date",
            "2026-12-01",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_nothing_written(&mock);
}

// ------------------------------------------------------------------ milestone delete

fn milestone_view(with_issues: bool) -> Reply {
    let mut v: Value = serde_json::from_str(&fixture("milestone_view")).unwrap();
    if !with_issues {
        v["data"]["detail"]["issues"]["nodes"] = json!([]);
    }
    ok(&v.to_string())
}

const DELETE: [&str; 5] = [
    "milestone",
    "delete",
    "Milestone 1",
    "--project",
    "Fixture Project",
];

#[test]
fn an_empty_milestone_is_deleted() {
    let sb = workspace();
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![
            ("MilestoneView", vec![milestone_view(false)]),
            (
                "MilestoneDelete",
                vec![data(
                    json!({ "projectMilestoneDelete": { "success": true } }),
                )],
            ),
        ],
    ));
    let mut args = DELETE.to_vec();
    args.push("--json");
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["deleted"], true);
    assert_eq!(v["name"], "Milestone 1");
    assert_eq!(mock.of("MilestoneDelete")[0]["id"], MILESTONE_ID);
}

#[test]
fn a_milestone_that_still_has_issues_is_not_deleted() {
    let sb = workspace();
    let mock = Routed::start(with(
        project_routes(Some(ALICE)),
        vec![("MilestoneView", vec![milestone_view(true)])],
    ));
    let o = run(&sb, &mock, &DELETE);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("EX-23"), "{}", stderr(&o));
    assert!(stderr(&o).contains("still has"), "{}", stderr(&o));
    assert_nothing_written(&mock);
}

#[test]
fn deleting_in_a_project_somebody_else_leads_is_refused_before_looking_inside() {
    let sb = workspace();
    let mock = Routed::start(project_routes(Some(BOT)));
    let o = run(&sb, &mock, &DELETE);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(!mock.ops().contains(&"MilestoneView".to_owned()));
    assert_nothing_written(&mock);
}

// ------------------------------------------------------------------ initiative

fn initiatives(names: &[&str]) -> Reply {
    let nodes: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            json!({
                "id": format!("00000000-0000-4000-8000-0000000001{n:02}"),
                "slugId": format!("slug{n}"), "name": name,
                "url": format!("https://linear.app/example/initiative/{name}"),
                "status": "Active", "targetDate": null, "owner": null
            })
        })
        .collect();
    data(
        json!({ "initiatives": { "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } }),
    )
}

fn initiative_created(name: &str) -> Reply {
    data(
        json!({ "initiativeCreate": { "success": true, "initiative": {
            "id": "00000000-0000-4000-8000-0000000001ff", "slugId": "newslug", "name": name,
            "url": "https://linear.app/example/initiative/new", "status": "Planned",
            "targetDate": null, "owner": null
        }}}),
    )
}

#[test]
fn an_initiative_is_created_with_its_description() {
    let sb = workspace();
    let desc = write_file(&sb, "init.md", "  A long effort.  \n");
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives(&["Other"])]),
        ("InitiativeCreate", vec![initiative_created("human-sim")]),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "initiative",
            "create",
            "--name",
            " human-sim ",
            "--description-file",
            &desc,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], false);
    assert_eq!(v["slugId"], "newslug");
    assert_eq!(
        mock.of("InitiativeCreate")[0]["input"],
        json!({ "name": "human-sim", "description": "A long effort." })
    );
}

#[test]
fn an_initiative_with_the_same_name_is_returned_not_created() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives(&["Other", "human-sim"])]),
    ]);
    let o = run(
        &sb,
        &mock,
        &["initiative", "create", "--name", "human-sim", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["slugId"], "slug1");
    assert_nothing_written(&mock);
}

#[test]
fn a_blank_initiative_name_is_a_usage_error() {
    let sb = workspace();
    let mock = Routed::start(vec![]);
    let o = run(&sb, &mock, &["initiative", "create", "--name", "  "]);
    assert_eq!(code(&o), 2);
    assert!(mock.ops().is_empty());
}

#[test]
fn an_initiative_a_workspace_cannot_have_surfaces_linears_message() {
    // The free plan answers the mutation with an error; it is reported, not swallowed.
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("InitiativeList", vec![initiatives(&[])]),
        (
            "InitiativeCreate",
            vec![graphql_error(
                "Initiatives are disabled for this workspace.",
            )],
        ),
    ]);
    let o = run(&sb, &mock, &["initiative", "create", "--name", "x"]);
    assert_ne!(code(&o), 0);
    assert!(
        stderr(&o).contains("Initiatives are disabled"),
        "{}",
        stderr(&o)
    );
}

// ------------------------------------------------------------------ template

const TEMPLATE_BODY: &str =
    "## Summary\n\nWhat this is.\n\n## Steps\n\n1. one\n2. **two**\n\n- bullet\n";

fn template_created(name: &str) -> Reply {
    data(json!({ "templateCreate": { "success": true, "template": {
        "id": "00000000-0000-4000-8000-0000000000b1", "name": name, "description": "Hi",
        "type": "issue",
        "team": { "id": "00000000-0000-4000-8000-000000000004", "key": "EX", "name": "Example" },
        "templateData": "{}", "updatedAt": "2026-10-06T13:35:53.246Z"
    }}}))
}

#[test]
fn a_template_is_created_from_markdown_in_the_default_team() {
    let sb = workspace();
    let body = write_file(&sb, "tpl.md", TEMPLATE_BODY);
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("Templates", vec![templates()]),
        ("TemplateCreate", vec![template_created("Survey")]),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "template",
            "create",
            "--name",
            "Survey",
            "--body-file",
            &body,
            "--description",
            " Hi ",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], false);
    assert_eq!(v["name"], "Survey");
    assert_eq!(v["sections"], json!(["Summary", "Steps"]));

    let input = &mock.of("TemplateCreate")[0]["input"];
    assert_eq!(input["type"], "issue");
    assert_eq!(input["name"], "Survey");
    assert_eq!(input["teamId"], "00000000-0000-4000-8000-000000000004");
    assert_eq!(input["description"], "Hi");
    let doc = &input["templateData"]["descriptionData"];
    assert_eq!(doc["type"], "doc");
    assert_eq!(doc["content"][0]["type"], "heading");
    assert_eq!(doc["content"][0]["content"][0]["text"], "Summary");
    assert_eq!(input["templateData"]["title"], "");
}

#[test]
fn an_issue_template_with_the_same_name_is_returned_not_created() {
    let sb = workspace();
    let body = write_file(&sb, "tpl.md", TEMPLATE_BODY);
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("Templates", vec![templates()]),
    ]);
    let o = run(
        &sb,
        &mock,
        &[
            "template",
            "create",
            "--name",
            "Sectioned Template",
            "--body-file",
            &body,
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    // The sections shown are the existing template's, not the body that was passed.
    assert_eq!(
        v["sections"],
        json!(["Background", "Acceptance criteria", "Out of scope"])
    );
    assert_nothing_written(&mock);
}

#[test]
fn a_template_body_without_a_heading_is_refused_before_anything_is_sent() {
    let sb = workspace();
    let body = write_file(&sb, "tpl.md", "Just a sentence.\n\n- and a list\n");
    let mock = Routed::start(vec![]);
    let o = run(
        &sb,
        &mock,
        &["template", "create", "--name", "Flat", "--body-file", &body],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("no sections"), "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}

#[test]
fn the_template_body_can_come_from_standard_input() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("Templates", vec![templates()]),
        ("TemplateCreate", vec![template_created("Piped")]),
    ]);
    let o = sb.run_stdin(
        &[
            "template",
            "create",
            "--name",
            "Piped",
            "--body-file",
            "-",
            "--quiet",
        ],
        None,
        &[
            ("LINEAR_API_URL", mock.url.as_str()),
            ("LINEAR_API_KEY_EXAMPLE", KEY),
        ],
        Some("## One\n"),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "Piped");
    assert_eq!(
        mock.of("TemplateCreate")[0]["input"]["templateData"]["descriptionData"]["content"][0]
            ["content"][0]["text"],
        "One"
    );
}
