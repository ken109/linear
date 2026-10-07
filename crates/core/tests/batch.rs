//! The input of `issue batch`: what it accepts, what it refuses on its own, and the published
//! JSON Schema.

use linear_core::batch::{self, Item, MAX_ITEMS};
use serde_json::{json, Value};

fn parse(v: Value) -> linear_core::batch::Batch {
    batch::parse(&v.to_string()).unwrap_or_else(|e| panic!("{e}"))
}

fn problems(v: Value) -> Vec<String> {
    parse(v).problems()
}

#[test]
fn a_batch_of_creates_and_updates_reads() {
    let b = parse(json!({ "issues": [
        { "op": "create", "title": "T", "project": "P", "labels": ["a", "b"], "priority": "urgent",
          "estimate": 3, "heldOn": "2026-10-05", "meta": { "kind": "slack", "n": 1 }, "source": "https://x.test/a" },
        { "op": "update", "issue": "KK-1", "estimate": "none", "cycle": 4, "addLabels": ["c"],
          "priority": 0, "parent": "none", "due": "2026-12-01" },
    ] }));
    assert_eq!(b.issues.len(), 2);
    assert!(b.problems().is_empty(), "{:?}", b.problems());
    let Item::Create(c) = &b.issues[0] else {
        panic!()
    };
    assert_eq!(c.priority.as_ref().unwrap().number(), Ok(1));
    assert_eq!(c.labels, ["a", "b"]);
    let meta = batch::metadata(&c.meta).unwrap().unwrap();
    assert_eq!(meta.get_str("kind"), Some("slack"));
    let Item::Update(u) = &b.issues[1] else {
        panic!()
    };
    assert_eq!(u.estimate.as_ref().unwrap().value(), Ok(None));
    assert_eq!(u.cycle.as_ref().unwrap().value(), Ok(Some(4)));
    assert_eq!(u.priority.as_ref().unwrap().number(), Ok(0));
}

#[test]
fn a_document_of_the_wrong_shape_is_a_usage_error() {
    for text in [
        "",
        "[]",
        r#"{"issues": {}}"#,
        r#"{"issues": [], "extra": 1}"#,
        r#"{"issues": [{"title": "no op", "project": "p"}]}"#,
        r#"{"issues": [{"op": "move", "issue": "KK-1"}]}"#,
        r#"{"issues": [{"op": "create", "title": "t", "project": "p", "typo": 1}]}"#,
        r#"{"issues": [{"op": "update", "issue": "KK-1", "labels": "bug"}]}"#,
        r#"{"issues": [{"op": "create", "title": "t", "project": "p", "heldOn": "2026-02-30"}]}"#,
        r#"{"issues": [{"op": "create", "title": "t", "project": "p", "meta": {"k": [1]}}]}"#,
    ] {
        let e = batch::parse(text).unwrap_err();
        assert_eq!(e.code().exit_code(), 2, "{text}: {e}");
        assert!(
            e.to_string().contains("issue batch --schema"),
            "{text}: {e}"
        );
    }
}

#[test]
fn the_document_alone_decides_these() {
    let item = |extra: Value| {
        let mut v = json!({ "op": "update", "issue": "KK-1" });
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let one = |item: Value| problems(json!({ "issues": [item] }));

    assert_eq!(problems(json!({ "issues": [] })).len(), 1);
    let many: Vec<Value> = (0..=MAX_ITEMS)
        .map(|n| json!({ "op": "create", "title": format!("t{n}"), "project": "p" }))
        .collect();
    assert!(problems(json!({ "issues": many }))[0].contains("at most"));

    assert!(one(item(json!({})))
        .iter()
        .any(|p| p.contains("nothing to change")));
    assert!(one(item(json!({ "template": "T" })))
        .iter()
        .any(|p| p.contains("needs a body")));
    assert!(one(item(json!({ "labels": [] })))
        .iter()
        .any(|p| p.contains("labels is empty")));
    assert!(one(item(json!({ "labels": ["a"], "addLabels": ["b"] })))
        .iter()
        .any(|p| p.contains("cannot be combined")));
    assert!(one(item(json!({ "priority": "soon" })))
        .iter()
        .any(|p| p.contains("not a priority")));
    assert!(one(item(json!({ "estimate": "lots" })))
        .iter()
        .any(|p| p.contains("lots")));
    assert!(one(item(json!({ "cycle": "later" })))
        .iter()
        .any(|p| p.contains("later")));
    assert!(one(item(json!({ "body": "  " })))
        .iter()
        .any(|p| p.contains("body is empty")));
    assert!(one(item(
        json!({ "meta": { "": 1 }, "source": "https://x.test" })
    ))
    .iter()
    .any(|p| p.contains("empty")));
    // Empty edits of the labels are no change at all.
    assert!(one(item(json!({ "addLabels": [] })))
        .iter()
        .any(|p| p.contains("nothing to change")));

    let create = |extra: Value| {
        let mut v = json!({ "op": "create", "title": "t", "project": "p" });
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    assert!(one(create(json!({ "title": " " })))
        .iter()
        .any(|p| p.contains("title is empty")));
    assert!(one(create(json!({ "sourceTitle": "x" })))
        .iter()
        .any(|p| p.contains("need a source")));
    assert!(one(create(json!({ "heldOn": "2026-10-05", "cycle": 1 })))
        .iter()
        .any(|p| p.contains("cannot be given together")));
    assert!(one(create(
        json!({ "labels": ["a"], "priority": 4, "estimate": 0 })
    ))
    .is_empty());
}

#[test]
fn every_problem_names_its_item_and_a_source_is_used_once() {
    let p = problems(json!({ "issues": [
        { "op": "create", "title": "a", "project": "p", "source": "https://x.test/s" },
        { "op": "update", "issue": "KK-1" },
        { "op": "update", "issue": "KK-2", "source": " https://x.test/s " },
    ] }));
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(p[0].starts_with("issues[1] (update KK-1): "), "{p:?}");
    assert!(
        p[1].starts_with("issues[2] (update KK-2): ") && p[1].contains("already used by issues[0]"),
        "{p:?}"
    );
}

#[test]
fn the_schema_describes_the_document_and_the_published_file_is_current() {
    let schema = batch::schema();
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["required"], json!(["issues"]));
    assert_eq!(schema["additionalProperties"], false);
    let item = &schema["$defs"]["Item"]["oneOf"];
    let ops: Vec<&Value> = item
        .as_array()
        .unwrap()
        .iter()
        .map(|v| &v["properties"]["op"]["const"])
        .collect();
    assert_eq!(ops, [&json!("create"), &json!("update")]);
    for variant in item.as_array().unwrap() {
        assert_eq!(variant["additionalProperties"], false);
        assert!(variant["required"]
            .as_array()
            .unwrap()
            .contains(&json!("op")));
    }

    // schema/issue-batch.schema.json is what `linear issue batch --schema` prints.
    let published = include_str!("../../../schema/issue-batch.schema.json");
    let mut current = serde_json::to_string_pretty(&schema).unwrap();
    current.push('\n');
    assert!(
        published == current,
        "schema/issue-batch.schema.json is stale; regenerate it with: linear issue batch --schema > schema/issue-batch.schema.json"
    );
}
