//! Finding the relation between two issues.
//!
//! Linear stores a relation once, as `issue` `type` `relatedIssue` (`KK-1 blocks KK-2`),
//! and shows it from both ends: on `KK-1` under `relations`, on `KK-2` under
//! `inverseRelations`. `relate` and `unrelate` both need to know whether the relation they
//! were asked for is already there, so the question is answered here, without any I/O.

use crate::inputs::IssueRelationType;
use crate::read::{IssueWithRelations, RelationLink};

/// The relations of `kind` that say what `issue <kind> other` says.
///
/// `blocks` and `duplicate` have a direction: only a relation that starts from `issue` and
/// ends at `other` matches (`other blocks issue` is a different statement, made with the
/// arguments the other way round). `related` has none, so it matches from either end.
/// Linear holds at most one such relation, but every match is returned so a caller that
/// deletes never leaves one behind.
pub fn matching<'a>(
    issue: &'a IssueWithRelations,
    other_id: &str,
    kind: IssueRelationType,
) -> Vec<&'a RelationLink> {
    let wanted = kind.as_str();
    let forward = issue
        .relations
        .iter()
        .filter(|r| r.type_ == wanted && r.related_issue.id.inner() == other_id);
    let backward = issue.inverse_relations.iter().filter(|r| {
        kind == IssueRelationType::Related && r.type_ == wanted && r.issue.id.inner() == other_id
    });
    forward.chain(backward).collect()
}
