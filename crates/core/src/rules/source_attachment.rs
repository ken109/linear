//! `source-attachment`: every issue records where it came from.
//!
//! The origin is an http(s) URL attached to the issue. It is also the
//! idempotence key: creating an issue whose source is already attached to
//! another issue creates nothing and reports the existing one.

use super::{Draft, Fetched, Violation, ViolationKind};
use crate::config::Rule;
use crate::types::{Issue, IssueRef};

/// Why a source is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceProblem {
    Missing,
    NotHttp,
}

/// Validate a source URL, returning it trimmed.
pub fn validate(source: Option<&str>) -> Result<&str, SourceProblem> {
    let url = source
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(SourceProblem::Missing)?;
    if is_http_url(url) {
        Ok(url)
    } else {
        Err(SourceProblem::NotHttp)
    }
}

/// `http://` or `https://`, a host, and no whitespace.
pub fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !url.chars().any(char::is_whitespace)
}

pub(super) enum Check {
    /// An issue already carries this source.
    Existing(IssueRef),
    Violations(Vec<Violation>),
}

pub(super) fn check(draft: &Draft, fetched: &Fetched) -> Check {
    match validate(draft.source.as_deref()) {
        Ok(url) => match fetched.existing_by_source.get(url) {
            Some(existing) => Check::Existing(existing.clone()),
            None => Check::Violations(Vec::new()),
        },
        Err(problem) => Check::Violations(vec![violation(problem, draft.source.as_deref())]),
    }
}

/// `source-attachment` on an issue that already exists: it must carry an
/// http(s) attachment.
pub(super) fn check_existing(issue: &Issue) -> Vec<Violation> {
    let has_source = issue.attachments.iter().any(|a| is_http_url(a.url.trim()));
    if has_source {
        Vec::new()
    } else {
        vec![Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceRequired,
            Some(issue.identifier.clone()),
            format!("{} has no http(s) source attachment", issue.identifier),
        )]
    }
}

fn violation(problem: SourceProblem, given: Option<&str>) -> Violation {
    match problem {
        SourceProblem::Missing => Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceRequired,
            None,
            "a source URL is required (pass --source)",
        ),
        SourceProblem::NotHttp => Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceInvalid,
            given.map(|s| s.trim().to_owned()),
            format!(
                "the source must be an http(s) URL, got {:?}",
                given.unwrap_or("").trim()
            ),
        ),
    }
}
