//! Narrowing an audit to the issues a caller worked on.

use super::{issue_target, Ctx, Finding, RuleId, Severity, Snapshot};
use crate::error::{Error, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// What an audit is narrowed to (`audit --issues KK-1,KK-2 --since <time>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AuditOptions {
    /// Issue identifiers, matched without regard to case.
    pub issues: Option<Vec<String>>,
    /// The time those issues must have been updated at or after.
    pub since: Option<DateTime<Utc>>,
}

/// The issues and projects an audit is narrowed to.
pub(crate) struct Scope<'a> {
    issue_ids: HashSet<&'a str>,
    project_ids: HashSet<&'a str>,
    since: Option<DateTime<Utc>>,
}

impl Scope<'_> {
    pub fn has_issue(&self, id: &str) -> bool {
        self.issue_ids.contains(id)
    }

    pub fn has_project(&self, id: &str) -> bool {
        self.project_ids.contains(id)
    }
}

/// Resolve `options` against the snapshot: the scope, and the requested
/// identifiers the snapshot does not hold. `None` when not narrowed.
pub(crate) fn resolve<'a>(
    snapshot: &'a Snapshot,
    options: &AuditOptions,
) -> Result<Option<(Scope<'a>, Vec<String>)>> {
    let Some(wanted) = options.issues.as_ref() else {
        if options.since.is_some() {
            return Err(Error::Usage(
                "--since needs --issues: it says which issues must have been updated".into(),
            ));
        }
        return Ok(None);
    };

    let wanted_upper: HashSet<String> = wanted
        .iter()
        .map(|w| w.trim().to_ascii_uppercase())
        .collect();
    let mut scope = Scope {
        issue_ids: HashSet::new(),
        project_ids: HashSet::new(),
        since: options.since,
    };
    let mut found = HashSet::new();
    for i in &snapshot.issues {
        let ident = i.identifier.to_ascii_uppercase();
        if wanted_upper.contains(&ident) {
            scope.issue_ids.insert(i.id.inner());
            if let Some(p) = &i.project {
                scope.project_ids.insert(p.id.inner());
            }
            found.insert(ident);
        }
    }

    let mut unresolved: Vec<String> = Vec::new();
    for w in wanted {
        let w = w.trim();
        if !found.contains(&w.to_ascii_uppercase())
            && !unresolved.iter().any(|u| u.eq_ignore_ascii_case(w))
        {
            unresolved.push(w.to_owned());
        }
    }
    Ok(Some((scope, unresolved)))
}

/// `not-updated-since`: an issue the caller named was not updated since `since`.
pub(super) fn run(ctx: &Ctx, out: &mut Vec<Finding>) {
    let Some(since) = ctx.scope.as_ref().and_then(|s| s.since) else {
        return;
    };
    for i in ctx.snapshot.issues.iter().filter(|i| ctx.issue_in_scope(i)) {
        if i.updated_at >= since {
            continue;
        }
        let at = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
        out.push(ctx.finding(
            RuleId::NotUpdatedSince,
            Severity::Warn,
            issue_target(i),
            true,
            format!(
                "{} was last updated {}, before {}",
                i.identifier,
                at(i.updated_at),
                at(since)
            ),
            format!("linear issue update {} --state <state>", i.identifier),
        ));
    }
}
