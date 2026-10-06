//! Telling whether Linear already stores a description.

use linear_core::markdown::same_description;

#[test]
fn trailing_whitespace_and_line_endings_are_not_differences() {
    assert!(same_description(Some("A\n\nB"), "A\n\nB\n\n"));
    assert!(same_description(Some("A  \nB"), "A\r\nB\r\n"));
}

#[test]
fn the_bullet_linear_rewrites_is_not_a_difference() {
    // Linear stores `- item` as `* item`.
    let sent = "## Acceptance criteria\n\n- one\n- two\n  - nested\n\n+ plus\n";
    let stored = "## Acceptance criteria\n\n* one\n* two\n  * nested\n\n* plus";
    assert!(same_description(Some(stored), sent));
    assert!(same_description(Some(sent), sent));
}

#[test]
fn real_differences_are_still_differences() {
    assert!(!same_description(Some("A"), "B"));
    assert!(!same_description(None, "A"));
    // A dash that is not a bullet stays a dash.
    assert!(!same_description(Some("a - b"), "a * b"));
    assert!(!same_description(Some("-b"), "*b"));
    // Indentation matters.
    assert!(!same_description(Some("* a"), "  * a"));
}
