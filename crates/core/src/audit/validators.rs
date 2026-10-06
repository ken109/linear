//! Validator rules applied to issues that already exist.

use super::{issue_is_open, issue_target, owns_issue, Ctx, Finding, RuleId, Severity};
use crate::config::Rule;
use crate::rules::Violation;
use crate::types::Issue;

/// Every issue that is open is checked, and every issue the audit was narrowed
/// to (a caller that touched an issue wants it right even if it just closed).
pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    if ctx.rules.is_empty() {
        return;
    }
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
    }
}

fn finding(ctx: &Ctx, i: &Issue, rule: Rule, message: String) -> Finding {
    let (id, fix) = match rule {
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
