//! `label-groups-exclusive`: at most one label from each label group.
//!
//! Linear itself rejects an issue with two labels of a single-select group,
//! but only after the issue exists, which would leave a half-made issue
//! behind. Checking first keeps writes all-or-nothing. A group Linear marks
//! multi-select is exempt: several of its labels are legitimate.

use super::{Violation, ViolationKind};
use crate::config::Rule;
use crate::types::{Issue, Label, LabelGroup, LabelGroupType};
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

/// A change to the label groups that the issues which already carry the labels
/// have to survive: it must not leave an issue with two labels of a
/// single-select group.
///
/// This is the same rule as for an issue write, asked about the other
/// direction: not "may this issue take these labels" but "may these labels
/// move". A label that is created is on no issue yet, so a creation has
/// nothing to check here.
#[derive(Debug, Clone, Copy)]
pub enum Regroup<'a> {
    /// The label with this id moves into `group` (whose type then applies to it).
    Move {
        label_id: &'a str,
        group: &'a LabelGroup,
    },
    /// The group with this id changes its selection mode.
    Retype {
        group_id: &'a str,
        group_type: &'a LabelGroupType,
    },
}

impl Regroup<'_> {
    /// The labels of an issue as they would be after the change.
    fn apply(&self, labels: &[Label]) -> Vec<Label> {
        labels
            .iter()
            .map(|label| {
                let mut label = label.clone();
                match *self {
                    Regroup::Move { label_id, group } if label.id.inner() == label_id => {
                        label.parent = Some(group.clone());
                    }
                    Regroup::Retype {
                        group_id,
                        group_type,
                    } => {
                        if let Some(parent) = label.parent.as_mut() {
                            if parent.id.inner() == group_id {
                                parent.group_type = Some(group_type.clone());
                            }
                        }
                    }
                    Regroup::Move { .. } => {}
                }
                label
            })
            .collect()
    }
}

/// The issues (of the ones given: those that carry the labels concerned) that
/// the change would leave with two labels of a single-select group. A conflict
/// an issue already has is not the change's doing and is not reported.
pub fn regroup_violations(issues: &[Issue], change: &Regroup<'_>) -> Vec<Violation> {
    let mut out = Vec::new();
    for issue in issues {
        let before = conflicts(&issue.labels);
        for conflict in conflicts(&change.apply(&issue.labels)) {
            if before.contains(&conflict) {
                continue;
            }
            out.push(Violation::new(
                Rule::LabelGroupsExclusive,
                ViolationKind::LabelGroupConflict,
                Some(conflict.group.clone()),
                format!(
                    "{} would have more than one label of the group {:?} ({})",
                    issue.identifier,
                    conflict.group,
                    conflict.labels.join(", ")
                ),
            ));
        }
    }
    out
}
