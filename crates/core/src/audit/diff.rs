//! What is new since the last audit.

use super::{Finding, FindingKey};
use std::collections::HashSet;

/// The findings in `current` that were not in `previous`.
///
/// A finding is the same finding when its [`FindingKey`] (workspace, rule and
/// target) matches; the message is not compared, because it carries counts
/// of days that change every day and would make a lasting finding look new
/// daily. This is what a notifier uses so the same problem is reported once.
/// Order follows `current`. Pass an empty `previous` when there was no
/// earlier audit: everything is new.
pub fn diff(previous: &[Finding], current: &[Finding]) -> Vec<Finding> {
    let mut seen: HashSet<FindingKey> = previous.iter().map(Finding::key).collect();
    current
        .iter()
        .filter(|f| seen.insert(f.key()))
        .cloned()
        .collect()
}
