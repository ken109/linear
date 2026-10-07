//! `linear issue relate|unrelate` against a mock Linear that answers by operation name:
//! which relation is meant, whose issue is written, and what is sent.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

fn end(n: u32) -> Value {
    json!({ "id": format!("id-EX-{n}"), "identifier": format!("EX-{n}"),
            "url": format!("https://example.com/EX-{n}") })
}

/// A relation `from` `kind` `to`, as the relations query lists it.
fn link(id: &str, kind: &str, from: u32, to: u32) -> Value {
    json!({ "id": id, "type": kind, "issue": end(from), "relatedIssue": end(to) })
}

/// What `issue(id:)` answers for the relations query: EX-`n` with its relations.
fn relations_of(n: u32, forward: Vec<Value>, inverse: Vec<Value>) -> Reply {
    let mut issue = end(n);
    issue["relations"] = json!({ "nodes": forward });
    issue["inverseRelations"] = json!({ "nodes": inverse });
    data(json!({ "issue": issue }))
}

fn created(kind: &str, from: u32, to: u32) -> Reply {
    data(json!({ "issueRelationCreate": {
        "success": true,
        "issueRelation": {
            "id": "rel-new", "type": kind,
            "issue": {
                "id": format!("id-EX-{from}"), "identifier": format!("EX-{from}"), "title": "t",
                "url": "https://example.com", "state": { "id": "s", "name": "Backlog", "type": "backlog" }
            },
            "relatedIssue": {
                "id": format!("id-EX-{to}"), "identifier": format!("EX-{to}"), "title": "t",
                "url": "https://example.com", "state": { "id": "s", "name": "Backlog", "type": "backlog" }
            }
        }
    }}))
}

fn deleted() -> Reply {
    data(json!({ "issueRelationDelete": { "success": true } }))
}

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

/// The routes of a write on EX-23 with EX-24 as the other issue. The relations query is
/// asked twice (EX-23 first, then EX-24), so it is given both answers in that order.
fn routes(
    issue: View,
    on_23: (Vec<Value>, Vec<Value>),
    kind: &str,
) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![issue.reply()]),
        (
            "IssueRelationsQuery",
            vec![
                relations_of(23, on_23.0, on_23.1),
                relations_of(24, vec![], vec![]),
            ],
        ),
        ("IssueRelationCreate", vec![created(kind, 23, 24)]),
        ("IssueRelationDelete", vec![deleted()]),
    ]
}

fn none() -> (Vec<Value>, Vec<Value>) {
    (vec![], vec![])
}

fn mutations(mock: &Routed) -> Vec<String> {
    mock.ops()
        .into_iter()
        .filter(|o| o.starts_with("IssueRelation") && o != "IssueRelationsQuery")
        .collect()
}

// ------------------------------------------------------------------ relate

#[test]
fn relate_creates_the_relation_from_the_first_issue_to_the_second() {
    let sb = workspace_with_rules(&[]);
    for (flag, kind) in [
        ("--blocks", "blocks"),
        ("--related", "related"),
        ("--duplicate", "duplicate"),
    ] {
        let mock = Routed::start(routes(mine(), none(), kind));
        let o = run(
            &sb,
            &mock,
            &["issue", "relate", "EX-23", flag, "EX-24", "--json"],
        );
        assert_eq!(code(&o), 0, "{flag}: {}", stderr(&o));
        assert_no_leak(&o);
        let v = stdout_json(&o);
        assert_eq!(v["workspace"], "example");
        assert_eq!(v["issue"], "EX-23");
        assert_eq!(v["relatedIssue"], "EX-24");
        assert_eq!(v["type"], kind);
        assert_eq!(v["id"], "rel-new");
        assert_eq!(v["alreadyRelated"], false);

        // Reads first (the write view for ownership, then both issues' relations), then one write.
        assert_eq!(
            mock.ops(),
            [
                "Whoami",
                "IssueWriteView",
                "IssueRelationsQuery",
                "IssueRelationsQuery",
                "IssueRelationCreate"
            ]
        );
        assert_eq!(
            mock.of("IssueRelationCreate"),
            vec![json!({ "input": {
                "issueId": "id-EX-23", "relatedIssueId": "id-EX-24", "type": kind
            }})]
        );
    }
}

#[test]
fn relate_prints_a_sentence_and_the_relation_id_with_quiet() {
    let sb = workspace_with_rules(&[]);
    // A mock answers the relations query for EX-23 once, so each run gets its own.
    for (flag, extra, want) in [
        ("--blocks", None, "EX-23 blocks EX-24"),
        ("--duplicate", None, "EX-23 is a duplicate of EX-24"),
        ("--related", None, "EX-23 is related to EX-24"),
        ("--related", Some("--quiet"), "rel-new"),
    ] {
        let mock = Routed::start(routes(mine(), none(), "related"));
        let mut args = vec!["issue", "relate", "EX-23", flag, "EX-24"];
        args.extend(extra);
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(stdout(&o).trim(), want);
    }
}

#[test]
fn a_relation_that_is_already_there_sends_nothing() {
    let sb = workspace_with_rules(&[]);

    // blocks and duplicate: only in the asked direction.
    for kind in ["blocks", "duplicate"] {
        let flag = format!("--{kind}");
        let mock = Routed::start(routes(
            mine(),
            (vec![link("rel-1", kind, 23, 24)], vec![]),
            kind,
        ));
        let o = run(
            &sb,
            &mock,
            &["issue", "relate", "EX-23", &flag, "EX-24", "--json"],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let v = stdout_json(&o);
        assert_eq!(v["alreadyRelated"], true);
        assert_eq!(v["id"], "rel-1");
        assert!(mutations(&mock).is_empty(), "{:?}", mock.ops());
    }

    // related has no direction: from the other end counts too.
    let mock = Routed::start(routes(
        mine(),
        (vec![], vec![link("rel-2", "related", 24, 23)]),
        "related",
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--related", "EX-24", "--json"],
    );
    assert_eq!(stdout_json(&o)["alreadyRelated"], true);
    assert!(mutations(&mock).is_empty());

    // The reverse blocks is another statement: it is made.
    let mock = Routed::start(routes(
        mine(),
        (vec![], vec![link("rel-3", "blocks", 24, 23)]),
        "blocks",
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--blocks", "EX-24", "--json"],
    );
    assert_eq!(stdout_json(&o)["alreadyRelated"], false);
    assert_eq!(mutations(&mock), ["IssueRelationCreate"]);
}

#[test]
fn mistakes_are_usage_errors_before_any_request() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(mine(), none(), "blocks"));
    for args in [
        // The same issue, in either spelling of the case.
        vec!["issue", "relate", "EX-23", "--blocks", "ex-23"],
        vec!["issue", "relate", "EX-23", "--blocks", " "],
        // No kind, or two.
        vec!["issue", "relate", "EX-23"],
        vec![
            "issue",
            "relate",
            "EX-23",
            "--blocks",
            "EX-24",
            "--related",
            "EX-24",
        ],
        vec!["issue", "unrelate", "EX-23"],
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty());
}

#[test]
fn the_same_issue_by_another_name_is_refused_before_the_write() {
    // `EX-23` and its id are the same issue; only the relations query shows it.
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![mine().reply()]),
        (
            "IssueRelationsQuery",
            vec![relations_of(23, vec![], vec![])],
        ),
        ("IssueRelationCreate", vec![created("blocks", 23, 23)]),
    ]);
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--blocks", "id-EX-23"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mutations(&mock).is_empty());
}

#[test]
fn an_unknown_other_issue_is_an_error_and_nothing_is_written() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![mine().reply()]),
        (
            "IssueRelationsQuery",
            vec![
                relations_of(23, vec![], vec![]),
                graphql_error("Entity not found: Issue"),
            ],
        ),
        ("IssueRelationCreate", vec![created("blocks", 23, 24)]),
    ]);
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--blocks", "EX-999"],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(mutations(&mock).is_empty());
}

#[test]
fn a_refused_relation_is_an_error() {
    let sb = workspace_with_rules(&[]);
    let mut r = routes(mine(), none(), "blocks");
    r.retain(|(op, _)| *op != "IssueRelationCreate");
    r.push((
        "IssueRelationCreate",
        vec![graphql_error("relation refused")],
    ));
    let mock = Routed::start(r);
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--blocks", "EX-24"],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("relation refused"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ ownership

#[test]
fn a_relation_is_a_write_to_the_issue_it_starts_from() {
    // Strict: not mine, refused whatever the kind.
    let sb = workspace_with_rules(&[]);
    for flag in ["--blocks", "--related", "--duplicate"] {
        let mock = Routed::start(routes(theirs(), none(), "blocks"));
        let o = run(&sb, &mock, &["issue", "relate", "EX-23", flag, "EX-24"]);
        assert_eq!(code(&o), 4, "{flag}: {}", stderr(&o));
        assert!(mutations(&mock).is_empty());
        // Nothing past the ownership question was asked.
        assert!(!mock.ops().contains(&"IssueRelationsQuery".to_owned()));
    }

    // Mine by assignment, in somebody else's project: allowed; so is a project I lead.
    for issue in [
        view("EX-23")
            .assigned_to(Some(ALICE))
            .in_project(PROJECT, Some(BOT)),
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(ALICE)),
    ] {
        let mock = Routed::start(routes(issue, none(), "blocks"));
        let o = run(
            &sb,
            &mock,
            &["issue", "relate", "EX-23", "--blocks", "EX-24"],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }

    // The other issue only has to exist: it may be somebody else's.
    let mock = Routed::start(routes(mine(), none(), "blocks"));
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--blocks", "EX-24"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn a_lenient_workspace_relates_others_issues_but_a_duplicate_closes_one() {
    let sb = lenient_workspace_with_rules(&[]);

    // blocks and related are changes to the issue: allowed for anyone's issue.
    for flag in ["--blocks", "--related"] {
        let mock = Routed::start(routes(theirs(), none(), "blocks"));
        let o = run(&sb, &mock, &["issue", "relate", "EX-23", flag, "EX-24"]);
        assert_eq!(code(&o), 0, "{flag}: {}", stderr(&o));
    }

    // A duplicate puts the issue in its Duplicate state, which is canceling it: still refused.
    let mock = Routed::start(routes(theirs(), none(), "duplicate"));
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--duplicate", "EX-24"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("canceling"), "{}", stderr(&o));
    assert!(mutations(&mock).is_empty());

    // On my own issue it is allowed.
    let mock = Routed::start(routes(mine(), none(), "duplicate"));
    let o = run(
        &sb,
        &mock,
        &["issue", "relate", "EX-23", "--duplicate", "EX-24"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

// ------------------------------------------------------------------ unrelate

#[test]
fn unrelate_removes_the_asked_relation_only() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        (
            vec![
                link("rel-b", "blocks", 23, 24),
                link("rel-r", "related", 23, 24),
            ],
            vec![link("rel-x", "blocks", 24, 23)],
        ),
        "blocks",
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "unrelate", "EX-23", "--blocks", "EX-24", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(v["relatedIssue"], "EX-24");
    assert_eq!(v["type"], "blocks");
    assert_eq!(v["removed"], json!(["rel-b"]));
    // Neither the other kind nor the other direction is touched.
    assert_eq!(
        mock.of("IssueRelationDelete"),
        vec![json!({ "id": "rel-b" })]
    );
}

#[test]
fn unrelate_finds_a_related_relation_from_either_end() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        (vec![], vec![link("rel-r", "related", 24, 23)]),
        "related",
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "unrelate", "EX-23", "--related", "EX-24"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "EX-23 is related to EX-24  (removed)");
    assert_eq!(
        mock.of("IssueRelationDelete"),
        vec![json!({ "id": "rel-r" })]
    );
}

#[test]
fn unrelate_of_a_relation_that_is_not_there_sends_nothing_and_succeeds() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        mine(),
        // Only the reverse direction exists.
        (vec![], vec![link("rel-x", "blocks", 24, 23)]),
        "blocks",
    ));
    let o = run(
        &sb,
        &mock,
        &["issue", "unrelate", "EX-23", "--blocks", "EX-24", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["removed"], json!([]));
    assert!(mutations(&mock).is_empty());

    for (extra, want) in [
        (None, "EX-23 blocks EX-24  (no such relation, nothing sent)"),
        (Some("--quiet"), ""),
    ] {
        let mock = Routed::start(routes(
            mine(),
            (vec![], vec![link("rel-x", "blocks", 24, 23)]),
            "blocks",
        ));
        let mut args = vec!["issue", "unrelate", "EX-23", "--blocks", "EX-24"];
        args.extend(extra);
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(stdout(&o).trim(), want);
    }
}

#[test]
fn unrelate_needs_no_confirmation_but_follows_ownership() {
    let sb = workspace_with_rules(&[]);
    let on_23 = (vec![link("rel-b", "blocks", 23, 24)], vec![]);

    let mock = Routed::start(routes(theirs(), on_23.clone(), "blocks"));
    let o = run(
        &sb,
        &mock,
        &["issue", "unrelate", "EX-23", "--blocks", "EX-24"],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(mutations(&mock).is_empty());

    // Removing a duplicate relation puts the issue back out of Duplicate: that is not a cancel,
    // so a lenient workspace allows it on anyone's issue.
    let lenient = lenient_workspace_with_rules(&[]);
    let mock = Routed::start(routes(
        theirs(),
        (vec![link("rel-d", "duplicate", 23, 24)], vec![]),
        "duplicate",
    ));
    let o = run(
        &lenient,
        &mock,
        &["issue", "unrelate", "EX-23", "--duplicate", "EX-24"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("IssueRelationDelete").len(), 1);
}
