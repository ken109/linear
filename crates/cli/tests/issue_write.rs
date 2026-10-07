//! `linear issue create|update|comment|reorder` against a mock Linear that
//! answers by operation name: the path every write takes (guard, validators,
//! mutation, rollback), checked from outside.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const ALL_RULES: [&str; 3] = [
    "template-sections",
    "source-attachment",
    "label-groups-exclusive",
];
const SOURCE: &str = "https://example.com/source/1";

/// Arguments of a create that satisfies every rule.
fn create_args<'a>(body_file: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "issue",
        "create",
        "--title",
        "Write the thing",
        "--project",
        "Fixture Project",
        "--template",
        "Sectioned Template",
        "--body-file",
        body_file,
        "--source",
        SOURCE,
    ];
    args.extend_from_slice(extra);
    args
}

// ------------------------------------------------------------------ create

#[test]
fn create_resolves_names_creates_then_attaches_the_source() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![]));

    let mut args = create_args(&body, &["--source-title", "life: a decision", "--json"]);
    args.extend(["--label", "api"]);
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);

    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["identifier"], "EX-30");
    assert_eq!(v["existing"], false);
    assert_eq!(v["sourceUrl"], SOURCE);

    // Reads first, then the creation, then the attachment; nothing else.
    let ops = mock.ops();
    let first_write = ops.iter().position(|o| o == "IssueCreate").unwrap();
    assert!(ops[..first_write].contains(&"Templates".to_owned()));
    assert!(ops[..first_write].contains(&"AttachmentsForUrlQuery".to_owned()));
    assert_eq!(&ops[first_write..], ["IssueCreate", "AttachmentCreate"]);

    assert_eq!(
        mock.of("IssueCreate")[0]["input"],
        json!({
            "teamId": "00000000-0000-4000-8000-000000000004",
            "title": "Write the thing",
            "description": GOOD_BODY.trim_end(),
            // No --assignee: the viewer.
            "assigneeId": ALICE,
            "projectId": PROJECT,
            "labelIds": ["00000000-0000-4000-8000-000000000008"],
        })
    );
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"],
        json!({ "issueId": "id-EX-30", "url": SOURCE, "title": "life: a decision" })
    );
}

#[test]
fn an_issue_with_the_same_source_is_returned_not_created() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![issue_with_source()],
    )]));

    let o = run(&sb, &mock, &create_args(&body, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["identifier"], "EX-23");
    mock.assert_read_only();
    assert_eq!(
        mock.of("AttachmentsForUrlQuery")[0]["url"],
        SOURCE,
        "the lookup is by the exact source URL"
    );

    // --quiet prints just the identifier, for scripts.
    let o = run(&sb, &mock, &create_args(&body, &["--quiet"]));
    assert_eq!(stdout(&o).trim(), "EX-23");
}

#[test]
fn a_failed_attachment_takes_the_issue_with_it() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentCreate",
        vec![graphql_error("attachment refused")],
    )]));

    let o = run(&sb, &mock, &create_args(&body, &[]));
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "a failed create prints no result");
    let err = stderr(&o);
    assert!(err.contains("attachment refused"), "{err}");
    assert!(err.contains("rolled back: deleted EX-30"), "{err}");

    // Three attempts at the attachment, then the issue is deleted.
    assert_eq!(mock.of("AttachmentCreate").len(), 3);
    assert_eq!(mock.of("IssueDelete"), vec![json!({ "id": "id-EX-30" })]);
    assert_eq!(mock.ops().last().unwrap(), "IssueDelete");
}

#[test]
fn a_rollback_that_fails_says_what_is_left_behind() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![
        (
            "AttachmentCreate",
            vec![graphql_error("attachment refused")],
        ),
        ("IssueDelete", vec![graphql_error("no delete for you")]),
    ]));

    let o = run(&sb, &mock, &create_args(&body, &[]));
    assert_eq!(code(&o), 1);
    let err = stderr(&o);
    assert!(err.contains("COULD NOT roll back"), "{err}");
    assert!(
        err.contains("deleted EX-30") && err.contains("no delete for you"),
        "{err}"
    );
}

#[test]
fn a_flaky_attachment_is_retried_and_the_issue_kept() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentCreate",
        vec![graphql_error("try later"), attachment_ok()],
    )]));

    let o = run(&sb, &mock, &create_args(&body, &["--quiet"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "EX-30");
    assert_eq!(mock.of("AttachmentCreate").len(), 2);
    assert!(mock.of("IssueDelete").is_empty());
}

#[test]
fn validator_violations_exit_5_together_and_send_no_mutation() {
    let sb = workspace_with_rules(&ALL_RULES);
    let thin = write_file(&sb, "thin.md", "## Background\n\nOnly this.\n");
    let mock = Routed::start(create_routes(vec![]));

    // A body missing two sections, and no source at all.
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
            "--template",
            "Sectioned Template",
            "--body-file",
            &thin,
        ],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("Acceptance criteria") && err.contains("Out of scope"),
        "{err}"
    );
    assert!(err.contains("a source URL is required"), "{err}");
    mock.assert_read_only();

    // --json reports it under the stable code.
    let o = run(
        &sb,
        &mock,
        &[
            "--json",
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
        ],
    );
    assert_eq!(code(&o), 5);
    let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(e["error"]["code"], "validation");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("a template is required"));
}

#[test]
fn two_labels_of_one_group_are_refused_before_anything_is_created() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![("Labels", vec![conflicting_labels()])]));

    let mut args = create_args(&body, &[]);
    args.extend(["--label", "api", "--label", "ui"]);
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("area"), "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn an_invalid_source_is_exit_5_with_the_rule_and_a_usage_error_without() {
    let body_sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&body_sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![]));
    let args = |source| {
        vec![
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
            "--template",
            "Sectioned Template",
            "--body-file",
            body.as_str(),
            "--source",
            source,
        ]
    };
    let o = run(&body_sb, &mock, &args("not a url"));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();

    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));
    let o = run(&sb, &mock, &args("not a url"));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn without_rules_the_template_and_source_are_optional() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "create",
            "--title",
            "Plain",
            "--project",
            "Fixture Project",
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // No source: no origin lookup, no attachment.
    assert_eq!(
        mock.ops()
            .iter()
            .filter(|o| o.starts_with("Attach"))
            .count(),
        0
    );
    assert_eq!(mock.of("IssueCreate")[0]["input"]["title"], "Plain");

    // A template that nothing checks is said to be ignored, not silently dropped.
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "create",
            "--title",
            "Plain",
            "--project",
            "Fixture Project",
            "--template",
            "Whatever",
        ],
    );
    assert_eq!(code(&o), 0);
    assert!(
        stderr(&o).contains("--template is ignored"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn unknown_names_stop_the_write_before_it_starts() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));
    for (flag, value, what) in [
        ("--label", "nope", "label"),
        ("--milestone", "nope", "milestone"),
        ("--assignee", "nobody@example.com", "user"),
        ("--team", "ZZ", "team"),
        ("--project", "Missing Project", "project"),
    ] {
        let mut args = vec![
            "issue",
            "create",
            "--title",
            "T",
            "--project",
            "Fixture Project",
        ];
        args.extend([flag, value]);
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{flag}: {}", stderr(&o));
        assert!(stderr(&o).contains(what), "{flag}: {}", stderr(&o));
    }
    mock.assert_read_only();
}

// ------------------------------------------------------------------ the write guard

#[test]
fn creating_in_somebody_elses_project_is_refused_with_exit_4() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let foreign = || {
        Routed::start(create_routes(vec![(
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(BOT))],
        )]))
    };

    let mock = foreign();
    let o = run(&sb, &mock, &create_args(&body, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("--allow-foreign"), "{}", stderr(&o));
    mock.assert_read_only();

    // --allow-foreign covers an issue assigned to me ...
    let mock = foreign();
    let o = run(
        &sb,
        &mock,
        &create_args(&body, &["--allow-foreign", "--quiet"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueCreate").len(), 1);

    // ... and nothing else: not an issue assigned to somebody else.
    let mock = foreign();
    let o = run(
        &sb,
        &mock,
        &create_args(
            &body,
            &["--allow-foreign", "--assignee", "linear@example.com"],
        ),
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();

    // A project nobody leads is not mine either.
    let mock = Routed::start(create_routes(vec![(
        "ProjectOwnershipQuery",
        vec![ownership(PROJECT, None)],
    )]));
    let o = run(&sb, &mock, &create_args(&body, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_lenient_workspace_lets_me_create_issues_for_others_in_any_project() {
    let sb = lenient_workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let foreign = || {
        Routed::start(create_routes(vec![(
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(BOT))],
        )]))
    };

    // Somebody else's project, an issue for somebody else, no --allow-foreign.
    let mock = foreign();
    let o = run(
        &sb,
        &mock,
        &create_args(&body, &["--assignee", "linear@example.com", "--quiet"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let input = &mock.of("IssueCreate")[0]["input"];
    assert_eq!(input["assigneeId"], BOT);
    assert_eq!(input["projectId"], PROJECT);

    // A project nobody leads, and the default assignee.
    let mock = Routed::start(create_routes(vec![(
        "ProjectOwnershipQuery",
        vec![ownership(PROJECT, None)],
    )]));
    let o = run(&sb, &mock, &create_args(&body, &["--quiet"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueCreate").len(), 1);

    // The validators still apply: a body that misses the template sections.
    let thin = write_file(&sb, "thin.md", "No sections.\n");
    let mock = foreign();
    let o = run(&sb, &mock, &create_args(&thin, &[]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn the_ownership_setting_defaults_to_strict() {
    // The same request that a lenient workspace allows is refused without the key.
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![(
        "ProjectOwnershipQuery",
        vec![ownership(PROJECT, Some(BOT))],
    )]));
    let o = run(
        &sb,
        &mock,
        &create_args(&body, &["--assignee", "linear@example.com"]),
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn credentials_for_another_workspace_never_write() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let elsewhere = WHOAMI_OK.replace("\"urlKey\":\"example\"", "\"urlKey\":\"elsewhere\"");
    let mock = Routed::start(create_routes(vec![("Whoami", vec![ok(&elsewhere)])]));

    let o = run(&sb, &mock, &create_args(&body, &[]));
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_eq!(
        mock.ops(),
        ["Whoami"],
        "nothing else is asked, let alone written"
    );
}

// ------------------------------------------------------------------ update

fn update_routes(view: View, extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes = vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("ProjectRefs", vec![two_project_refs()]),
        (
            "ProjectOwnershipQuery",
            vec![ownership(OTHER_PROJECT, Some(ALICE))],
        ),
        ("Users", vec![users()]),
        ("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

#[test]
fn update_sends_only_what_was_asked() {
    let sb = workspace_with_rules(&ALL_RULES);
    let mock = Routed::start(update_routes(mine(), vec![]));

    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--state", "done", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["workspace"], "example");
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({
            "id": "id-EX-23",
            "input": { "stateId": "00000000-0000-4000-8000-000000000022" },
        })]
    );

    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "update",
            "EX-23",
            "--due",
            "2026-12-01",
            "--assignee",
            "me",
            "--milestone",
            "milestone 1",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // The issue is already mine and already in that milestone: only the date differs.
    assert_eq!(
        mock.of("IssueUpdate")[1]["input"],
        json!({ "dueDate": "2026-12-01" })
    );

    // Assigned to somebody else and in no milestone: all three are sent.
    let mut unplanned = mine().assigned_to(Some(BOT));
    unplanned.0["issue"]["projectMilestone"] = Value::Null;
    let mock = Routed::start(update_routes(unplanned, vec![]));
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "update",
            "EX-23",
            "--due",
            "2026-12-01",
            "--assignee",
            "me",
            "--milestone",
            "milestone 1",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({
            "assigneeId": ALICE,
            "projectMilestoneId": "00000000-0000-4000-8000-000000000007",
            "dueDate": "2026-12-01",
        })
    );
}

#[test]
fn an_update_that_is_already_true_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(
        mine(),
        // "Fixture Project" is the project it is in.
        vec![(
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(ALICE))],
        )],
    ));

    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "update",
            "EX-23",
            "--state",
            "in progress",
            "--assignee",
            "me",
            "--due",
            "2026-11-01",
            "--milestone",
            "Milestone 1",
            "--project",
            "Fixture Project",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    mock.assert_read_only();
    let v = stdout_json(&o);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["changed"], json!([]));
}

#[test]
fn moving_to_another_project_drops_the_old_milestone_unless_one_is_given() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));

    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--project", "Other Project"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "projectId": OTHER_PROJECT, "projectMilestoneId": null }),
        "the old milestone is cleared with an explicit null"
    );

    // With --milestone it is looked up in the new project.
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "update",
            "EX-23",
            "--project",
            "Other Project",
            "--milestone",
            "Milestone 1",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[1]["input"],
        json!({ "projectId": OTHER_PROJECT, "projectMilestoneId": "00000000-0000-4000-8000-000000000007" })
    );
}

#[test]
fn update_mistakes_are_usage_errors_before_anything_is_written() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(update_routes(mine(), vec![]));

    // Nothing to change: not even the workspace is asked.
    let o = run(&sb, &mock, &["issue", "update", "EX-23"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("nothing to change"), "{}", stderr(&o));
    // A malformed date.
    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--due", "2026-13-45"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        mock.ops().is_empty(),
        "no request for either: {:?}",
        mock.ops()
    );

    // A state the team does not have.
    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--state", "Shipped"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Backlog"),
        "the choices are listed: {}",
        stderr(&o)
    );
    mock.assert_read_only();

    // A milestone for an issue that is in no project.
    let mock = Routed::start(update_routes(
        view("EX-23").assigned_to(Some(ALICE)).without_project(),
        vec![],
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--milestone", "Milestone 1"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn update_follows_ownership() {
    let sb = workspace_with_rules(&[]);
    let denied = |view: View, args: &[&str]| {
        let mock = Routed::start(update_routes(view, vec![]));
        let mut full = vec!["issue", "update", "EX-23"];
        full.extend_from_slice(args);
        let o = run(&sb, &mock, &full);
        (code(&o), stderr(&o), mock)
    };

    // Somebody else's issue in somebody else's project.
    let (c, err, mock) = denied(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
        &["--state", "done"],
    );
    assert_eq!(c, 4, "{err}");
    mock.assert_read_only();

    // Unassigned, in a project nobody leads.
    let (c, _, mock) = denied(
        view("EX-23").assigned_to(None).in_project(PROJECT, None),
        &["--state", "done"],
    );
    assert_eq!(c, 4);
    mock.assert_read_only();

    // Mine by assignment, in somebody else's project: allowed.
    let (c, err, _) = denied(
        view("EX-23")
            .assigned_to(Some(ALICE))
            .in_project(PROJECT, Some(BOT)),
        &["--state", "done"],
    );
    assert_eq!(c, 0, "{err}");

    // In a project I lead, assigned to somebody else: allowed.
    let (c, err, _) = denied(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(ALICE)),
        &["--state", "done"],
    );
    assert_eq!(c, 0, "{err}");

    // Moving into a project somebody else leads is refused, --allow-foreign or not.
    let mock = Routed::start(update_routes(
        mine(),
        vec![(
            "ProjectOwnershipQuery",
            vec![ownership(OTHER_PROJECT, Some(BOT))],
        )],
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "update", "EX-23", "--project", "Other Project"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ comment

fn comment_routes(view: View) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("CommentCreate", vec![comment_ok()]),
    ]
}

#[test]
fn comment_writes_the_trimmed_body() {
    let sb = workspace_with_rules(&ALL_RULES);
    let file = write_file(&sb, "comment.md", "\nDone in #12.\n\n");
    let mock = Routed::start(comment_routes(mine()));

    let o = run(
        &sb,
        &mock,
        &["issue", "comment", "EX-23", "--body-file", &file, "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["issue"], "EX-23");
    assert!(v["url"].as_str().unwrap().contains("#comment-"));
    assert_eq!(
        mock.of("CommentCreate"),
        vec![json!({ "input": { "issueId": "id-EX-23", "body": "Done in #12." } })]
    );
}

#[test]
fn an_empty_comment_is_refused_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "empty.md", " \n\n");
    let mock = Routed::start(comment_routes(mine()));
    let o = run(
        &sb,
        &mock,
        &["issue", "comment", "EX-23", "--body-file", &file],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}

#[test]
fn comment_follows_ownership_too() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "hello");
    let mock = Routed::start(comment_routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "comment", "EX-23", "--body-file", &file],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ reorder

/// Three issues in one project holding ascending values, asked for in this order.
fn three_views() -> Vec<Reply> {
    // EX-3, EX-1, EX-2: the order the command asks for them below.
    let held = |id: &str, n: f64| mine_in(id).order(n, n * 10.0).reply();
    vec![held("EX-3", 3.0), held("EX-1", 1.0), held("EX-2", 2.0)]
}

fn mine_in(id: &str) -> View {
    view(id)
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn reorder_routes(views: Vec<Reply>, updates: Vec<Reply>) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", views),
        ("IssueUpdate", updates),
    ]
}

#[test]
fn reorder_rewrites_both_orderings_in_the_requested_order() {
    let sb = workspace_with_rules(&ALL_RULES);
    let mock = Routed::start(reorder_routes(
        three_views(),
        vec![issue_payload("issueUpdate", "EX-3")],
    ));

    let o = run(
        &sb,
        &mock,
        &["issue", "reorder", "EX-3", "EX-1,EX-2", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["updated"], json!(["EX-3", "EX-1", "EX-2"]));
    assert_eq!(v["unchanged"], false);

    // The values the three held (1, 2, 3 and 10, 20, 30) go out in the new order.
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![
            json!({ "id": "id-EX-3", "input": { "sortOrder": 1.0, "prioritySortOrder": 10.0 } }),
            json!({ "id": "id-EX-1", "input": { "sortOrder": 2.0, "prioritySortOrder": 20.0 } }),
            json!({ "id": "id-EX-2", "input": { "sortOrder": 3.0, "prioritySortOrder": 30.0 } }),
        ]
    );
}

#[test]
fn reorder_that_changes_nothing_writes_nothing() {
    let sb = workspace_with_rules(&[]);
    let held = |id: &str, n: f64| mine_in(id).order(n, n * 10.0).reply();
    let mock = Routed::start(reorder_routes(
        vec![held("EX-1", 1.0), held("EX-2", 2.0)],
        vec![],
    ));

    let o = run(&sb, &mock, &["issue", "reorder", "EX-1", "EX-2", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["unchanged"], true);
    assert_eq!(v["updated"], json!([]));
    mock.assert_read_only();
}

#[test]
fn a_reorder_that_fails_part_way_puts_the_old_values_back() {
    let sb = workspace_with_rules(&[]);
    // The first write succeeds, the second fails, the restore succeeds.
    let mock = Routed::start(reorder_routes(
        three_views(),
        vec![
            issue_payload("issueUpdate", "EX-3"),
            graphql_error("refused"),
            issue_payload("issueUpdate", "EX-3"),
        ],
    ));

    let o = run(&sb, &mock, &["issue", "reorder", "EX-3", "EX-1", "EX-2"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("rolled back: restored EX-3"),
        "{}",
        stderr(&o)
    );
    let updates = mock.of("IssueUpdate");
    assert_eq!(updates.len(), 3);
    // EX-3 held 3 and 30 before the reorder; that is what the restore writes.
    assert_eq!(
        updates[2],
        json!({ "id": "id-EX-3", "input": { "sortOrder": 3.0, "prioritySortOrder": 30.0 } })
    );
}

#[test]
fn reorder_needs_one_project_and_ownership_of_every_issue() {
    let sb = workspace_with_rules(&[]);
    let held = |v: View| v.order(1.0, 10.0).reply();

    // Different projects: their values cannot be compared.
    let mock = Routed::start(reorder_routes(
        vec![
            held(mine_in("EX-1")),
            held(
                view("EX-2")
                    .assigned_to(Some(ALICE))
                    .in_project(OTHER_PROJECT, Some(ALICE)),
            ),
        ],
        vec![],
    ));
    let o = run(&sb, &mock, &["issue", "reorder", "EX-1", "EX-2"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();

    // One of them is not mine: nothing is written, not even the ones that are.
    let mock = Routed::start(reorder_routes(
        vec![
            held(mine_in("EX-1")),
            held(
                view("EX-2")
                    .assigned_to(Some(BOT))
                    .in_project(PROJECT, Some(BOT)),
            ),
        ],
        vec![],
    ));
    let o = run(&sb, &mock, &["issue", "reorder", "EX-1", "EX-2"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

/// The three issues of `three_views`, in the order they are asked for.
fn three_writes() -> Vec<Value> {
    vec![
        json!({ "id": "id-EX-3", "input": { "sortOrder": 1.0, "prioritySortOrder": 10.0 } }),
        json!({ "id": "id-EX-1", "input": { "sortOrder": 2.0, "prioritySortOrder": 20.0 } }),
        json!({ "id": "id-EX-2", "input": { "sortOrder": 3.0, "prioritySortOrder": 30.0 } }),
    ]
}

#[test]
fn reorder_reads_one_comma_separated_list_the_same_as_separate_arguments() {
    let sb = workspace_with_rules(&[]);
    for args in [
        // One comma-separated value (clap counts values before it splits them).
        vec!["EX-3,EX-1,EX-2"],
        // Separate arguments.
        vec!["EX-3", "EX-1", "EX-2"],
        // Mixed.
        vec!["EX-3", "EX-1,EX-2"],
        vec!["EX-3,EX-1", "EX-2"],
        // Stray spaces and a trailing comma are not issues.
        vec!["EX-3, EX-1 ,EX-2,"],
    ] {
        let mock = Routed::start(reorder_routes(
            three_views(),
            vec![issue_payload("issueUpdate", "EX-3")],
        ));
        let mut full = vec!["issue", "reorder"];
        full.extend(&args);
        full.push("--json");
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
        assert_eq!(
            stdout_json(&o)["updated"],
            json!(["EX-3", "EX-1", "EX-2"]),
            "{args:?}"
        );
        assert_eq!(mock.of("IssueUpdate"), three_writes(), "{args:?}");
    }
}

#[test]
fn reorder_of_a_single_issue_is_a_usage_error_before_any_request() {
    let sb = workspace_with_rules(&[]);
    for args in [vec!["EX-1"], vec!["EX-1,"], vec!["EX-1", " "], vec![","]] {
        let mock = Routed::start(reorder_routes(three_views(), vec![]));
        let mut full = vec!["issue", "reorder"];
        full.extend(&args);
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("at least two issues"),
            "{args:?}: {}",
            stderr(&o)
        );
        assert!(mock.ops().is_empty(), "{args:?}: {:?}", mock.ops());
    }

    // No issue at all is refused by clap itself.
    let mock = Routed::start(reorder_routes(three_views(), vec![]));
    let o = run(&sb, &mock, &["issue", "reorder"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_of_create_plans_the_issue_the_source_and_the_undo() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![]));

    let mut args = create_args(
        &body,
        &["--source-title", "life: a decision", "--label", "api"],
    );
    args.extend(["--dry-run", "--json"]);
    let v = plan(&run(&sb, &mock, &args), &mock);

    assert_eq!(v["command"], "issue create");
    assert_eq!(
        v["target"],
        json!({ "kind": "issue", "name": "Write the thing", "id": null, "new": true })
    );
    assert_eq!(planned(&v), ["IssueCreate", "AttachmentCreate"]);
    // The input is the one the real run sends.
    assert_eq!(
        v["mutations"][0]["variables"]["input"],
        json!({
            "teamId": "00000000-0000-4000-8000-000000000004",
            "title": "Write the thing",
            "description": GOOD_BODY.trim_end(),
            "assigneeId": ALICE,
            "projectId": PROJECT,
            "labelIds": ["00000000-0000-4000-8000-000000000008"],
        })
    );
    // What only exists once the issue does is a placeholder.
    let attach = &v["mutations"][1]["variables"]["input"];
    assert_eq!(attach["url"], SOURCE);
    assert_eq!(attach["title"], "life: a decision");
    assert!(
        attach["issueId"].as_str().unwrap().starts_with('<'),
        "{attach}"
    );
    assert_eq!(v["rollback"][0]["operation"], "IssueDelete");
    assert_eq!(v["rollback"][0]["variables"]["id"], attach["issueId"]);
}

#[test]
fn a_dry_run_of_create_returns_the_issue_that_has_the_source_and_sends_nothing() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![issue_with_source()],
    )]));

    let args = create_args(&body, &["--dry-run", "--json"]);
    let v = plan(&run(&sb, &mock, &args), &mock);
    assert_eq!(v["target"]["name"], "EX-23");
    assert_eq!(v["target"]["new"], false);
    assert_eq!(v["mutations"], json!([]));
    assert!(
        v["reason"].as_str().unwrap().contains("already exists"),
        "{v}"
    );
}

#[test]
fn a_dry_run_of_create_fails_like_the_real_run() {
    let sb = workspace_with_rules(&ALL_RULES);
    let body = write_file(&sb, "body.md", GOOD_BODY);

    // Exit 5: the body does not fill the template's sections.
    let thin = write_file(&sb, "thin.md", "## Background\n\nOnly this.\n");
    let mock = Routed::start(create_routes(vec![]));
    let o = run(&sb, &mock, &create_args(&thin, &["--dry-run"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    mock.assert_read_only();

    // Exit 4: a project somebody else leads (and --allow-foreign is not given).
    let mock = Routed::start(create_routes(vec![(
        "ProjectOwnershipQuery",
        vec![ownership(PROJECT, Some(BOT))],
    )]));
    let o = run(&sb, &mock, &create_args(&body, &["--dry-run"]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();

    // Exit 2: a name that matches nothing.
    let mock = Routed::start(create_routes(vec![]));
    let mut args = create_args(&body, &["--dry-run"]);
    let at = args.iter().position(|a| *a == "Fixture Project").unwrap();
    args[at] = "No such project";
    let o = run(&sb, &mock, &args);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_dry_run_of_comment_plans_the_comment() {
    let sb = workspace_with_rules(&ALL_RULES);
    let file = write_file(&sb, "comment.md", "\nDone in #12.\n\n");
    let mock = Routed::start(comment_routes(mine()));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "issue",
                "comment",
                "EX-23",
                "--body-file",
                &file,
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "issue comment");
    assert_eq!(planned(&v), ["CommentCreate"]);
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "input": { "issueId": "id-EX-23", "body": "Done in #12." } })
    );

    // Ownership applies: somebody else's issue is refused with exit 4.
    let theirs = view("EX-23")
        .assigned_to(Some(BOT))
        .in_project(PROJECT, Some(BOT));
    let mock = Routed::start(comment_routes(theirs));
    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "comment",
            "EX-23",
            "--body-file",
            &file,
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn a_dry_run_of_reorder_plans_each_write_and_its_undo() {
    let sb = workspace_with_rules(&ALL_RULES);
    let mock = Routed::start(reorder_routes(three_views(), vec![]));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "issue",
                "reorder",
                "EX-3",
                "EX-1,EX-2",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "issue reorder");
    assert_eq!(planned(&v), ["IssueUpdate", "IssueUpdate", "IssueUpdate"]);
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "id": "id-EX-3", "input": { "sortOrder": 1.0, "prioritySortOrder": 10.0 } })
    );
    assert_eq!(v["rollback"].as_array().unwrap().len(), 3);

    // Already in order: nothing to send.
    let held = |id: &str, n: f64| mine_in(id).order(n, n * 10.0).reply();
    let mock = Routed::start(reorder_routes(
        vec![held("EX-1", 1.0), held("EX-2", 2.0)],
        vec![],
    ));
    let v = plan(
        &run(
            &sb,
            &mock,
            &["issue", "reorder", "EX-1", "EX-2", "--dry-run", "--json"],
        ),
        &mock,
    );
    assert_eq!(v["mutations"], json!([]));

    // A single issue is still a usage error.
    let o = run(&sb, &mock, &["issue", "reorder", "EX-1", "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}
