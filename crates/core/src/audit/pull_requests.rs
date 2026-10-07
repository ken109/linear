//! Rules about the GitHub pull requests Linear's integration linked to issues.
//!
//! Both only *warn*: a pull request is evidence, never the authority on an
//! issue's state, so nothing here says to change one automatically. A workspace
//! without the integration, or an issue with no pull-request attachment, has
//! nothing to look at and yields no finding.

use super::{issue_target, owns_issue, Ctx, Finding, RuleId, Severity};
use crate::pull_request::{PullRequest, PullRequestStatus};
use crate::types::{Issue, StateType};

pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    for i in ctx.snapshot.issues.iter().filter(|i| ctx.issue_in_scope(i)) {
        let prs = i.pull_requests();
        if prs.is_empty() {
            continue;
        }
        merged_but_in_progress(ctx, i, &prs, out);
        open_too_long(ctx, i, &prs, out);
    }
}

/// An In Progress issue whose pull request has been merged, and which has no
/// other pull request still waiting: the work looks done and the state does not.
fn merged_but_in_progress(ctx: &Ctx, i: &Issue, prs: &[PullRequest], out: &mut Vec<Finding>) {
    if i.state.state_type() != StateType::Started {
        return;
    }
    if prs.iter().any(|p| p.status.is_pending()) {
        return;
    }
    let Some(merged) = prs
        .iter()
        .filter(|p| p.status == PullRequestStatus::Merged)
        .max_by_key(|p| p.merged_at)
    else {
        return;
    };
    out.push(ctx.finding(
        RuleId::PrMergedIssueOpen,
        Severity::Warn,
        issue_target(i),
        owns_issue(i),
        format!(
            "{} is still {} but its pull request #{} is merged ({})",
            i.identifier, i.state.name, merged.number, merged.url
        ),
        format!("linear issue update {} --state <state>", i.identifier),
    ));
}

/// A pull request (not a draft) that has stayed open for `pr_open_days`.
fn open_too_long(ctx: &Ctx, i: &Issue, prs: &[PullRequest], out: &mut Vec<Finding>) {
    let limit = i64::from(ctx.config.pr_open_days);
    let Some((oldest, age)) = prs
        .iter()
        .filter(|p| p.status == PullRequestStatus::Open)
        .map(|p| (p, (ctx.now - p.opened_at).num_days()))
        .filter(|(_, age)| *age >= limit)
        .max_by_key(|(_, age)| *age)
    else {
        return;
    };
    out.push(ctx.finding(
        RuleId::PrOpenTooLong,
        Severity::Warn,
        issue_target(i),
        owns_issue(i),
        format!(
            "{}: pull request #{} has been open for {age} days (limit {limit}): {}",
            i.identifier, oldest.number, oldest.url
        ),
        format!("linear issue view {}", i.identifier),
    ));
}
