//! `milestone`, `initiative`, `template`, `label`, `team` and `user` against a
//! mock Linear server.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

// ------------------------------------------------------------------ milestone

#[test]
fn milestone_list_resolves_the_project_and_sorts_like_linear() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("milestones"))]);
    let o = linear(
        &sb,
        &mock,
        &["milestone", "list", "--project", "Fixture Project"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("NAME"), "{out}");
    // The response has Milestone 2 first; sortOrder puts Milestone 1 first.
    assert!(
        out.find("Milestone 1").unwrap() < out.find("Milestone 2").unwrap(),
        "{out}"
    );
    assert!(
        out.contains("2026-11-15") && out.contains("25%") && out.contains("unstarted"),
        "{out}"
    );

    assert_eq!(request(&mock, 1)["operationName"], "MilestonesOfProject");
    assert_eq!(
        request(&mock, 1)["variables"]["id"],
        "00000000-0000-4000-8000-000000000006"
    );

    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("milestones"))]);
    let o = linear(
        &sb,
        &mock,
        &["milestone", "list", "--project", "aaaaaaaaaaaa", "--json"],
    );
    let v = stdout_json(&o);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["name"], "Milestone 1");
    assert_eq!(v[0]["workspace"], "example");
    assert_eq!(v[0]["project"]["slugId"], "aaaaaaaaaaaa");
}

#[test]
fn milestone_list_requires_a_project() {
    let sb = workspace();
    let o = sb.run(&["milestone", "list"], None, &[]);
    assert_eq!(code(&o), 2);
}

#[test]
fn milestone_view_lists_its_issues() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(PROJECT_REFS),
        ok(&fixture("milestones")),
        ok(&fixture("milestone_view")),
    ]);
    let o = linear(
        &sb,
        &mock,
        &[
            "milestone",
            "view",
            "milestone 1",
            "--project",
            "aaaaaaaaaaaa",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Milestone 1",
        "First milestone",
        "Issues (2)",
        "EX-23",
        "Alice Example",
        "EX-24",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let req = request(&mock, 2);
    assert_eq!(req["operationName"], "MilestoneView");
    assert_eq!(
        req["variables"]["id"],
        "00000000-0000-4000-8000-000000000007"
    );

    let mock = Mock::start(vec![
        ok(PROJECT_REFS),
        ok(&fixture("milestones")),
        ok(&fixture("milestone_view")),
    ]);
    let o = linear(
        &sb,
        &mock,
        &[
            "milestone",
            "view",
            "Milestone 1",
            "--project",
            "aaaaaaaaaaaa",
            "--json",
        ],
    );
    let v = stdout_json(&o);
    assert_eq!(v["name"], "Milestone 1");
    assert_eq!(v["issues"]["nodes"][1]["assignee"], Value::Null);
}

#[test]
fn an_unknown_milestone_lists_the_known_ones() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(PROJECT_REFS), ok(&fixture("milestones"))]);
    let o = linear(
        &sb,
        &mock,
        &["milestone", "view", "M9", "--project", "aaaaaaaaaaaa"],
    );
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("Milestone 2, Milestone 1") || stderr(&o).contains("Milestone 1"),
        "{}",
        stderr(&o)
    );
}

// ------------------------------------------------------------------ initiative

#[test]
fn initiative_list_shows_status_owner_and_target_and_keeps_unknown_statuses() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("initiatives"))]);
    let o = linear(
        &sb,
        &mock,
        &["initiative", "list", "--status", "active", "--owner", "me"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "dddddddddddd",
        "Example Initiative",
        "Active",
        "Alice Example",
        "2026-12-31",
        "Paused",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    assert_eq!(
        request(&mock, 0)["variables"]["filter"],
        json!({"status": {"in": ["Active"]}, "owner": {"isMe": {"eq": true}}})
    );
}

#[test]
fn initiative_list_json_includes_the_description() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("initiatives"))]);
    let o = linear(&sb, &mock, &["initiative", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v[0]["description"], "What the example initiative is for.");
    assert_eq!(
        v[1]["description"],
        Value::Null,
        "an empty one is null, not missing"
    );
    // It is asked for in the list query itself, not by a request per initiative.
    assert!(request(&mock, 0)["query"]
        .as_str()
        .unwrap()
        .contains("description"));
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn initiative_view_json_has_the_description_once() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("initiatives")),
        ok(&fixture("initiative_view")),
    ]);
    let o = linear(
        &sb,
        &mock,
        &["initiative", "view", "dddddddddddd", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // A key given twice would be two values in the JSON text: count the text, not the parsed value.
    assert_eq!(
        stdout(&o).matches("\"description\"").count(),
        1,
        "{}",
        stdout(&o)
    );
    assert_eq!(
        stdout_json(&o)["description"],
        "Everything about the example."
    );
}

#[test]
fn initiative_view_resolves_by_name_and_lists_projects() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("initiatives")),
        ok(&fixture("initiative_view")),
    ]);
    let o = linear(&sb, &mock, &["initiative", "view", "example initiative"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Example Initiative  (dddddddddddd)",
        "Everything about the example.",
        "Projects (1)",
        "Fixture Project",
        "In Progress (started)",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    assert_eq!(
        request(&mock, 1)["variables"]["id"],
        "00000000-0000-4000-8000-000000000101"
    );
}

// ------------------------------------------------------------------ template

#[test]
fn template_list_leaves_the_payload_out_of_json_and_filters_by_type() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("templates_sections")),
        ok(&fixture("templates_sections")),
    ]);
    let o = linear(&sb, &mock, &["template", "list", "--json"]);
    let v = stdout_json(&o);
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert!(v[0].get("templateData").is_none());
    assert_eq!(v[1]["name"], "Sectioned Template");
    assert_eq!(v[1]["type"], "issue");
    assert_eq!(v[1]["team"]["key"], "EX");

    let o = linear(
        &sb,
        &mock,
        &["template", "list", "--type", "project", "--quiet"],
    );
    assert_eq!(stdout(&o), "A project template\n");
}

#[test]
fn template_view_prints_the_sections_read_from_linear() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("templates_sections")),
        ok(&fixture("templates_sections")),
    ]);
    let o = linear(&sb, &mock, &["template", "view", "sectioned template"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "Sections",
        "1. Background",
        "2. Acceptance criteria",
        "3. Out of scope",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    let o = linear(
        &sb,
        &mock,
        &["template", "view", "Sectioned Template", "--json"],
    );
    let v = stdout_json(&o);
    assert_eq!(
        v["sections"],
        json!(["Background", "Acceptance criteria", "Out of scope"])
    );
    assert_eq!(v["data"]["descriptionData"]["type"], "doc");
}

#[test]
fn template_skeleton_prints_only_the_headings() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("templates_sections")),
        ok(&fixture("templates_sections")),
    ]);
    let o = linear(&sb, &mock, &["template", "skeleton", "Sectioned Template"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "## Background\n\n## Acceptance criteria\n\n## Out of scope\n"
    );

    // Without a name: every issue template under its name; the project template is not one.
    let o = linear(&sb, &mock, &["template", "skeleton"]);
    let out = stdout(&o);
    assert!(
        out.contains("# Fixture Template") && out.contains("# Sectioned Template"),
        "{out}"
    );
    assert!(!out.contains("A project template"), "{out}");
    assert!(
        stderr(&o).contains("\"Fixture Template\" has no headings"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn skeleton_of_a_project_template_is_not_found_among_issue_templates() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("templates_sections"))]);
    let o = linear(&sb, &mock, &["template", "skeleton", "A project template"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("no template"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ label

#[test]
fn labels_are_grouped_and_the_group_is_shown() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("labels")),
        ok(&fixture("labels")),
        ok(&fixture("labels")),
    ]);
    let o = linear(&sb, &mock, &["label", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    let lines: Vec<&str> = text.lines().collect();
    // Linear returned api, area, Bug; the group comes before its child, then Bug.
    assert!(
        lines[1].starts_with("area ") && lines[1].contains("group"),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("area/api") && lines[2].contains("label"),
        "{lines:?}"
    );
    assert!(lines[3].starts_with("Bug"), "{lines:?}");

    let o = linear(&sb, &mock, &["label", "list", "--json"]);
    let v = stdout_json(&o);
    assert_eq!(v[1]["name"], "api");
    assert_eq!(v[1]["parent"]["name"], "area");
    assert_eq!(v[0]["isGroup"], true);

    let o = linear(&sb, &mock, &["label", "view", "area/api", "--quiet"]);
    assert_eq!(stdout(&o), "area/api\n");
}

// ------------------------------------------------------------------ team, user

#[test]
fn team_list_and_view() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("teams")),
        ok(&fixture("teams")),
        ok(&fixture("teams")),
    ]);
    let o = linear(&sb, &mock, &["team", "list"]);
    assert!(
        stdout(&o).contains("EX") && stdout(&o).contains("Example"),
        "{}",
        stdout(&o)
    );
    let o = linear(&sb, &mock, &["team", "view", "ex", "--json"]);
    let v = stdout_json(&o);
    assert_eq!(v["key"], "EX");
    assert_eq!(v["workspace"], "example");
    let o = linear(&sb, &mock, &["team", "view", "ZZ"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("available: EX"), "{}", stderr(&o));
}

#[test]
fn user_list_marks_me_and_includes_disabled_only_when_asked() {
    let sb = workspace();
    let mock = Mock::start(vec![
        ok(&fixture("users")),
        ok(&fixture("users")),
        ok(&fixture("users")),
        ok(&fixture("users")),
    ]);
    let o = linear(&sb, &mock, &["user", "list"]);
    let out = stdout(&o);
    assert!(
        out.contains("Alice Example (me)") && out.contains("alice@example.com"),
        "{out}"
    );
    assert_eq!(
        request(&mock, 0)["variables"]["includeDisabled"],
        Value::Null
    );

    let o = linear(
        &sb,
        &mock,
        &["user", "list", "--include-disabled", "--quiet"],
    );
    assert_eq!(stdout(&o), "linear@example.com\nalice@example.com\n");
    assert_eq!(request(&mock, 1)["variables"]["includeDisabled"], true);

    // `me` and an email both find Alice; a view looks through disabled users too.
    let o = linear(&sb, &mock, &["user", "view", "me", "--json"]);
    assert_eq!(stdout_json(&o)["email"], "alice@example.com");
    assert_eq!(request(&mock, 2)["variables"]["includeDisabled"], true);
    let o = linear(
        &sb,
        &mock,
        &["user", "view", "ALICE@example.com", "--quiet"],
    );
    assert_eq!(stdout(&o), "alice@example.com\n");
}

#[test]
fn skeleton_of_a_project_template_is_read_with_type_project() {
    let sb = workspace();
    let mut v: serde_json::Value = serde_json::from_str(&fixture("templates_sections")).unwrap();
    let doc = serde_json::json!({ "descriptionData": { "type": "doc", "content": [
        { "type": "heading", "attrs": { "level": 2 },
          "content": [{ "type": "text", "text": "Definition of done" }] },
    ]}});
    v["data"]["templates"][2]["templateData"] = doc.to_string().into();
    let mock = Mock::start(vec![ok(&v.to_string()), ok(&v.to_string())]);
    let o = linear(
        &sb,
        &mock,
        &[
            "template",
            "skeleton",
            "A project template",
            "--type",
            "project",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "## Definition of done\n");
    // An issue template is not found among the project templates.
    let o = linear(
        &sb,
        &mock,
        &[
            "template",
            "skeleton",
            "Sectioned Template",
            "--type",
            "project",
        ],
    );
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("no template"), "{}", stderr(&o));
}
