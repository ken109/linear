//! Which relation between two issues a command is asking about.

use linear_core::inputs::IssueRelationType::{Blocks, Duplicate, Related};
use linear_core::read::IssueWithRelations;
use linear_core::relation::matching;
use serde_json::{json, Value};

fn end(id: &str) -> Value {
    json!({ "id": id, "identifier": id.to_uppercase(), "url": format!("https://example.com/{id}") })
}

fn link(id: &str, kind: &str, from: &str, to: &str) -> Value {
    json!({ "id": id, "type": kind, "issue": end(from), "relatedIssue": end(to) })
}

/// Issue `a` with the relations that start at it and the ones that end at it.
fn issue(relations: Vec<Value>, inverse: Vec<Value>) -> IssueWithRelations {
    serde_json::from_value(json!({
        "id": "a",
        "identifier": "A",
        "relations": { "nodes": relations },
        "inverseRelations": { "nodes": inverse },
    }))
    .unwrap()
}

fn ids(found: Vec<&linear_core::read::RelationLink>) -> Vec<String> {
    found.iter().map(|r| r.id.inner().to_owned()).collect()
}

#[test]
fn a_directed_relation_matches_only_from_the_issue_towards_the_other() {
    let a = issue(
        vec![link("r1", "blocks", "a", "b")],
        vec![link("r2", "blocks", "c", "a")],
    );
    assert_eq!(ids(matching(&a, "b", Blocks)), ["r1"]);
    // `c blocks a` is another statement: `a` does not block `c`.
    assert!(matching(&a, "c", Blocks).is_empty());
    // The kind has to be the asked one.
    assert!(matching(&a, "b", Duplicate).is_empty());
    assert!(matching(&a, "b", Related).is_empty());
}

#[test]
fn a_duplicate_is_directed_too() {
    let a = issue(
        vec![link("r1", "duplicate", "a", "b")],
        vec![link("r2", "duplicate", "c", "a")],
    );
    assert_eq!(ids(matching(&a, "b", Duplicate)), ["r1"]);
    // `c` is a duplicate of `a`; that does not make `a` a duplicate of `c`.
    assert!(matching(&a, "c", Duplicate).is_empty());
}

#[test]
fn a_related_relation_matches_from_either_end() {
    let a = issue(
        vec![link("r1", "related", "a", "b")],
        vec![link("r2", "related", "c", "a")],
    );
    assert_eq!(ids(matching(&a, "b", Related)), ["r1"]);
    assert_eq!(ids(matching(&a, "c", Related)), ["r2"]);
    assert!(matching(&a, "d", Related).is_empty());
}

#[test]
fn every_match_is_returned_so_a_delete_leaves_none_behind() {
    let a = issue(
        vec![link("r1", "related", "a", "b")],
        vec![link("r2", "related", "b", "a")],
    );
    assert_eq!(ids(matching(&a, "b", Related)), ["r1", "r2"]);
}

#[test]
fn another_kind_between_the_same_two_issues_is_not_a_match() {
    let a = issue(
        vec![
            link("r1", "blocks", "a", "b"),
            link("r2", "related", "a", "b"),
        ],
        vec![],
    );
    assert_eq!(ids(matching(&a, "b", Related)), ["r2"]);
    assert_eq!(ids(matching(&a, "b", Blocks)), ["r1"]);
}
