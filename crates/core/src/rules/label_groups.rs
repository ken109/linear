//! `label-groups-exclusive`: at most one label from each label group.
//!
//! Linear itself rejects an issue with two labels of a single-select group,
//! but only after the issue exists, which would leave a half-made issue
//! behind. Checking first keeps writes all-or-nothing. A group Linear marks
//! multi-select is exempt: several of its labels are legitimate.

use super::{Violation, ViolationKind};
use crate::config::Rule;
use crate::types::{Label, LabelGroupType};
use std::collections::BTreeMap;

/// A group with more than one of its labels chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupConflict {
    pub group: String,
    /// The chosen labels of the group, in the order given.
    pub labels: Vec<String>,
}

/// The exclusive groups that `labels` picks more than one label from.
/// The same label listed twice is not a conflict.
pub fn conflicts(labels: &[Label]) -> Vec<GroupConflict> {
    // group id -> (group name, chosen label ids seen, chosen label names)
    let mut groups: BTreeMap<&str, (&str, Vec<&str>, Vec<&str>)> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();

    for label in labels {
        let Some(parent) = &label.parent else {
            continue;
        };
        if parent.group_type == Some(LabelGroupType::MultiSelect) {
            continue;
        }
        let key = parent.id.inner();
        let entry = groups.entry(key).or_insert_with(|| {
            order.push(key);
            (parent.name.as_str(), Vec::new(), Vec::new())
        });
        if !entry.1.contains(&label.id.inner()) {
            entry.1.push(label.id.inner());
            entry.2.push(label.name.as_str());
        }
    }

    order
        .into_iter()
        .filter_map(|key| {
            let (name, _, names) = groups.remove(key)?;
            (names.len() > 1).then(|| GroupConflict {
                group: name.to_owned(),
                labels: names.into_iter().map(str::to_owned).collect(),
            })
        })
        .collect()
}

pub(super) fn check(labels: &[Label]) -> Vec<Violation> {
    conflicts(labels)
        .into_iter()
        .map(|c| {
            Violation::new(
                Rule::LabelGroupsExclusive,
                ViolationKind::LabelGroupConflict,
                Some(c.group.clone()),
                format!(
                    "only one label of the group {:?} can be set (got {})",
                    c.group,
                    c.labels.join(", ")
                ),
            )
        })
        .collect()
}
