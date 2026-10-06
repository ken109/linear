//! `linear issue update --body-file|--source|--meta|--labels|--add-labels|--remove-labels`
//! against a mock Linear that answers by operation name: the same path every
//! write takes (guard, validators, mutation, rollback), and the rule that only
//! what differs from now is sent.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const SOURCE: &str = "https://example.com/source/1";
const NEW_SOURCE: &str = "https://example.com/source/2";
/// The description of the fixture issue.
const CURRENT_BODY: &str = "Body of the fixture issue.";
const API: &str = "00000000-0000-4000-8000-000000000008";
const BUG: &str = "00000000-0000-4000-8000-000000000010";
const UI: &str = "00000000-0000-4000-8000-000000000070";

/// The fixture issue (EX-23): mine, in a project I lead, with the labels `api`
/// (in the single-select group `area`) and `Bug`, and one attachment (`SOURCE`,
/// titled `Origin`, no metadata).
fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

/// The attachment of `mine()` stores this metadata.
fn with_stored_metadata(mut v: View, metadata: Value) -> View {
    v.0["issue"]["attachments"]["nodes"][0]["metadata"] = metadata;
    v
}

/// `attachmentsForURL` answering that `SOURCE` is carried by `identifier`.
fn carried_by(id: &str, identifier: &str) -> Reply {
    let mut v = serde_json::from_str::<Value>(&fixture("attachments_for_url")).unwrap();
    let issue = &mut v["data"]["attachmentsForURL"]["nodes"][0]["issue"];
    issue["id"] = json!(id);
    issue["identifier"] = json!(identifier);
    ok(&v.to_string())
}

fn routes(view: View, extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes = vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("Labels", vec![labels()]),
        ("Templates", vec![templates()]),
        // By default no other issue carries a source.
        ("AttachmentsForUrlQuery", vec![no_issue_with_source()]),
        ("IssueUpdate", vec![issue_payload("issueUpdate", "EX-23")]),
        ("AttachmentCreate", vec![attachment_ok()]),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

fn update<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["issue", "update", "EX-23"];
    args.extend_from_slice(extra);
    args
}

// ------------------------------------------------------------------ body

#[test]
fn body_file_replaces_the_description() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "A new body.\n\nWith two paragraphs.\n\n");
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(&sb, &mock, &update(&["--body-file", &body, "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["changed"], json!(["description"]));
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({
            "id": "id-EX-23",
            "input": { "description": "A new body.\n\nWith two paragraphs." },
        })]
    );
    assert!(mock.of("AttachmentCreate").is_empty());
}

#[test]
fn body_file_reads_standard_input_with_a_dash() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = sb.run_stdin(
        &update(&["--body-file", "-"]),
        None,
        &[
            ("LINEAR_API_URL", mock.url.as_str()),
            ("LINEAR_API_KEY_EXAMPLE", KEY),
        ],
        Some("From stdin.\n"),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "description": "From stdin." })
    );
}

#[test]
fn a_bullet_linear_rewrote_is_not_a_difference() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "Items:\n\n- one\n- two\n");
    let mut stored = mine();
    stored.0["issue"]["description"] = json!("Items:\n\n* one\n* two");
    let mock = Routed::start(routes(stored, vec![]));

    let o = run(&sb, &mock, &update(&["--body-file", &body, "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    mock.assert_read_only();
    assert_eq!(stdout_json(&o)["changed"], json!([]));
}

#[test]
fn a_description_that_is_already_there_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    // Trailing whitespace is not a difference.
    let body = write_file(&sb, "body.md", &format!("{CURRENT_BODY}\n\n"));
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(&sb, &mock, &update(&["--body-file", &body, "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    mock.assert_read_only();
    assert_eq!(stdout_json(&o)["changed"], json!([]));
}

#[test]
fn an_empty_body_file_is_a_usage_error_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "  \n");
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--body-file", &body]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

#[test]
fn template_sections_judge_the_new_body_when_a_template_is_named() {
    let sb = workspace_with_rules(&["template-sections"]);
    let good = write_file(&sb, "good.md", GOOD_BODY);
    let bad = write_file(&sb, "bad.md", "## Background\n\nOnly this.\n");
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(
        &sb,
        &mock,
        &update(&["--body-file", &bad, "--template", "Sectioned Template"]),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("Acceptance criteria"), "{}", stderr(&o));
    mock.assert_read_only();

    let o = run(
        &sb,
        &mock,
        &update(&["--body-file", &good, "--template", "Sectioned Template"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueUpdate").len(), 1);

    // A template that does not exist is refused too; the choices are listed.
    let o = run(
        &sb,
        &mock,
        &update(&["--body-file", &good, "--template", "Nope"]),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(mock.of("IssueUpdate").len(), 1);
}

#[test]
fn template_needs_a_body_and_is_ignored_without_the_rule() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--template", "Sectioned Template"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let body = write_file(&sb, "body.md", "Whatever.\n");
    let o = run(
        &sb,
        &mock,
        &update(&["--body-file", &body, "--template", "Sectioned Template"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--template is ignored"),
        "{}",
        stderr(&o)
    );
    assert!(mock.of("Templates").is_empty());
}

// ------------------------------------------------------------------ source

#[test]
fn a_new_source_is_attached_with_its_title_and_metadata() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(
        &sb,
        &mock,
        &update(&[
            "--source",
            NEW_SOURCE,
            "--source-title",
            "life: a decision",
            "--meta",
            "kind=life-decision",
            "--meta",
            "ticket=42",
            "--meta",
            "build=str:123",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["source"]));
    assert!(mock.of("IssueUpdate").is_empty(), "no field changed");
    assert_eq!(
        mock.of("AttachmentCreate"),
        vec![json!({ "input": {
            "issueId": "id-EX-23",
            "url": NEW_SOURCE,
            "title": "life: a decision",
            "metadata": { "kind": "life-decision", "ticket": 42, "build": "123" },
        }})]
    );
}

#[test]
fn a_new_source_without_a_title_is_called_source() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--source", NEW_SOURCE]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"],
        json!({ "issueId": "id-EX-23", "url": NEW_SOURCE, "title": "Source" })
    );
    // Without the rule nothing asks who else carries the URL.
    assert!(mock.of("AttachmentsForUrlQuery").is_empty());
}

#[test]
fn the_workspace_source_title_names_a_new_attachment_but_never_renames_one() {
    let sb = workspace_with_setting(&[], "source_title = \"出どころ\"");

    // A new attachment: the workspace's title, unless --source-title says otherwise.
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--source", NEW_SOURCE]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentCreate")[0]["input"]["title"], "出どころ");

    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(
        &sb,
        &mock,
        &update(&["--source", NEW_SOURCE, "--source-title", "Mine"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentCreate")[0]["input"]["title"], "Mine");

    // The attachment the issue already has keeps its stored title ("Origin").
    let mock = Routed::start(routes(
        with_stored_metadata(mine(), json!({ "kind": "slack" })),
        vec![],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&["--source", SOURCE, "--meta", "kind=github"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentCreate")[0]["input"]["title"], "Origin");
}

#[test]
fn a_source_the_issue_already_has_is_not_sent_again() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(routes(
        with_stored_metadata(mine(), json!({ "kind": "slack", "n": 1 })),
        vec![(
            "AttachmentsForUrlQuery",
            vec![carried_by("id-EX-23", "EX-23")],
        )],
    ));

    // Same URL, same title, metadata that is what is stored (numbers by value).
    let o = run(
        &sb,
        &mock,
        &update(&[
            "--source",
            SOURCE,
            "--source-title",
            "Origin",
            "--meta",
            "kind=slack",
            "--meta",
            "n=1.0",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    mock.assert_read_only();
    assert_eq!(stdout_json(&o)["changed"], json!([]));

    // Just the URL: nothing to write either.
    let o = run(&sb, &mock, &update(&["--source", SOURCE, "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn different_metadata_replaces_the_stored_metadata_and_keeps_the_title() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(routes(
        with_stored_metadata(mine(), json!({ "kind": "slack" })),
        vec![(
            "AttachmentsForUrlQuery",
            vec![carried_by("id-EX-23", "EX-23")],
        )],
    ));

    let o = run(
        &sb,
        &mock,
        &update(&[
            "--source",
            SOURCE,
            "--meta",
            "kind=github",
            "--meta",
            "pr=7",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["source"]));
    assert_eq!(
        mock.of("AttachmentCreate"),
        vec![json!({ "input": {
            "issueId": "id-EX-23",
            "url": SOURCE,
            "title": "Origin",
            "metadata": { "kind": "github", "pr": 7 },
        }})]
    );
}

#[test]
fn a_new_title_alone_keeps_the_stored_metadata() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        with_stored_metadata(mine(), json!({ "kind": "slack" })),
        vec![],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&["--source", SOURCE, "--source-title", "Renamed"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"],
        json!({
            "issueId": "id-EX-23",
            "url": SOURCE,
            "title": "Renamed",
            "metadata": { "kind": "slack" },
        })
    );
}

#[test]
fn the_source_rule_judges_the_url_the_owner_and_the_kind() {
    let sb = workspace_with_source_kinds(&["slack", "github"]);
    let mock = Routed::start(routes(mine(), vec![]));

    // A kind that is not allowed.
    let o = run(
        &sb,
        &mock,
        &update(&["--source", NEW_SOURCE, "--meta", "kind=email"]),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("slack, github"), "{}", stderr(&o));
    // No kind at all for a source that is new.
    let o = run(&sb, &mock, &update(&["--source", NEW_SOURCE]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    // Not a URL.
    let o = run(&sb, &mock, &update(&["--source", "ftp://example.com/x"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();

    // A URL another issue carries is not ours to take over.
    let taken = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentsForUrlQuery",
            vec![carried_by("id-EX-99", "EX-99")],
        )],
    ));
    let o = run(
        &sb,
        &taken,
        &update(&["--source", NEW_SOURCE, "--meta", "kind=slack"]),
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("already attached to EX-99"),
        "{}",
        stderr(&o)
    );
    taken.assert_read_only();

    // One this issue carries already is fine, and without metadata it is left alone.
    let own = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentsForUrlQuery",
            vec![carried_by("id-EX-23", "EX-23")],
        )],
    ));
    let o = run(&sb, &own, &update(&["--source", SOURCE]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    own.assert_read_only();
}

#[test]
fn a_source_that_is_not_http_is_a_usage_error_without_the_rule() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&["--source", "not a url"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

#[test]
fn meta_and_title_belong_to_a_source() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    for bad in [
        vec!["--meta", "kind=slack", "--state", "done"],
        vec!["--source-title", "T", "--state", "done"],
    ] {
        let o = run(&sb, &mock, &update(&bad));
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
    }
    // And the metadata must be well formed.
    for (bad, hint) in [
        (vec!["--meta", "kind"], "key=value"),
        (vec!["--meta", "a=1", "--meta", "a=2"], "twice"),
    ] {
        let mut args = vec!["--source", NEW_SOURCE];
        args.extend(bad.iter());
        let o = run(&sb, &mock, &update(&args));
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(hint), "{bad:?}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

// ------------------------------------------------------------------ labels

#[test]
fn labels_set_exactly_these_labels() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(&sb, &mock, &update(&["--labels", "Bug", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["labels"]));
    assert_eq!(
        mock.of("IssueUpdate"),
        vec![json!({ "id": "id-EX-23", "input": { "labelIds": [BUG] } })]
    );

    // Repeatable and comma-separated, and `--label` is the same flag as in `create`.
    let o = run(&sb, &mock, &update(&["--label", "api", "--label", "Bug"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // The set is what matters, not the order: this is what the issue has.
    assert_eq!(mock.of("IssueUpdate").len(), 1);
    let o = run(&sb, &mock, &update(&["--labels", "Bug,api"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueUpdate").len(), 1, "already these labels");
}

#[test]
fn labels_can_be_added_and_removed() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(&sb, &mock, &update(&["--remove-labels", "Bug"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "labelIds": [API] })
    );

    // Adding what is there, or removing what is not, changes nothing.
    let none = Routed::start(routes(mine(), vec![]));
    let o = run(
        &sb,
        &none,
        &update(&["--add-labels", "Bug", "--remove-labels", "area", "--json"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    none.assert_read_only();
    assert_eq!(stdout_json(&o)["changed"], json!([]));

    // Adding keeps the labels it has.
    let without_bug = {
        let mut v = mine();
        let nodes = v.0["issue"]["labels"]["nodes"].as_array_mut().unwrap();
        nodes.retain(|l| l["name"] != "Bug");
        v
    };
    let add = Routed::start(routes(without_bug, vec![]));
    let o = run(&sb, &add, &update(&["--add-labels", "Bug"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        add.of("IssueUpdate")[0]["input"],
        json!({ "labelIds": [API, BUG] })
    );
}

#[test]
fn two_labels_of_one_group_are_refused_before_anything_is_written() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mock = Routed::start(routes(mine(), vec![("Labels", vec![conflicting_labels()])]));

    let o = run(&sb, &mock, &update(&["--labels", "api,ui"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("area"), "{}", stderr(&o));
    // `api` is on the issue and `ui` is its sibling.
    let o = run(&sb, &mock, &update(&["--add-labels", "ui"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();

    // Taking the other one out in the same breath is fine.
    let o = run(
        &sb,
        &mock,
        &update(&["--add-labels", "ui", "--remove-labels", "api"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("IssueUpdate")[0]["input"],
        json!({ "labelIds": [BUG, UI] })
    );
}

#[test]
fn label_mistakes_are_usage_errors_before_anything_is_written() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    // A label that does not exist.
    let o = run(&sb, &mock, &update(&["--labels", "nonesuch"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
    // The replacing form and the editing forms do not mix.
    let o = run(
        &sb,
        &mock,
        &update(&["--labels", "api", "--add-labels", "Bug"]),
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}

// ------------------------------------------------------------------ guard, rollback

#[test]
fn the_new_flags_follow_ownership() {
    let sb = workspace_with_rules(&["source-attachment", "label-groups-exclusive"]);
    let body = write_file(&sb, "body.md", "A new body.\n");
    let foreign = || {
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT))
    };
    for args in [
        vec!["--body-file", body.as_str()],
        vec!["--source", NEW_SOURCE, "--meta", "kind=slack"],
        vec!["--labels", "Bug"],
        vec!["--add-labels", "Bug"],
        vec!["--remove-labels", "Bug"],
    ] {
        let mock = Routed::start(routes(foreign(), vec![]));
        let o = run(&sb, &mock, &update(&args));
        assert_eq!(code(&o), 4, "{args:?}: {}", stderr(&o));
        mock.assert_read_only();
    }
}

/// An issue of somebody else's, in somebody else's project; the team also has a Canceled state.
fn foreign_with_canceled_state() -> View {
    let mut v = view("EX-23")
        .assigned_to(Some(BOT))
        .in_project(PROJECT, Some(BOT));
    v.0["write"]["team"]["states"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "00000000-0000-4000-8000-000000000023",
            "name": "Canceled",
            "type": "canceled",
        }));
    v
}

#[test]
fn a_lenient_workspace_lets_me_change_issues_owned_by_others_but_not_cancel_them() {
    let sb = lenient_workspace_with_rules(&["source-attachment", "label-groups-exclusive"]);
    let body = write_file(&sb, "body.md", "A new body.\n");

    // Body, labels, state, assignee: all allowed on somebody else's issue.
    for args in [
        vec!["--body-file", body.as_str()],
        vec!["--labels", "Bug"],
        vec!["--state", "Done"],
        vec!["--due", "2026-12-01"],
        vec!["--assignee", "me"],
    ] {
        let mock = Routed::start(routes(foreign_with_canceled_state(), vec![]));
        let o = run(&sb, &mock, &update(&args));
        assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
        assert_eq!(mock.of("IssueUpdate").len(), 1, "{args:?}");
    }

    // A comment follows the same rule.
    let mock = Routed::start(routes(
        foreign_with_canceled_state(),
        vec![("CommentCreate", vec![comment_ok()])],
    ));
    let comment = write_file(&sb, "comment.md", "Hello.\n");
    let o = run(
        &sb,
        &mock,
        &["issue", "comment", "EX-23", "--body-file", comment.as_str()],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    // Canceling is not: exit 4, nothing written.
    let mock = Routed::start(routes(foreign_with_canceled_state(), vec![]));
    let o = run(&sb, &mock, &update(&["--state", "Canceled"]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("canceling"), "{}", stderr(&o));
    mock.assert_read_only();

    // Canceling my own issue is fine.
    let own = foreign_with_canceled_state().assigned_to(Some(ALICE));
    let mock = Routed::start(routes(own, vec![]));
    let o = run(&sb, &mock, &update(&["--state", "Canceled"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn a_strict_workspace_refuses_every_change_to_issues_owned_by_others() {
    let sb = workspace_with_rules(&[]);
    for args in [vec!["--state", "Done"], vec!["--state", "Canceled"]] {
        let mock = Routed::start(routes(foreign_with_canceled_state(), vec![]));
        let o = run(&sb, &mock, &update(&args));
        assert_eq!(code(&o), 4, "{args:?}: {}", stderr(&o));
        mock.assert_read_only();
    }
}

#[test]
fn a_source_that_cannot_be_attached_puts_the_other_fields_back() {
    let sb = workspace_with_rules(&[]);
    let body = write_file(&sb, "body.md", "A new body.\n");
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentCreate",
            vec![graphql_error("attachment refused")],
        )],
    ));

    let o = run(
        &sb,
        &mock,
        &update(&[
            "--body-file",
            &body,
            "--due",
            "2026-12-01",
            "--labels",
            "Bug",
            "--state",
            "done",
            "--source",
            NEW_SOURCE,
        ]),
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert_eq!(stdout(&o), "", "a failed update prints no result");
    let err = stderr(&o);
    assert!(err.contains("attachment refused"), "{err}");
    assert!(err.contains("rolled back: restored EX-23"), "{err}");

    // Written, tried three times to attach, then written back with the old values.
    assert_eq!(mock.of("AttachmentCreate").len(), 3);
    let writes = mock.of("IssueUpdate");
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0]["input"],
        json!({
            "description": "A new body.",
            "stateId": "00000000-0000-4000-8000-000000000022",
            "dueDate": "2026-12-01",
            "labelIds": [BUG],
        })
    );
    assert_eq!(
        writes[1]["input"],
        json!({
            "description": CURRENT_BODY,
            "stateId": "00000000-0000-4000-8000-000000000005",
            "dueDate": "2026-11-01",
            "labelIds": [API, BUG],
        })
    );
    assert_eq!(mock.ops().last().unwrap(), "IssueUpdate");
}

#[test]
fn a_rollback_that_fails_says_what_is_left_behind() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        vec![
            (
                "AttachmentCreate",
                vec![graphql_error("attachment refused")],
            ),
            (
                "IssueUpdate",
                vec![
                    issue_payload("issueUpdate", "EX-23"),
                    graphql_error("no restore for you"),
                ],
            ),
        ],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&["--due", "2026-12-01", "--source", NEW_SOURCE]),
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("COULD NOT roll back"), "{err}");
    assert!(err.contains("no restore for you"), "{err}");
}

#[test]
fn a_failed_field_update_attaches_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        vec![("IssueUpdate", vec![graphql_error("no")])],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&["--due", "2026-12-01", "--source", NEW_SOURCE]),
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(mock.of("AttachmentCreate").is_empty());
}

#[test]
fn a_flaky_attachment_is_retried() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        vec![(
            "AttachmentCreate",
            vec![graphql_error("try later"), attachment_ok()],
        )],
    ));
    let o = run(
        &sb,
        &mock,
        &update(&["--due", "2026-12-01", "--source", NEW_SOURCE, "--json"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], json!(["dueDate", "source"]));
    assert_eq!(mock.of("AttachmentCreate").len(), 2);
    assert_eq!(mock.of("IssueUpdate").len(), 1, "nothing was rolled back");
}

// ------------------------------------------------------------------ everything at once

#[test]
fn everything_at_once_is_one_field_update_and_one_attachment() {
    let sb = workspace_with_rules(&[
        "template-sections",
        "source-attachment",
        "label-groups-exclusive",
    ]);
    let body = write_file(&sb, "body.md", GOOD_BODY);
    let mock = Routed::start(routes(mine(), vec![]));

    let o = run(
        &sb,
        &mock,
        &update(&[
            "--body-file",
            &body,
            "--template",
            "Sectioned Template",
            "--labels",
            "Bug",
            "--source",
            NEW_SOURCE,
            "--meta",
            "kind=slack",
            "--state",
            "done",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout_json(&o)["changed"],
        json!(["description", "state", "labels", "source"])
    );
    assert_eq!(mock.of("IssueUpdate").len(), 1);
    assert_eq!(mock.of("AttachmentCreate").len(), 1);
    let ops = mock.ops();
    let update_at = ops.iter().position(|o| o == "IssueUpdate").unwrap();
    let attach_at = ops.iter().position(|o| o == "AttachmentCreate").unwrap();
    assert!(update_at < attach_at, "{ops:?}");
}

#[test]
fn nothing_to_change_names_the_new_flags() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), vec![]));
    let o = run(&sb, &mock, &update(&[]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    for flag in ["--body-file", "--source", "--labels"] {
        assert!(stderr(&o).contains(flag), "{flag}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty());
}
