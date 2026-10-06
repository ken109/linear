//! Rules about Linear being consistent with itself.

use super::{
    issue_is_open, issue_target, owns_issue, owns_project, project_is_closed, project_target, Ctx,
    Finding, RuleId, Severity, Target, TargetKind,
};
use crate::types::{Issue, Project, ProjectMilestoneStatus, ProjectStatusType};
use chrono::NaiveDate;

pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    for p in ctx
        .snapshot
        .projects
        .iter()
        .filter(|p| ctx.project_in_scope(p))
    {
        project_state_vs_issues(ctx, p, out);
        project_without_lead(ctx, p, out);
        project_overdue(ctx, p, out);
        milestones_overdue(ctx, p, out);
    }
    for i in ctx.snapshot.issues.iter().filter(|i| ctx.issue_in_scope(i)) {
        issue_overdue(ctx, i, out);
        issue_without_milestone(ctx, i, out);
    }
}

fn days_before(today: NaiveDate, date: NaiveDate) -> i64 {
    (today - date).num_days()
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The project's status says one thing and its issues say another.
///
/// The issue window can be cut short (`IssueCounts::complete`). A mismatch that
/// holds for the issues seen is still real; "everything is closed" is not
/// knowable from a partial list, so that check needs a complete one.
fn project_state_vs_issues(ctx: &Ctx, p: &Project, out: &mut Vec<Finding>) {
    let c = p.issue_counts();
    let open = c.triage + c.backlog + c.unstarted + c.started;
    let status = &p.status.name;
    let slug = &p.slug_id;

    let (message, new_status) = match p.status.type_ {
        ProjectStatusType::Completed | ProjectStatusType::Canceled if open > 0 => (
            format!("project is {status} but has {open} open issue(s)"),
            "started",
        ),
        ProjectStatusType::Backlog | ProjectStatusType::Planned if c.started > 0 => (
            format!(
                "project is {status} but {} issue(s) are already in progress",
                c.started
            ),
            "started",
        ),
        ProjectStatusType::Started if c.complete && c.total() > 0 && open == 0 => (
            format!(
                "project is {status} but all {} issue(s) are closed",
                c.total()
            ),
            "completed",
        ),
        _ => return,
    };
    out.push(ctx.finding(
        RuleId::ProjectStateVsIssues,
        Severity::Warn,
        project_target(p),
        owns_project(p),
        message,
        format!("linear project update {slug} --status {new_status}"),
    ));
}

fn project_without_lead(ctx: &Ctx, p: &Project, out: &mut Vec<Finding>) {
    if p.lead.is_some() || project_is_closed(p) {
        return;
    }
    out.push(ctx.finding(
        RuleId::ProjectWithoutLead,
        Severity::Info,
        project_target(p),
        // Nobody owns it yet, so nobody is asked to fix it.
        false,
        format!("project {} has no lead", p.name),
        format!("linear project update {} --lead <user>", p.slug_id),
    ));
}

fn project_overdue(ctx: &Ctx, p: &Project, out: &mut Vec<Finding>) {
    let Some(target_date) = p.target_date else {
        return;
    };
    if project_is_closed(p) || target_date >= ctx.today {
        return;
    }
    let late = days_before(ctx.today, target_date);
    out.push(ctx.finding(
        RuleId::Overdue,
        Severity::Warn,
        project_target(p),
        owns_project(p),
        format!(
            "project target date {target_date} passed {late} day{} ago",
            plural(late)
        ),
        format!("linear project update {} --target-date <date>", p.slug_id),
    ));
}

/// Milestones of a project that is not closed. A milestone's owner is its
/// project's lead.
fn milestones_overdue(ctx: &Ctx, p: &Project, out: &mut Vec<Finding>) {
    if project_is_closed(p) {
        return;
    }
    for m in p.project_milestones.iter() {
        let Some(target_date) = m.target_date else {
            continue;
        };
        if m.status == ProjectMilestoneStatus::Done || target_date >= ctx.today {
            continue;
        }
        let late = days_before(ctx.today, target_date);
        out.push(ctx.finding(
            RuleId::Overdue,
            Severity::Warn,
            Target {
                kind: TargetKind::Milestone,
                id: m.id.inner().to_owned(),
                identifier: m.name.clone(),
                title: m.name.clone(),
                url: p.url.clone(),
            },
            owns_project(p),
            format!(
                "milestone {} of {} was due {target_date} ({late} day{} ago)",
                m.name,
                p.name,
                plural(late)
            ),
            format!(
                "linear milestone update {} --target-date <date>",
                m.id.inner()
            ),
        ));
    }
}

fn issue_overdue(ctx: &Ctx, i: &Issue, out: &mut Vec<Finding>) {
    let Some(due) = i.due_date else {
        return;
    };
    if !issue_is_open(i) || due >= ctx.today {
        return;
    }
    let late = days_before(ctx.today, due);
    out.push(ctx.finding(
        RuleId::Overdue,
        Severity::Warn,
        issue_target(i),
        owns_issue(i),
        format!(
            "{} was due {due} ({late} day{} ago) and is {}",
            i.identifier,
            plural(late),
            i.state.name
        ),
        format!("linear issue update {} --due-date <date>", i.identifier),
    ));
}

/// An open issue in a project that has milestones but in none of them. A
/// project without milestones, or one that is closed, imposes nothing.
fn issue_without_milestone(ctx: &Ctx, i: &Issue, out: &mut Vec<Finding>) {
    if !issue_is_open(i) || i.project_milestone.is_some() {
        return;
    }
    let Some(p) = ctx.project_of(i) else {
        return;
    };
    if project_is_closed(p) || p.project_milestones.is_empty() {
        return;
    }
    out.push(ctx.finding(
        RuleId::IssueWithoutMilestone,
        Severity::Info,
        issue_target(i),
        owns_issue(i),
        format!(
            "{} is in project {}, which has milestones, but in none of them",
            i.identifier, p.name
        ),
        format!(
            "linear issue update {} --milestone <milestone>",
            i.identifier
        ),
    ));
}
