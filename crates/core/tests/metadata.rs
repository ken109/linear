//! Attachment metadata: parsing `--meta`, validating JSON, comparing with what is stored.

use linear_core::metadata::{AttachmentMetadata, MetaValue, MetadataError};
use serde_json::{json, Value};

fn object(v: Value) -> serde_json::Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("not an object: {other}"),
    }
}

fn json_of(pairs: &[&str]) -> Value {
    Value::Object(AttachmentMetadata::from_pairs(pairs).unwrap().to_json())
}

// ------------------------------------------------------------------ --meta

#[test]
fn values_that_look_like_numbers_are_numbers() {
    assert_eq!(
        json_of(&["a=42", "b=-7", "c=3.5", "d=1e3", "e=0", "f=0.25", "g=-0.5", "h=2E-2"]),
        json!({ "a": 42, "b": -7, "c": 3.5, "d": 1000.0, "e": 0, "f": 0.25, "g": -0.5, "h": 0.02 })
    );
}

#[test]
fn everything_else_stays_a_string() {
    for text in [
        "007", "+1", ".5", "5.", "1_000", "0x1F", "NaN", "inf", "-inf", "1e", "1e+", "--1", "",
        " 1", "1 ", "true", "null", "1,5", "12abc",
    ] {
        let meta = AttachmentMetadata::from_pairs(&[format!("k={text}")]).unwrap();
        assert_eq!(
            meta.get("k"),
            Some(&MetaValue::String(text.to_owned())),
            "{text:?}"
        );
    }
}

#[test]
fn an_integer_too_large_to_hold_exactly_is_a_string() {
    // 2^64: an id must not be rounded to a float.
    let big = "18446744073709551616";
    assert_eq!(json_of(&[&format!("k={big}")]), json!({ "k": big }));
    // The largest u64 is still exact.
    assert_eq!(
        json_of(&["k=18446744073709551615"]),
        json!({ "k": 18446744073709551615u64 })
    );
}

#[test]
fn the_str_prefix_forces_a_string() {
    assert_eq!(
        json_of(&["id=str:123", "x=str:1.5", "y=str:", "z=str:str:1"]),
        json!({ "id": "123", "x": "1.5", "y": "", "z": "str:1" })
    );
}

#[test]
fn a_value_may_contain_equals_signs() {
    assert_eq!(
        json_of(&["q=a=b=c", "url=https://x.test/?a=1"]),
        json!({ "q": "a=b=c", "url": "https://x.test/?a=1" })
    );
}

#[test]
fn keys_are_trimmed_and_must_not_be_empty() {
    assert_eq!(json_of(&[" kind =slack"]), json!({ "kind": "slack" }));
    assert_eq!(
        AttachmentMetadata::from_pairs(&["=x"]).unwrap_err(),
        MetadataError::EmptyKey
    );
    assert_eq!(
        AttachmentMetadata::from_pairs(&["  =x"]).unwrap_err(),
        MetadataError::EmptyKey
    );
}

#[test]
fn a_pair_without_equals_is_refused() {
    let err = AttachmentMetadata::from_pairs(&["kind"]).unwrap_err();
    assert_eq!(err, MetadataError::NotKeyValue("kind".into()));
    assert!(err.to_string().contains("key=value"), "{err}");
}

#[test]
fn a_repeated_key_is_refused_rather_than_overridden() {
    let err = AttachmentMetadata::from_pairs(&["a=1", "a=2"]).unwrap_err();
    assert_eq!(err, MetadataError::DuplicateKey("a".into()));
}

// ------------------------------------------------------------------ JSON

#[test]
fn json_objects_of_strings_and_numbers_are_accepted() {
    let meta = AttachmentMetadata::from_json(&object(json!({ "kind": "slack", "n": 3, "f": 0.5 })))
        .unwrap();
    assert_eq!(meta.get_str("kind"), Some("slack"));
    assert_eq!(meta.get_str("n"), None, "a number is not a string");
    assert_eq!(
        Value::Object(meta.to_json()),
        json!({ "kind": "slack", "n": 3, "f": 0.5 })
    );
    assert!(AttachmentMetadata::from_json(&object(json!({})))
        .unwrap()
        .is_empty());
}

#[test]
fn nested_values_booleans_and_null_are_refused_with_the_key_named() {
    for (value, kind) in [
        (json!({ "x": 1 }), "a nested object"),
        (json!([1]), "an array"),
        (json!(true), "a boolean"),
        (json!(null), "null"),
    ] {
        let err = AttachmentMetadata::from_json(&object(json!({ "ok": "fine", "bad": value })))
            .unwrap_err();
        assert_eq!(
            err,
            MetadataError::BadValue {
                key: "bad".into(),
                found: kind
            }
        );
        assert!(err.to_string().contains("\"bad\""), "{err}");
    }
}

#[test]
fn an_empty_key_in_json_is_refused() {
    assert_eq!(
        AttachmentMetadata::from_json(&object(json!({ "": "x" }))).unwrap_err(),
        MetadataError::EmptyKey
    );
}

#[test]
fn it_serializes_as_a_plain_json_object() {
    let meta = AttachmentMetadata::from_pairs(&["b=1", "a=x"]).unwrap();
    assert_eq!(serde_json::to_string(&meta).unwrap(), r#"{"a":"x","b":1}"#);
}

// ------------------------------------------------------------------ matching

#[test]
fn metadata_matches_exactly_what_is_stored() {
    let meta = AttachmentMetadata::from_pairs(&["kind=slack", "n=1"]).unwrap();
    assert!(meta.matches(&object(json!({ "kind": "slack", "n": 1 }))));
    // A number Linear hands back as a float is the same number.
    assert!(meta.matches(&object(json!({ "kind": "slack", "n": 1.0 }))));
    // A changed value, a missing key, an extra key and a changed type all differ.
    assert!(!meta.matches(&object(json!({ "kind": "other", "n": 1 }))));
    assert!(!meta.matches(&object(json!({ "kind": "slack" }))));
    assert!(!meta.matches(&object(json!({ "kind": "slack", "n": 1, "x": "y" }))));
    assert!(!meta.matches(&object(json!({ "kind": "slack", "n": "1" }))));
    assert!(!meta.matches(&object(json!({ "kind": "slack", "n": null }))));
}

#[test]
fn a_string_and_a_number_with_the_same_digits_differ() {
    let as_string = AttachmentMetadata::from_pairs(&["id=str:42"]).unwrap();
    assert!(as_string.matches(&object(json!({ "id": "42" }))));
    assert!(!as_string.matches(&object(json!({ "id": 42 }))));
}
