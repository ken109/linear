//! Validator rules applied to issues that already exist.

use super::{issue_is_open, issue_target, owns_issue, Ctx, Finding, RuleId, Severity};
use crate::config::Rule;
use crate::rules::template_sections::{headings, missing_sections, SectionProblem};
use crate::rules::{Operation, Violation};
use crate::types::{Issue, Template};

/// Every issue that is open is checked, and every issue the audit was narrowed
/// to (a caller that touched an issue wants it right even if it just closed).
pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    if ctx.rules.is_empty() {
        return;
    }
    let templates = issue_templates(ctx);
    let check_templates = ctx
        .rules
        .applies(Rule::TemplateSections, Operation::IssueCreate);
    for i in &ctx.snapshot.issues {
        let explicit = ctx.scope.is_some() && ctx.issue_in_scope(i);
        if !(explicit || ctx.scope.is_none() && issue_is_open(i)) {
            continue;
        }
        let violations = ctx.rules.audit_issue(i);
        for rule in [Rule::SourceAttachment, Rule::LabelGroupsExclusive] {
            let messages: Vec<&str> = violations
                .iter()
                .filter(|v| v.rule == rule)
                .map(|v: &Violation| v.message.as_str())
                .collect();
            if !messages.is_empty() {
                out.push(finding(ctx, i, rule, messages.join("; ")));
            }
        }
        if check_templates {
            if let Some(message) = template_problem(i, &templates) {
                out.push(finding(ctx, i, Rule::TemplateSections, message));
            }
        }
    }
}

fn finding(ctx: &Ctx, i: &Issue, rule: Rule, message: String) -> Finding {
    let (id, fix) = match rule {
        Rule::TemplateSections => (
            RuleId::TemplateSections,
            format!("linear issue update {} --body-file <file>", i.identifier),
        ),
        Rule::SourceAttachment => (
            RuleId::SourceAttachment,
            format!("linear issue update {} --source <url>", i.identifier),
        ),
        _ => (
            RuleId::LabelGroupsExclusive,
            format!("linear issue update {} --labels <labels>", i.identifier),
        ),
    };
    ctx.finding(
        id,
        Severity::Warn,
        issue_target(i),
        owns_issue(i),
        message,
        fix,
    )
}

// ------------------------------------------------------- template-sections

/// An issue template with the headings its body must have.
struct Sectioned<'a> {
    template: &'a Template,
    headings: Vec<String>,
}

/// The snapshot's issue templates that have at least one section.
fn issue_templates<'a>(ctx: &Ctx<'a>) -> Vec<Sectioned<'a>> {
    let mut found: Vec<Sectioned> = ctx
        .snapshot
        .templates
        .iter()
        .filter(|t| t.type_ == "issue")
        .filter_map(|t| {
            let headings = headings(t).filter(|h| !h.is_empty())?;
            Some(Sectioned {
                template: t,
                headings,
            })
        })
        .collect();
    found.sort_by(|a, b| a.template.name.cmp(&b.template.name));
    found
}

/// What is wrong with an issue's body against the templates, if anything.
///
/// An issue does not record which template it was written from, so the one
/// whose sections the body shares most is taken (ties go to the one with the
/// fewest sections still missing, then to the first by name). A body that
/// shares no section with any template follows none of them, which is a
/// problem in itself. Without templates there is nothing to hold it to.
fn template_problem(i: &Issue, templates: &[Sectioned]) -> Option<String> {
    let candidates: Vec<&Sectioned> = templates
        .iter()
        .filter(|t| {
            t.template
                .team
                .as_ref()
                .is_none_or(|team| team.id == i.team.id)
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let body = i.description.as_deref().unwrap_or("");

    let mut best: Option<(&Sectioned, Vec<SectionProblem>, usize)> = None;
    for c in candidates.iter().copied() {
        let problems = missing_sections(body, &c.headings);
        let present = c.headings.len() - problems.iter().filter(|p| !p.empty).count();
        let better = match &best {
            None => true,
            Some((_, best_problems, best_present)) => {
                present > *best_present
                    || (present == *best_present && problems.len() < best_problems.len())
            }
        };
        if better {
            best = Some((c, problems, present));
        }
    }
    let (chosen, problems, present) = best?;

    if present == 0 {
        let names: Vec<&str> = candidates
            .iter()
            .map(|c| c.template.name.as_str())
            .collect();
        return Some(format!(
            "{} follows none of the issue templates ({})",
            i.identifier,
            names.join(", ")
        ));
    }
    if problems.is_empty() {
        return None;
    }
    let sections: Vec<String> = problems
        .iter()
        .map(|p| {
            format!(
                "section {:?} {}",
                p.section,
                if p.empty { "is empty" } else { "is missing" }
            )
        })
        .collect();
    Some(format!(
        "{} does not fill the template {:?}: {}",
        i.identifier,
        chosen.template.name,
        sections.join("; ")
    ))
}
