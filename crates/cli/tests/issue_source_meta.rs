//! `linear issue create --meta`: metadata on the source attachment, against a
//! mock Linear that answers by operation name.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use write_support::*;

const SOURCE: &str = "https://example.com/source/1";
/// The issue the `attachments_for_url` fixture says carries `SOURCE`.
const EXISTING_ID: &str = "00000000-0000-4000-8000-000000000003";

fn create<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "issue",
        "create",
        "--title",
        "Write the thing",
        "--project",
        "Fixture Project",
        "--source",
        SOURCE,
    ];
    args.extend_from_slice(extra);
    args
}

// ------------------------------------------------------------------ new issue

#[test]
fn meta_goes_onto_the_source_attachment_numbers_as_numbers() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![]));

    let o = run(
        &sb,
        &mock,
        &create(&[
            "--source-title",
            "life: a decision",
            "--meta",
            "kind=life-decision",
            "--meta",
            "ticket=42",
            "--meta",
            "ratio=0.5",
            "--meta",
            "zip=007",
            "--meta",
            "build=str:123",
            "--meta",
            "note=a=b",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["existing"], false);
    assert_eq!(v["metadataUpdated"], false);

    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"],
        json!({
            "issueId": "id-EX-30",
            "url": SOURCE,
            "title": "life: a decision",
            "metadata": {
                "kind": "life-decision",
                "ticket": 42,
                "ratio": 0.5,
                "zip": "007",
                "build": "123",
                "note": "a=b",
            },
        })
    );
    // Still one issue and one attachment.
    assert_eq!(mock.of("IssueCreate").len(), 1);
    assert_eq!(mock.of("AttachmentCreate").len(), 1);
}

#[test]
fn without_meta_the_attachment_is_sent_as_before() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![]));

    let o = run(&sb, &mock, &create(&["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"],
        json!({ "issueId": "id-EX-30", "url": SOURCE, "title": "Source" })
    );
}

#[test]
fn meta_works_without_the_rule_too() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(create_routes(vec![]));
    let o = run(&sb, &mock, &create(&["--meta", "kind=slack", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"]["metadata"],
        json!({ "kind": "slack" })
    );
}

#[test]
fn bad_meta_is_a_usage_error_before_any_request() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![]));
    for (bad, hint) in [
        (vec!["--meta", "kind"], "key=value"),
        (vec!["--meta", "=x"], "empty"),
        (vec!["--meta", "a=1", "--meta", "a=2"], "twice"),
    ] {
        let o = run(&sb, &mock, &create(&bad));
        assert_eq!(code(&o), 2, "{bad:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(hint), "{bad:?}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());

    // --meta belongs to a source: without one it is refused as a usage error.
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
            "--meta",
            "kind=slack",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

#[test]
fn help_explains_numbers_and_the_str_prefix() {
    let sb = workspace_with_rules(&[]);
    let o = sb.run(&["issue", "create", "--help"], None, &[]);
    assert_eq!(code(&o), 0);
    let help = stdout(&o);
    assert!(help.contains("--meta <KEY=VALUE>"), "{help}");
    assert!(help.contains("KEY=str:123"), "{help}");
}

// ------------------------------------------------------------------ same source

#[test]
fn the_same_source_with_different_meta_updates_the_attachment_and_creates_nothing() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let stored = issue_with_source_stored("Origin", Some("kept"), json!({ "kind": "slack" }));
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![stored],
    )]));

    let o = run(
        &sb,
        &mock,
        &create(&["--meta", "kind=slack", "--meta", "ticket=7", "--json"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["metadataUpdated"], true);

    // No second issue; the existing attachment is rewritten through the same
    // upsert, with its own title and subtitle kept.
    assert!(mock.of("IssueCreate").is_empty());
    assert_eq!(
        mock.of("AttachmentCreate"),
        vec![json!({ "input": {
            "issueId": EXISTING_ID,
            "url": SOURCE,
            "title": "Origin",
            "subtitle": "kept",
            "metadata": { "kind": "slack", "ticket": 7 },
        }})]
    );
    assert!(mock.of("IssueDelete").is_empty());
}

#[test]
fn an_explicit_source_title_wins_on_the_update() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![issue_with_source_stored("Origin", None, json!({}))],
    )]));
    let o = run(
        &sb,
        &mock,
        &create(&["--source-title", "New title", "--meta", "kind=slack"]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"]["title"],
        "New title"
    );
    // The text output says what happened.
    assert!(
        stdout(&o).contains("already exists, nothing created; source metadata updated"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn identical_meta_sends_nothing() {
    let sb = workspace_with_rules(&["source-attachment"]);
    // Linear hands numbers back as it stored them: 42 here, 1.0 for `ratio=1`.
    let stored = issue_with_source_stored(
        "Origin",
        None,
        json!({ "kind": "slack", "ticket": 42, "ratio": 1.0, "id": "42" }),
    );
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![stored],
    )]));

    let o = run(
        &sb,
        &mock,
        &create(&[
            "--meta",
            "kind=slack",
            "--meta",
            "ticket=42",
            "--meta",
            "ratio=1",
            "--meta",
            "id=str:42",
            "--json",
        ]),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["metadataUpdated"], false);
    mock.assert_read_only();
}

#[test]
fn a_type_change_or_a_dropped_key_counts_as_different() {
    for stored in [
        json!({ "ticket": "42" }),
        json!({ "ticket": 42, "extra": "x" }),
    ] {
        let sb = workspace_with_rules(&["source-attachment"]);
        let mock = Routed::start(create_routes(vec![(
            "AttachmentsForUrlQuery",
            vec![issue_with_source_stored("Origin", None, stored.clone())],
        )]));
        let o = run(&sb, &mock, &create(&["--meta", "ticket=42", "--json"]));
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(stdout_json(&o)["metadataUpdated"], true, "{stored}");
        assert_eq!(
            mock.of("AttachmentCreate")[0]["input"]["metadata"],
            json!({ "ticket": 42 })
        );
    }
}

#[test]
fn the_same_source_without_meta_does_not_even_read_the_attachment_again() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![issue_with_source_stored(
            "Origin",
            None,
            json!({ "kind": "slack" }),
        )],
    )]));
    let o = run(&sb, &mock, &create(&["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["metadataUpdated"], false);
    mock.assert_read_only();
    // One lookup: the idempotence check.
    assert_eq!(mock.of("AttachmentsForUrlQuery").len(), 1);
}

#[test]
fn a_failed_metadata_update_is_an_error_and_nothing_is_rolled_back() {
    let sb = workspace_with_rules(&["source-attachment"]);
    let mock = Routed::start(create_routes(vec![
        (
            "AttachmentsForUrlQuery",
            vec![issue_with_source_stored("Origin", None, json!({}))],
        ),
        ("AttachmentCreate", vec![graphql_error("refused")]),
    ]));
    let o = run(&sb, &mock, &create(&["--meta", "kind=slack"]));
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    // The existing issue is not deleted.
    assert!(mock.of("IssueDelete").is_empty());
    assert!(mock.of("IssueCreate").is_empty());
}

// ------------------------------------------------------------------ source_kinds

#[test]
fn source_kinds_require_an_allowed_kind_on_create() {
    let sb = workspace_with_source_kinds(&["life-decision", "slack"]);

    // Allowed.
    let mock = Routed::start(create_routes(vec![]));
    let o = run(&sb, &mock, &create(&["--meta", "kind=slack", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentCreate").len(), 1);

    // Not in the list, a missing kind and a missing --meta are all exit 5, and send nothing.
    for args in [
        vec!["--meta", "kind=email"],
        vec!["--meta", "ticket=1"],
        vec![],
    ] {
        let mock = Routed::start(create_routes(vec![]));
        let o = run(&sb, &mock, &create(&args));
        assert_eq!(code(&o), 5, "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("metadata.kind") && stderr(&o).contains("life-decision, slack"),
            "{}",
            stderr(&o)
        );
        mock.assert_read_only();
    }
}

#[test]
fn source_kinds_also_judge_the_meta_written_onto_an_existing_source() {
    let sb = workspace_with_source_kinds(&["slack"]);
    let mock = Routed::start(create_routes(vec![(
        "AttachmentsForUrlQuery",
        vec![issue_with_source_stored("Origin", None, json!({}))],
    )]));
    let o = run(&sb, &mock, &create(&["--meta", "kind=email"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    mock.assert_read_only();

    // Without --meta nothing is written, so nothing is judged.
    let o = run(&sb, &mock, &create(&["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["existing"], true);
}
