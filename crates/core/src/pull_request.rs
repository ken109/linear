//! GitHub pull requests as Linear's GitHub integration records them.
//!
//! Linear keeps no pull-request object that the API exposes on an issue (the
//! `PullRequest` type of the schema is internal and no field reaches it).
//! What the integration leaves on the issue is an *attachment*: its
//! `sourceType` is `github`, its `url` is the pull request's, and its
//! `metadata` is a free-form JSON object that the integration keeps in sync
//! with GitHub (status, review state, ...). The attachment the integration
//! makes for a linked GitHub *issue* has the same `sourceType`, so a pull
//! request is told apart by its URL (`.../pull/<number>`).
//!
//! The shape of `metadata` is the integration's, not a documented contract,
//! and no workspace we could read held a pull-request attachment when this was
//! written (see `tests/fixtures/README.md`). So nothing here insists on a key:
//! every field is read if it is there and in a form we understand, and
//! otherwise left unknown. A pull request whose state cannot be read is
//! [`PullRequestStatus::Unknown`], and no audit rule acts on it.

use crate::types::{Attachment, Issue};
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

/// The `sourceType` Linear gives an attachment its GitHub integration made.
pub const GITHUB_SOURCE_TYPE: &str = "github";

/// Where a pull request stands on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestStatus {
    Open,
    /// Open, and marked a draft.
    Draft,
    Merged,
    /// Closed without being merged.
    Closed,
    /// The metadata does not say (or says something this code does not know).
    Unknown,
}

impl PullRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Draft => "draft",
            Self::Merged => "merged",
            Self::Closed => "closed",
            Self::Unknown => "unknown",
        }
    }

    /// Is the pull request still waiting to be merged or closed (a draft included)?
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Open | Self::Draft)
    }
}

/// One pull request linked to an issue, read from its attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    /// The pull request number, from the URL.
    pub number: u64,
    pub title: String,
    pub status: PullRequestStatus,
    /// When it was opened: the metadata's `createdAt` (or `openedAt`) if
    /// there is one, else when the attachment was made, which is when Linear
    /// first saw the pull request.
    pub opened_at: DateTime<Utc>,
    pub merged_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// A GitHub pull request URL taken apart: `https://<host>/<owner>/<repo>/pull/<n>`,
/// with a query, a fragment or a trailing path such as `/files` dropped.
pub fn parse_pull_request_url(url: &str) -> Option<(String, u64)> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let path = rest.split(['?', '#']).next()?;
    let mut parts = path.split('/');
    let host = parts.next().filter(|h| !h.is_empty())?;
    let (owner, repo) = (parts.next()?, parts.next()?);
    if owner.is_empty() || repo.is_empty() || parts.next()? != "pull" {
        return None;
    }
    let number = parts.next()?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u64 = number.parse().ok().filter(|n| *n > 0)?;
    Some((
        format!("{scheme}://{host}/{owner}/{repo}/pull/{number}"),
        number,
    ))
}

/// The number of a GitHub pull request URL (see [`parse_pull_request_url`]).
pub fn pull_request_number(url: &str) -> Option<u64> {
    parse_pull_request_url(url).map(|(_, number)| number)
}

impl Attachment {
    /// This attachment as a GitHub pull request, or `None` when it is anything else
    /// (another integration, a linked GitHub issue, a plain link).
    pub fn pull_request(&self) -> Option<PullRequest> {
        if self.source_type.as_deref() != Some(GITHUB_SOURCE_TYPE) {
            return None;
        }
        let number = pull_request_number(&self.url)?;
        let meta = &self.metadata;
        let merged_at = time(meta, &["mergedAt"]);
        let closed_at = time(meta, &["closedAt"]);
        Some(PullRequest {
            url: self.url.clone(),
            number,
            title: meta
                .get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(&self.title)
                .to_owned(),
            status: status(meta, merged_at, closed_at),
            opened_at: time(meta, &["createdAt", "openedAt"]).unwrap_or(self.created_at),
            merged_at,
            closed_at,
        })
    }
}

impl Issue {
    /// The GitHub pull requests the integration linked to this issue.
    ///
    /// Only the attachments the issue fragment holds (the first ten) are seen.
    pub fn pull_requests(&self) -> Vec<PullRequest> {
        self.attachments
            .iter()
            .filter_map(Attachment::pull_request)
            .collect()
    }
}

/// `metadata.status` if it names a state we know; else what the timestamps
/// and the `draft` flag say; else unknown.
fn status(
    meta: &Map<String, Value>,
    merged_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
) -> PullRequestStatus {
    let draft = meta.get("draft").or(meta.get("isDraft")) == Some(&Value::Bool(true));
    let named = meta
        .get("status")
        .or(meta.get("state"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase());
    match named.as_deref() {
        Some("merged") => return PullRequestStatus::Merged,
        Some("closed") => return PullRequestStatus::Closed,
        Some("draft") => return PullRequestStatus::Draft,
        Some("open") => {
            return if draft {
                PullRequestStatus::Draft
            } else {
                PullRequestStatus::Open
            }
        }
        _ => {}
    }
    if merged_at.is_some() {
        PullRequestStatus::Merged
    } else if closed_at.is_some() {
        PullRequestStatus::Closed
    } else if draft {
        PullRequestStatus::Draft
    } else {
        PullRequestStatus::Unknown
    }
}

/// The first of `keys` that holds a time: an RFC 3339 string or epoch milliseconds.
fn time(meta: &Map<String, Value>, keys: &[&str]) -> Option<DateTime<Utc>> {
    keys.iter().find_map(|key| match meta.get(*key)? {
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|t| t.to_utc()),
        Value::Number(n) => Utc.timestamp_millis_opt(n.as_i64()?).single(),
        _ => None,
    })
}
