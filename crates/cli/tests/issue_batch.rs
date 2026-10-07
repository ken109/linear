//! `linear issue batch` against a mock Linear that answers by operation name: every item is
//! planned (names resolved, ownership, validators) before the first mutation is sent, the
//! mutations then go out in order, and a failure undoes what was sent.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const SOURCE_1: &str = "https://example.com/source/1";
const SOURCE_2: &str = "https://example.com/source/2";

/// The fixture issue EX-23: mine, in a project I lead.
fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn theirs() -> View {
    view("EX-23")
        .assigned_to(Some(BOT))
        .in_project(PROJECT, Some(BOT))
}

/// The routes of a batch that creates two issues and updates EX-23.
fn routes(view: View, extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes = create_routes(vec![(
        "IssueCreate",
        vec![
            issue_payload("issueCreate", "EX-30"),
            issue_payload("issueCreate", "EX-31"),
        ],
    )]);
    routes.push(("IssueWriteView", vec![view.reply()]));
    routes.push(("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]));
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

/// Two creations around an update that also attaches a source.
fn batch() -> Value {
    json!({ "issues": [
        { "op": "create", "title": "First", "project": "Fixture Project",
          "source": SOURCE_1, "sourceTitle": "life", "meta": { "kind": "slack", "n": 7 },
          "labels": ["api"], "priority": "high", "body": "Why.\n" },
        { "op": "update", "issue": "EX-23", "state": "done", "source": SOURCE_2 },
        { "op": "create", "title": "Second", "project": "Fixture Project" },
    ] })
}

fn file(sb: &Sandbox, batch: &Value) -> String {
    write_file(sb, "batch.json", &batch.to_string())
}

/// The names of the requests that change something, in the order they were sent.
fn mutations(mock: &Routed) -> Vec<String> {
    mock.calls()
        .into_iter()
        .filter(|c| c.mutation)
        .map(|c| c.op)
        .collect()
}

#[test]
fn a_batch_plans_every_item_then_sends_the_mutations_in_order() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);

    // Reads for all three items come before the first mutation; what can be undone goes out
    // first (in item order) and the source of an existing issue goes out last.
    let ops = mock.ops();
    let first_write = ops.iter().position(|o| o == "IssueCreate").unwrap();
    assert!(
        ops[..first_write].contains(&"IssueWriteView".to_owned()),
        "{ops:?}"
    );
    assert_eq!(
        mutations(&mock),
        [
            "IssueCreate",
            "AttachmentCreate",
            "IssueUpdate",
            "IssueCreate",
            "AttachmentCreate"
        ]
    );
    let attach = mock.of("AttachmentCreate");
    assert_eq!(
        attach[0]["input"]["issueId"], "id-EX-30",
        "the new issue's id"
    );
    assert_eq!(attach[0]["input"]["url"], SOURCE_1);
    assert_eq!(
        attach[0]["input"]["metadata"],
        json!({ "kind": "slack", "n": 7 })
    );
    assert_eq!(attach[1]["input"]["issueId"], "id-EX-23");
    assert_eq!(attach[1]["input"]["url"], SOURCE_2);

    // The variables are the ones the single commands send.
    let created = mock.of("IssueCreate");
    assert_eq!(
        created[0]["input"],
        json!({
            "teamId": "00000000-0000-4000-8000-000000000004",
            "title": "First",
            "description": "Why.",
            "assigneeId": ALICE,
            "projectId": PROJECT,
            "labelIds": ["00000000-0000-4000-8000-000000000008"],
            "priority": 2,
        })
    );
    assert_eq!(created[1]["input"]["title"], "Second");
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![
            json!({ "id": "id-EX-23", "input": { "stateId": "00000000-0000-4000-8000-000000000022" } })
        ]
    );

    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["created"], 2);
    assert_eq!(v["updated"], 1);
    let ids: Vec<&str> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["identifier"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["EX-30", "EX-23", "EX-31"]);
    assert_eq!(v["results"][0]["op"], "create");
    assert_eq!(v["results"][0]["existing"], false);
    assert_eq!(v["results"][1]["changed"], json!(["state", "source"]));
}

#[test]
fn quiet_prints_the_identifiers_and_text_one_line_per_item() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path, "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "EX-30\nEX-23\nEX-31");

    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    let text = stdout(&o);
    assert!(text.contains("1. create  EX-30  (created)"), "{text}");
    assert!(
        text.contains("2. update  EX-23  (updated state, source)"),
        "{text}"
    );
    assert!(text.contains("2 created, 1 updated"), "{text}");
}

#[test]
fn the_batch_may_come_from_standard_input() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let body = json!({ "issues": [
        { "op": "create", "title": "First", "project": "Fixture Project" }
    ] })
    .to_string();
    let o = sb.run_stdin(
        &["issue", "batch", "--file", "-", "--quiet"],
        None,
        &[
            ("LINEAR_API_URL", mock.url.as_str()),
            ("LINEAR_API_KEY_EXAMPLE", KEY),
        ],
        Some(&body),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "EX-30");
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_plans_every_mutation_and_sends_none() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(
        &sb,
        &mock,
        &["issue", "batch", "--file", &path, "--dry-run", "--json"],
    );
    let v = plan(&o, &mock);
    assert_eq!(v["command"], "issue batch");
    assert_eq!(v["target"]["kind"], "issues");
    // In the order they would be sent: the source of the existing issue last.
    assert_eq!(
        planned(&v),
        [
            "IssueCreate",
            "AttachmentCreate",
            "IssueUpdate",
            "IssueCreate",
            "AttachmentCreate"
        ]
    );
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["op"], "create");
    assert_eq!(items[0]["target"]["name"], "First");
    assert_eq!(
        items[0]["mutations"],
        json!(["IssueCreate", "AttachmentCreate"])
    );
    assert_eq!(items[1]["op"], "update");
    assert_eq!(items[1]["changed"], json!(["state", "source"]));
    // What would undo the sent mutations, newest first: the second issue, the update, the first issue.
    let undo: Vec<&str> = v["rollback"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["operation"].as_str().unwrap())
        .collect();
    assert_eq!(undo, ["IssueDelete", "IssueUpdate", "IssueDelete"]);
    assert!(mutations(&mock).is_empty());
}

#[test]
fn a_dry_run_is_refused_where_the_real_run_is() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(theirs(), vec![]));
    let o = run(
        &sb,
        &mock,
        &["issue", "batch", "--file", &path, "--dry-run"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ refusals send nothing

#[test]
fn an_item_the_ownership_rules_refuse_stops_the_whole_batch() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(theirs(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    let err = stderr(&o);
    assert!(err.contains("issues[1] (update EX-23)"), "{err}");
    assert!(
        err.contains("nothing was sent: 1 of 3 item(s) cannot be written"),
        "{err}"
    );
    // The first item, which is fine, was not sent either.
    assert!(mutations(&mock).is_empty(), "{:?}", mock.ops());
}

#[test]
fn every_refusal_is_reported_together_and_the_first_decides_the_exit_code() {
    let sb = workspace_with_rules(&["template-sections"]);
    let path = file(
        &sb,
        &json!({ "issues": [
            // No template while the rule is on: exit 5.
            { "op": "create", "title": "Bare", "project": "Fixture Project", "body": "x" },
            // Somebody else's issue: exit 4.
            { "op": "update", "issue": "EX-23", "state": "done" },
            // A project that does not exist: exit 2.
            { "op": "create", "title": "Lost", "project": "No such project" },
        ] }),
    );
    let mock = Routed::start(routes(theirs(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let err = stderr(&o);
    for item in [
        "issues[0] (create \"Bare\")",
        "issues[1] (update EX-23)",
        "issues[2] (create \"Lost\")",
    ] {
        assert!(err.contains(item), "{item}: {err}");
    }
    assert!(err.contains("nothing was sent: 3 of 3"), "{err}");
    assert!(mutations(&mock).is_empty());
}

#[test]
fn an_issue_written_twice_is_refused_before_anything_is_sent() {
    let sb = workspace_with_rules(&[]);
    let path = file(
        &sb,
        &json!({ "issues": [
            { "op": "update", "issue": "EX-23", "state": "done" },
            { "op": "update", "issue": "EX-23", "due": "2026-12-01" },
        ] }),
    );
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("both write"), "{}", stderr(&o));
    assert!(mutations(&mock).is_empty());
}

#[test]
fn a_document_that_is_wrong_is_a_usage_error_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let cases = [
        ("not json {", "the batch is not valid"),
        (r#"{"issues": []}"#, "needs at least one item"),
        (
            r#"{"issues": [{"op": "delete", "issue": "EX-1"}]}"#,
            "delete",
        ),
        (r#"{"issues": [{"op": "create", "title": "t"}]}"#, "project"),
        (
            r#"{"issues": [{"op": "create", "title": "t", "project": "p", "colour": "red"}]}"#,
            "colour",
        ),
        (
            r#"{"issues": [{"op": "update", "issue": "EX-1"}]}"#,
            "nothing to change",
        ),
        (
            r#"{"issues": [{"op": "update", "issue": "EX-1", "template": "T"}]}"#,
            "template needs a body",
        ),
        (
            r#"{"issues": [{"op": "create", "title": "t", "project": "p", "priority": 9}]}"#,
            "not a priority",
        ),
        (
            r#"{"issues": [{"op": "create", "title": "t", "project": "p", "heldOn": "2026-10-05", "cycle": 4}]}"#,
            "cannot be given together",
        ),
        (
            r#"{"issues": [
                {"op": "create", "title": "a", "project": "p", "source": "https://example.com/x"},
                {"op": "update", "issue": "EX-1", "source": "https://example.com/x"}]}"#,
            "already used by issues[0]",
        ),
        (
            r#"{"issues": [{"op": "create", "title": "t", "project": "p", "meta": {"k": 1}}]}"#,
            "need a source",
        ),
    ];
    for (text, expected) in cases {
        let path = write_file(&sb, "bad.json", text);
        let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
        assert_eq!(code(&o), 2, "{text}: {}", stderr(&o));
        assert!(stderr(&o).contains(expected), "{text}: {}", stderr(&o));
    }
    let o = run(
        &sb,
        &mock,
        &["issue", "batch", "--file", "/no/such/file.json"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let items: Vec<Value> = (0..51)
        .map(|n| json!({ "op": "create", "title": format!("t{n}"), "project": "p" }))
        .collect();
    let path = file(&sb, &json!({ "issues": items }));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("at most 50"), "{}", stderr(&o));
    assert!(
        mock.ops().is_empty(),
        "nothing was asked of Linear: {:?}",
        mock.ops()
    );
}

#[test]
fn the_validators_apply_to_every_item() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let path = file(
        &sb,
        &json!({ "issues": [
            { "op": "create", "title": "Fine", "project": "Fixture Project", "source": SOURCE_1 },
            // The rule wants a source on every creation.
            { "op": "create", "title": "Bare", "project": "Fixture Project" },
        ] }),
    );
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("issues[1] (create \"Bare\")") && err.contains("source-attachment"),
        "{err}"
    );
    assert!(!err.contains("issues[0]"), "the first item is fine: {err}");
    assert!(mutations(&mock).is_empty());

    // An issue that already has the source is returned, not made, and the batch goes on.
    let path = file(
        &sb,
        &json!({ "issues": [
            { "op": "create", "title": "Fine", "project": "Fixture Project", "source": SOURCE_1 },
        ] }),
    );
    let mock = Routed::start(routes(
        mine(),
        vec![("AttachmentsForUrlQuery", vec![issue_with_source()])],
    ));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["created"], 0);
    assert_eq!(v["results"][0]["existing"], true);
    assert_eq!(v["results"][0]["identifier"], "EX-23");
    assert!(mutations(&mock).is_empty(), "{:?}", mock.ops());
}

// ------------------------------------------------------------------ rollback

#[test]
fn a_failed_mutation_deletes_what_was_created() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(
        mine(),
        vec![("IssueUpdate", vec![graphql_error("update refused")])],
    ));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "a failed batch prints no result");
    let err = stderr(&o);
    assert!(err.contains("issues[1] (update EX-23)"), "{err}");
    assert!(err.contains("update refused"), "{err}");
    assert!(err.contains("rolled back: deleted EX-30"), "{err}");

    // The first issue and its source were made, the update failed, the first issue was deleted
    // again, and the second item was never sent.
    assert_eq!(
        mutations(&mock),
        [
            "IssueCreate",
            "AttachmentCreate",
            "IssueUpdate",
            "IssueDelete"
        ]
    );
    assert_eq!(mock.of("IssueDelete"), vec![json!({ "id": "id-EX-30" })]);
}

#[test]
fn a_source_that_cannot_be_attached_undoes_everything_before_it() {
    let sb = workspace_with_rules(&[]);
    let path = file(
        &sb,
        &json!({ "issues": [
            { "op": "create", "title": "First", "project": "Fixture Project" },
            { "op": "update", "issue": "EX-23", "state": "done", "source": SOURCE_2 },
            { "op": "create", "title": "Second", "project": "Fixture Project" },
        ] }),
    );
    // The first attachment (of an existing issue) is the last mutation; it fails every try.
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentCreate",
            vec![graphql_error("attachment refused")],
        )],
    ));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("attachment refused"), "{err}");
    assert!(
        err.contains("rolled back: deleted EX-31, undid IssueUpdate for EX-23, deleted EX-30"),
        "{err}"
    );
    // Three tries at the attachment, then the undo, newest first.
    assert_eq!(mock.of("AttachmentCreate").len(), 3);
    assert_eq!(
        mutations(&mock)[..4],
        [
            "IssueCreate",
            "IssueUpdate",
            "IssueCreate",
            "AttachmentCreate"
        ]
    );
    assert_eq!(
        mutations(&mock)[6..],
        ["IssueDelete", "IssueUpdate", "IssueDelete"]
    );
    // The update was put back to what the issue had.
    let updates = mock.of("IssueUpdate");
    assert_eq!(
        updates[1]["input"],
        json!({ "stateId": "00000000-0000-4000-8000-000000000005" })
    );
    assert_eq!(
        mock.of("IssueDelete"),
        vec![json!({ "id": "id-EX-31" }), json!({ "id": "id-EX-30" })]
    );
}

#[test]
fn a_rollback_that_fails_says_what_is_left_behind() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(
        mine(),
        vec![
            ("IssueUpdate", vec![graphql_error("update refused")]),
            ("IssueDelete", vec![graphql_error("delete refused")]),
        ],
    ));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("update refused"), "{err}");
    assert!(err.contains("COULD NOT roll back"), "{err}");
    assert!(err.contains("deleted EX-30"), "{err}");
    assert!(err.contains("delete refused"), "{err}");
}

#[test]
fn a_first_mutation_that_fails_has_nothing_to_undo() {
    let sb = workspace_with_rules(&[]);
    let path = file(&sb, &batch());
    let mock = Routed::start(routes(
        mine(),
        vec![("IssueCreate", vec![graphql_error("create refused")])],
    ));
    let o = run(&sb, &mock, &["issue", "batch", "--file", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("issues[0]") && err.contains("create refused"),
        "{err}"
    );
    assert!(!err.contains("rolled back"), "{err}");
    assert_eq!(mutations(&mock), ["IssueCreate"]);
}

// ------------------------------------------------------------------ the schema

#[test]
fn schema_prints_the_json_schema_without_a_workspace() {
    // No configuration at all: the schema needs neither a workspace nor credentials.
    let sb = Sandbox::new();
    let o = sb.run(&["issue", "batch", "--schema"], None, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let schema: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(schema["title"], "linear issue batch");
    assert_eq!(schema["required"], json!(["issues"]));
    // It is the file the repository publishes.
    let published = std::fs::read_to_string(format!(
        "{}/../../schema/issue-batch.schema.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert_eq!(stdout(&o), published);

    // --file or --schema is needed, and not both.
    let o = sb.run(&["issue", "batch"], None, &[]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = sb.run(&["issue", "batch", "--schema", "--file", "x"], None, &[]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}
