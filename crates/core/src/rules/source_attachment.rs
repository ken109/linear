//! `source-attachment`: every issue records where it came from.
//!
//! The origin is an http(s) URL attached to the issue. It is also the
//! idempotence key: creating an issue whose source is already attached to
//! another issue creates nothing and reports the existing one.
//!
//! An update (`issue update --source`) attaches the source to an existing
//! issue: the same URL on the same issue is an upsert, the URL of another
//! issue is refused. It is only checked when the update names a source.
//!
//! With `source_kinds` configured, the source attachment also carries a
//! `metadata.kind` (`--meta kind=...`) that must be one of them. Without it,
//! metadata is not looked at.

use super::{Draft, Fetched, Violation, ViolationKind};
use crate::config::Rule;
use crate::metadata::AttachmentMetadata;
use crate::types::{Issue, IssueRef};
use serde_json::Value;

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

pub(super) fn check(draft: &Draft, fetched: &Fetched, kinds: &[String]) -> Check {
    let update = !draft.operation.is_create();
    // An update that names no source leaves the attachments alone.
    if update && draft.source.is_none() {
        return Check::Violations(Vec::new());
    }
    let url = match validate(draft.source.as_deref()) {
        Ok(url) => url,
        Err(problem) => {
            return Check::Violations(vec![violation(problem, draft.source.as_deref())])
        }
    };
    let existing = fetched.existing_by_source.get(url);
    // An issue that already carries the source is only touched when metadata
    // is given (the CLI then updates the attachment), so only then is the kind
    // judged; without metadata nothing is written.
    let kind_problem = match (existing, &draft.source_metadata) {
        (Some(_), None) => None,
        _ => kind_violation(url, draft.source_metadata.as_ref(), kinds),
    };
    if update {
        // The attachment may already be this issue's (an upsert); the one of
        // another issue is not ours to take over.
        let taken = existing
            .filter(|owner| Some(owner.id.inner()) != draft.issue.as_deref())
            .map(|owner| {
                Violation::new(
                    Rule::SourceAttachment,
                    ViolationKind::SourceTaken,
                    Some(url.to_owned()),
                    format!(
                        "the source {url} is already attached to {}",
                        owner.identifier
                    ),
                )
            });
        return Check::Violations(taken.into_iter().chain(kind_problem).collect());
    }
    match (kind_problem, existing) {
        (Some(v), _) => Check::Violations(vec![v]),
        (None, Some(existing)) => Check::Existing(existing.clone()),
        (None, None) => Check::Violations(Vec::new()),
    }
}

/// The `kind` rule for something about to be written.
fn kind_violation(
    url: &str,
    metadata: Option<&AttachmentMetadata>,
    kinds: &[String],
) -> Option<Violation> {
    if kinds.is_empty() {
        return None;
    }
    let allowed = kinds.join(", ");
    match metadata.and_then(|m| m.get_str("kind")) {
        Some(kind) if kinds.iter().any(|k| k == kind) => None,
        Some(kind) => Some(Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceKindInvalid,
            Some(url.to_owned()),
            format!("the source's metadata.kind {kind:?} is not one of: {allowed} (pass --meta kind=<kind>)"),
        )),
        None => Some(Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceKindInvalid,
            Some(url.to_owned()),
            format!("the source needs a metadata.kind, one of: {allowed} (pass --meta kind=<kind>)"),
        )),
    }
}

/// `source-attachment` on an issue that already exists: it must carry an
/// http(s) attachment, and, when `kinds` is not empty, one whose
/// `metadata.kind` is among them.
pub(super) fn check_existing(issue: &Issue, kinds: &[String]) -> Vec<Violation> {
    let sources: Vec<_> = issue
        .attachments
        .iter()
        .filter(|a| is_http_url(a.url.trim()))
        .collect();
    if sources.is_empty() {
        return vec![Violation::new(
            Rule::SourceAttachment,
            ViolationKind::SourceRequired,
            Some(issue.identifier.clone()),
            format!("{} has no http(s) source attachment", issue.identifier),
        )];
    }
    if kinds.is_empty() {
        return Vec::new();
    }
    let has_kind = sources.iter().any(|a| {
        matches!(a.metadata.get("kind"), Some(Value::String(k)) if kinds.iter().any(|allowed| allowed == k))
    });
    if has_kind {
        return Vec::new();
    }
    vec![Violation::new(
        Rule::SourceAttachment,
        ViolationKind::SourceKindInvalid,
        Some(issue.identifier.clone()),
        format!(
            "{} has no source attachment with a metadata.kind of: {}",
            issue.identifier,
            kinds.join(", ")
        ),
    )]
}

/// Does re-creating an issue with the same source need to rewrite its
/// attachment? Only when metadata was given and differs from what is stored;
/// identical metadata sends nothing (the upsert would be a no-op).
pub fn metadata_needs_update(
    wanted: Option<&AttachmentMetadata>,
    stored: &serde_json::Map<String, Value>,
) -> bool {
    wanted.is_some_and(|w| !w.matches(stored))
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
