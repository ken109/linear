//! Which operations does a user-written document define?

use linear_core::document::{operation_kinds, OperationKind::*};

#[test]
fn shorthand_and_named_queries() {
    assert_eq!(operation_kinds("{ viewer { id } }").unwrap(), [Query]);
    assert_eq!(operation_kinds("query { viewer { id } }").unwrap(), [Query]);
    assert_eq!(
        operation_kinds("query Q($id: String!) @live { issue(id: $id) { id } }").unwrap(),
        [Query]
    );
}

#[test]
fn mutations_and_subscriptions_are_found() {
    assert_eq!(
        operation_kinds("mutation { issueDelete(id: \"x\") { success } }").unwrap(),
        [Mutation]
    );
    assert_eq!(
        operation_kinds("subscription S { a }").unwrap(),
        [Subscription]
    );
    assert_eq!(
        operation_kinds(
            "query A { viewer { id } }\nmutation B { issueDelete(id: \"x\") { success } }"
        )
        .unwrap(),
        [Query, Mutation]
    );
    // A default value with braces in the variable list does not hide what follows.
    assert_eq!(
        operation_kinds("mutation M($i: In = {a: 1, b: [1, 2]}) { x(input: $i) { id } }").unwrap(),
        [Mutation]
    );
    // Insignificant characters before the keyword.
    assert_eq!(
        operation_kinds("\u{feff}  ,,\n# note\nmutation{a}").unwrap(),
        [Mutation]
    );
}

#[test]
fn keywords_inside_strings_comments_and_names_do_not_count() {
    let doc = r#"
        # mutation { evil }
        query Q {
          a(s: "mutation { x }", b: """ mutation " { """, c: "esc \" mutation {")
          mutation
        }
    "#;
    assert_eq!(operation_kinds(doc).unwrap(), [Query]);

    // A fragment may be named like a keyword.
    let doc = "fragment mutation on Issue { id }\nquery Q { issue(id: \"a\") { ...mutation } }";
    assert_eq!(operation_kinds(doc).unwrap(), [Query]);
    assert_eq!(
        operation_kinds("query { a } fragment F on Issue { id } mutation { b }").unwrap(),
        [Query, Mutation]
    );

    // A block string with an escaped triple quote.
    let doc = "query { a(s: \"\"\"x \\\"\"\" mutation { y\"\"\") }";
    assert_eq!(operation_kinds(doc).unwrap(), [Query]);
}

#[test]
fn malformed_documents_are_rejected() {
    for doc in [
        "",
        "   # only a comment",
        "fragment F on Issue { id }",
        "query { a",
        "query a }",
        "query { a(s: \"unterminated) }",
        "query { a(s: \"\"\"unterminated) }",
        "query { a(s: \"line\nbreak\") }",
        "query ( { a }",
    ] {
        let err = operation_kinds(doc).unwrap_err();
        assert_eq!(err.code(), linear_core::ErrorCode::Usage, "{doc:?}");
    }
}
