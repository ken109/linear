//! `linear comment update|delete` against a mock Linear that answers by operation name:
//! whose comment it is (the ownership rule), what is sent, and what is refused before anything is.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const COMMENT: &str = "00000000-0000-4000-8000-000000000017";

/// What `comment(id:)` answers: the fixture comment, written by `author` on `issue`.
fn comment_reply(author: Option<&str>, issue: Option<&str>, body: &str) -> Reply {
    let mut c: Value = serde_json::from_str::<Value>(&fixture("issue_comments")).unwrap()["data"]
        ["issue"]["comments"]["nodes"][0]
        .clone();
    c["body"] = json!(body);
    c["user"] = match author {
        Some(id) => {
            let mut u = c["user"].clone();
            u["id"] = json!(id);
            u
        }
        None => Value::Null,
    };
    c["issue"] = match issue {
        Some(identifier) => {
            json!({ "id": "id-1", "identifier": identifier, "url": "https://example.com" })
        }
        None => Value::Null,
    };
    data(json!({ "comment": c }))
}

fn mine(body: &str) -> Reply {
    comment_reply(Some(ALICE), Some("EX-23"), body)
}

fn updated_ok(body: &str) -> Reply {
    let c = serde_json::from_str::<Value>(&mine(body).body).unwrap()["data"]["comment"].clone();
    data(json!({ "commentUpdate": { "success": true, "comment": c } }))
}

fn delete_ok_reply() -> Reply {
    data(json!({ "commentDelete": { "success": true } }))
}

fn routes(comment: Reply) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("CommentQuery", vec![comment]),
        ("CommentUpdate", vec![updated_ok("new text")]),
        ("CommentDelete", vec![delete_ok_reply()]),
    ]
}

fn no_mutation(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter()
            .all(|o| o != "CommentUpdate" && o != "CommentDelete"),
        "a mutation was sent: {ops:?}"
    );
}

// ------------------------------------------------------------------ update

#[test]
fn update_replaces_the_text_with_the_trimmed_body() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "\nnew text\n\n");
    let mock = Routed::start(routes(mine("old text")));

    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file, "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["id"], COMMENT);
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(v["changed"], true);
    assert!(v["url"].as_str().unwrap().contains("#comment-"));

    assert_eq!(mock.ops(), ["Whoami", "CommentQuery", "CommentUpdate"]);
    assert_eq!(mock.of("CommentQuery"), vec![json!({ "id": COMMENT })]);
    assert_eq!(
        mock.of("CommentUpdate"),
        vec![json!({ "id": COMMENT, "input": { "body": "new text" } })]
    );
}

#[test]
fn update_to_the_same_text_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "same text\n");
    let mock = Routed::start(routes(mine("same text")));
    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file, "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], false);
    no_mutation(&mock);

    // --quiet prints the comment's URL, for scripts.
    let o = run(
        &sb,
        &mock,
        &[
            "comment",
            "update",
            COMMENT,
            "--body-file",
            &file,
            "--quiet",
        ],
    );
    assert!(stdout(&o).trim().contains("#comment-"), "{}", stdout(&o));
}

#[test]
fn an_empty_text_is_refused_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", " \n");
    let mock = Routed::start(routes(mine("old text")));
    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty());
}

#[test]
fn somebody_elses_comment_is_refused_in_a_strict_workspace() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "new text");
    for author in [Some(BOT), None] {
        let mock = Routed::start(routes(comment_reply(author, Some("EX-23"), "old text")));
        let o = run(
            &sb,
            &mock,
            &["comment", "update", COMMENT, "--body-file", &file],
        );
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert!(stderr(&o).contains("your own comments"), "{}", stderr(&o));
        no_mutation(&mock);
    }
}

#[test]
fn a_lenient_workspace_lets_me_edit_somebody_elses_comment() {
    let sb = lenient_workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "new text");
    let mock = Routed::start(routes(comment_reply(Some(BOT), Some("EX-23"), "old text")));
    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("CommentUpdate").len(), 1);
}

#[test]
fn only_a_comment_on_an_issue_can_be_changed() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "new text");
    let mock = Routed::start(routes(comment_reply(Some(ALICE), None, "old text")));
    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    no_mutation(&mock);
}

#[test]
fn an_unknown_comment_and_a_refused_update_are_errors() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "c.md", "new text");

    let mock = Routed::start(routes(graphql_error("Entity not found: Comment")));
    let o = run(
        &sb,
        &mock,
        &["comment", "update", "nope", "--body-file", &file],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    no_mutation(&mock);

    // Linear refuses the edit.
    let mut r = routes(mine("old text"));
    r.retain(|(op, _)| *op != "CommentUpdate");
    r.push(("CommentUpdate", vec![graphql_error("could not edit")]));
    let mock = Routed::start(r);
    let o = run(
        &sb,
        &mock,
        &["comment", "update", COMMENT, "--body-file", &file],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("could not edit"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ delete

#[test]
fn delete_without_yes_sends_nothing() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine("old text")));
    let o = run(&sb, &mock, &["comment", "delete", COMMENT]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--yes"), "{}", stderr(&o));
    // Not even a read: the confirmation is judged first.
    assert!(mock.ops().is_empty());
}

#[test]
fn delete_with_yes_removes_my_comment() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine("old text")));
    let o = run(
        &sb,
        &mock,
        &["comment", "delete", COMMENT, "--yes", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["deleted"], true);
    assert_eq!(v["id"], COMMENT);
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(mock.ops(), ["Whoami", "CommentQuery", "CommentDelete"]);
    assert_eq!(mock.of("CommentDelete"), vec![json!({ "id": COMMENT })]);

    let o = run(
        &sb,
        &mock,
        &["comment", "delete", COMMENT, "--yes", "--quiet"],
    );
    assert_eq!(stdout(&o).trim(), COMMENT);
}

#[test]
fn delete_is_the_authors_only_in_a_lenient_workspace_too() {
    for sb in [workspace_with_rules(&[]), lenient_workspace_with_rules(&[])] {
        let mock = Routed::start(routes(comment_reply(Some(BOT), Some("EX-23"), "old text")));
        let o = run(&sb, &mock, &["comment", "delete", COMMENT, "--yes"]);
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert!(stderr(&o).contains("delete your own"), "{}", stderr(&o));
        no_mutation(&mock);
    }
}

#[test]
fn a_comment_of_a_workspace_the_key_does_not_belong_to_never_writes() {
    // The credentials are checked before the comment is read.
    let sb = workspace_with_rules(&[]);
    let other = ok(&WHOAMI_OK.replace("\"urlKey\":\"example\"", "\"urlKey\":\"elsewhere\""));
    let mut r = routes(mine("old text"));
    r.retain(|(op, _)| *op != "Whoami");
    r.push(("Whoami", vec![other]));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["comment", "delete", COMMENT, "--yes"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    no_mutation(&mock);
    assert!(!mock.ops().contains(&"CommentQuery".to_owned()));
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_of_update_and_delete_plans_the_mutation() {
    let sb = workspace_with_rules(&[]);
    let file = write_file(&sb, "text.md", "new text\n");
    let mock = Routed::start(routes(mine("old text")));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "comment",
                "update",
                COMMENT,
                "--body-file",
                &file,
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "comment update");
    assert_eq!(v["changed"], json!(["body"]));
    assert_eq!(planned(&v), ["CommentUpdate"]);
    assert_eq!(
        v["mutations"][0]["variables"],
        json!({ "id": COMMENT, "input": { "body": "new text" } })
    );

    // The same text: nothing to send.
    let mock = Routed::start(routes(mine("new text")));
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "comment",
                "update",
                COMMENT,
                "--body-file",
                &file,
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["mutations"], json!([]));

    let mock = Routed::start(routes(mine("old text")));
    let v = plan(
        &run(
            &sb,
            &mock,
            &["comment", "delete", COMMENT, "--yes", "--dry-run", "--json"],
        ),
        &mock,
    );
    assert_eq!(planned(&v), ["CommentDelete"]);
    assert_eq!(v["mutations"][0]["variables"], json!({ "id": COMMENT }));

    // --yes is still needed (exit 2), and the author rule still holds in a lenient workspace (exit 4).
    let o = run(&sb, &mock, &["comment", "delete", COMMENT, "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let lenient = lenient_workspace_with_rules(&[]);
    let mock = Routed::start(routes(comment_reply(Some(BOT), Some("EX-23"), "old text")));
    let o = run(
        &lenient,
        &mock,
        &["comment", "delete", COMMENT, "--yes", "--dry-run"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();
}
