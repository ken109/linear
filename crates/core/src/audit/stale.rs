//! Rules about things that have gone quiet.

use super::{
    issue_target, owns_issue, owns_project, project_target, Ctx, Finding, RuleId, Severity,
};
use crate::types::{Issue, Project, ProjectStatusType, StateType};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    for i in ctx.snapshot.issues.iter().filter(|i| ctx.issue_in_scope(i)) {
        stale_in_progress(ctx, i, out);
    }
    // State changes are read from every issue, narrowed or not: an issue
    // outside the scope still changes what its project's update must say.
    let changes = last_state_changes(ctx);
    for p in ctx
        .snapshot
        .projects
        .iter()
        .filter(|p| ctx.project_in_scope(p))
    {
        status_update_outdated(ctx, p, &changes, out);
    }
}

fn whole_days(ctx: &Ctx, since: DateTime<Utc>) -> i64 {
    (ctx.now - since).num_days()
}

/// An issue that is In Progress and has not been touched for `stale_days`.
fn stale_in_progress(ctx: &Ctx, i: &Issue, out: &mut Vec<Finding>) {
    if i.state.state_type() != StateType::Started {
        return;
    }
    let idle = whole_days(ctx, i.updated_at);
    let limit = i64::from(ctx.config.stale_days);
    if idle < limit {
        return;
    }
    out.push(ctx.finding(
        RuleId::StaleInProgress,
        Severity::Warn,
        issue_target(i),
        owns_issue(i),
        format!(
            "{} has been {} with no update for {idle} days (limit {limit})",
            i.identifier, i.state.name
        ),
        format!("linear issue update {} --state <state>", i.identifier),
    ));
}

/// When an issue last changed state, and which issue it was.
type StateChange<'a> = (DateTime<Utc>, &'a str);

/// The latest state change among each project's issues, by project id.
///
/// A state change is when the issue started, completed or was canceled
/// (`canceledAt` also covers duplicates). Linear always sets `canceledAt` on a
/// canceled issue; `updatedAt` stands in only if it is somehow missing, and can
/// then only be later than the real change, never earlier.
fn last_state_changes<'a>(ctx: &Ctx<'a>) -> HashMap<&'a str, StateChange<'a>> {
    let mut latest: HashMap<&str, StateChange> = HashMap::new();
    for i in &ctx.snapshot.issues {
        let Some(project) = i.project.as_ref() else {
            continue;
        };
        let canceled = matches!(
            i.state.state_type(),
            StateType::Canceled | StateType::Duplicate
        )
        .then(|| i.canceled_at.unwrap_or(i.updated_at));
        let Some(changed) = [i.started_at, i.completed_at, canceled]
            .into_iter()
            .flatten()
            .max()
        else {
            continue;
        };
        let entry = latest
            .entry(project.id.inner())
            .or_insert((changed, i.identifier.as_str()));
        if changed > entry.0 {
            *entry = (changed, i.identifier.as_str());
        }
    }
    latest
}

/// An In Progress project whose latest status update no longer describes it:
/// there is none, issues changed state after it, or it is `status_update_days`
/// old.
fn status_update_outdated(
    ctx: &Ctx,
    p: &Project,
    changes: &HashMap<&str, StateChange>,
    out: &mut Vec<Finding>,
) {
    if p.status.type_ != ProjectStatusType::Started {
        return;
    }
    let mut reasons = Vec::new();
    match p.last_update.as_ref() {
        None => reasons.push("it has no status update".to_owned()),
        Some(update) => {
            let posted = update.created_at;
            if let Some(&(changed, issue)) = changes.get(p.id.inner()) {
                if changed > posted {
                    reasons.push(format!(
                        "its latest status update ({}) is older than the last issue state change ({}, {issue})",
                        posted.date_naive(),
                        changed.date_naive()
                    ));
                }
            }
            let age = whole_days(ctx, posted);
            let limit = i64::from(ctx.config.status_update_days);
            if age >= limit {
                reasons.push(format!(
                    "its latest status update is {age} days old (limit {limit})"
                ));
            }
        }
    }
    if reasons.is_empty() {
        return;
    }
    out.push(ctx.finding(
        RuleId::StatusUpdateOutdated,
        Severity::Warn,
        project_target(p),
        owns_project(p),
        format!(
            "project {} is {}: {}",
            p.name,
            p.status.name,
            reasons.join("; ")
        ),
        format!("linear project status-update {} --body <text>", p.slug_id),
    ));
}
