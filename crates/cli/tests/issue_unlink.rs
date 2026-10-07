//! `issue unlink`: deletes the attachment of an issue that has a given URL.
//! Against a mock Linear that answers by operation name.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const URL: &str = "https://github.com/example/app/pull/41";
const ATTACHMENT: &str = "00000000-0000-4000-8000-0000000000a1";

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn routes(view: View, found: Reply) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![view.reply()]),
        ("AttachmentTargetsQuery", vec![found]),
        (
            "AttachmentDelete",
            vec![data(json!({ "attachmentDelete": { "success": true } }))],
        ),
    ]
}

fn linked() -> Reply {
    // Another issue carries the same URL too: only this issue's attachment may go.
    attachment_targets(&[
        ("00000000-0000-4000-8000-0000000000b2", URL, "id-EX-99"),
        (ATTACHMENT, URL, "id-EX-23"),
    ])
}

#[test]
fn unlink_deletes_this_issues_attachment_with_that_url() {
    let sb = workspace();
    let mock = Routed::start(routes(mine(), linked()));
    let o = run(
        &sb,
        &mock,
        &["issue", "unlink", "EX-23", URL, "--yes", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(v["url"], URL);
    assert_eq!(v["notLinked"], false);
    assert_eq!(v["attachmentId"], ATTACHMENT);
    assert_eq!(
        mock.of("AttachmentDelete"),
        vec![json!({ "id": ATTACHMENT })]
    );
    assert_eq!(
        mock.of("AttachmentTargetsQuery"),
        vec![json!({ "url": URL })]
    );
}

#[test]
fn unlink_prints_one_line_and_quiet_prints_the_url() {
    let sb = workspace();
    let mock = Routed::start(routes(mine(), linked()));
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL, "--yes"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), format!("EX-23  {URL}  (unlinked)"));

    let o = run(
        &sb,
        &mock,
        &["issue", "unlink", "EX-23", URL, "--yes", "--quiet"],
    );
    assert_eq!(stdout(&o).trim(), URL);
}

#[test]
fn without_yes_it_says_what_it_would_delete_and_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(routes(mine(), linked()));
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains(URL) && err.contains("EX-23"), "{err}");
    assert!(err.contains("--yes"), "{err}");
    assert!(mock.of("AttachmentDelete").is_empty());
    mock.assert_read_only();
}

#[test]
fn an_issue_without_that_attachment_is_left_alone_even_without_yes() {
    let sb = workspace();
    // The URL is on another issue only.
    let found = attachment_targets(&[(ATTACHMENT, URL, "id-EX-99")]);
    let mock = Routed::start(routes(mine(), found));
    for extra in [&[][..], &["--yes"][..]] {
        let mut args = vec!["issue", "unlink", "EX-23", URL, "--json"];
        args.extend_from_slice(extra);
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let v = stdout_json(&o);
        assert_eq!(v["notLinked"], true);
        assert_eq!(v["attachmentId"], Value::Null);
    }
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL]);
    assert_eq!(
        stdout(&o).trim(),
        format!("EX-23  {URL}  (not linked, nothing sent)")
    );
    assert!(mock.of("AttachmentDelete").is_empty());
}

#[test]
fn only_an_exact_url_match_counts() {
    let sb = workspace();
    let found = attachment_targets(&[(ATTACHMENT, &format!("{URL}/files"), "id-EX-23")]);
    let mock = Routed::start(routes(mine(), found));
    let o = run(
        &sb,
        &mock,
        &["issue", "unlink", "EX-23", URL, "--yes", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["notLinked"], true);
    assert!(mock.of("AttachmentDelete").is_empty());
}

#[test]
fn unlink_follows_ownership_like_any_change_to_the_issue() {
    let sb = workspace();
    let mock = Routed::start(routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
        linked(),
    ));
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL, "--yes"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(mock.of("AttachmentDelete").is_empty());
    assert!(!mock.ops().contains(&"AttachmentTargetsQuery".to_owned()));
}

#[test]
fn a_lenient_workspace_lets_you_unlink_from_a_colleagues_issue() {
    let sb = lenient_workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT)),
        linked(),
    ));
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL, "--yes"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("AttachmentDelete").len(), 1);
}

#[test]
fn a_blank_url_is_a_usage_error_and_nothing_is_asked() {
    let sb = workspace();
    let mock = Routed::start(routes(mine(), linked()));
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", "  ", "--yes"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}

#[test]
fn a_refusal_from_linear_is_an_error() {
    let sb = workspace();
    let mut r = routes(mine(), linked());
    r.retain(|(o, _)| *o != "AttachmentDelete");
    r.push((
        "AttachmentDelete",
        vec![data(json!({ "attachmentDelete": { "success": false } }))],
    ));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["issue", "unlink", "EX-23", URL, "--yes"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("could not delete"), "{}", stderr(&o));
}
