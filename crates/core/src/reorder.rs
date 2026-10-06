//! Planning `issue reorder`.
//!
//! Linear keeps two numbers per issue that decide where it sits on screen,
//! both ascending (smallest on top): `sortOrder` for a manually ordered view
//! and `prioritySortOrder` for the default, priority-ordered view. Fixing only
//! one leaves the other view unchanged, so a reorder rewrites both.
//!
//! The plan only permutes the values the named issues already hold: the
//! values they occupy are handed back out in the requested order. The set of
//! values does not change, so issues sitting between them keep their relative
//! position and nothing collides with an issue that was not named. Only when
//! some of the held values are equal (the order was undefined) is the range
//! spread evenly instead.

use crate::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

/// An issue's two ordering values, as they are now.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRow {
    /// The identifier the user named it by (e.g. `KK-12`).
    pub identifier: String,
    pub sort_order: f64,
    pub priority_sort_order: f64,
}

/// New ordering values for one issue. `None` leaves that value alone.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderChange {
    pub identifier: String,
    pub sort_order: Option<f64>,
    pub priority_sort_order: Option<f64>,
}

/// Plan the new order of `wanted` (identifiers, top first).
///
/// Returns only the issues that need a write, in the requested order; an empty
/// plan means they already sit in that order. A request naming fewer than two
/// issues, naming one twice, or naming one that `rows` does not hold is a usage
/// error.
pub fn plan(rows: &[OrderRow], wanted: &[String]) -> Result<Vec<OrderChange>> {
    if wanted.len() < 2 {
        return Err(Error::Usage(
            "reordering needs at least two issues".to_owned(),
        ));
    }
    let mut seen = BTreeSet::new();
    for id in wanted {
        if !seen.insert(id.as_str()) {
            return Err(Error::Usage(format!("{id} is listed more than once")));
        }
    }
    let by_id: BTreeMap<&str, &OrderRow> =
        rows.iter().map(|r| (r.identifier.as_str(), r)).collect();
    let missing: Vec<&str> = wanted
        .iter()
        .map(String::as_str)
        .filter(|id| !by_id.contains_key(id))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Usage(format!(
            "no such issue: {}",
            missing.join(", ")
        )));
    }

    let held = |pick: fn(&OrderRow) -> f64| -> Vec<f64> {
        wanted.iter().map(|id| pick(by_id[id.as_str()])).collect()
    };
    let sort = assign(&held(|r| r.sort_order));
    let priority = assign(&held(|r| r.priority_sort_order));

    Ok(wanted
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let row = by_id[id.as_str()];
            let change = OrderChange {
                identifier: id.clone(),
                sort_order: (sort[i] != row.sort_order).then_some(sort[i]),
                priority_sort_order: (priority[i] != row.priority_sort_order)
                    .then_some(priority[i]),
            };
            (change.sort_order.is_some() || change.priority_sort_order.is_some()).then_some(change)
        })
        .collect())
}

/// The value each position in the requested order receives, given the values
/// the issues hold now (listed in the requested order).
fn assign(current: &[f64]) -> Vec<f64> {
    let mut slots = current.to_vec();
    slots.sort_by(f64::total_cmp);
    let tied = slots.windows(2).any(|w| w[0] == w[1]);
    if !tied {
        return slots;
    }
    let lo = slots[0];
    let hi = slots[slots.len() - 1];
    let span = if hi > lo { hi - lo } else { 1.0 };
    let steps = (slots.len() - 1) as f64;
    (0..slots.len())
        .map(|i| lo + span * i as f64 / steps)
        .collect()
}
