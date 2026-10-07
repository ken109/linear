//! What the cache holds and the decisions made about it.
//!
//! The cache is for readers that must not wait for Linear (a hook, a
//! statusline, a Worker): one entry per workspace, holding what the viewer
//! looks at and the result of the last `audit`. Reading and writing it is the
//! caller's job; this module only decides, from a `now` the caller hands in:
//!
//! * what a refresh stores ([`WorkspaceCache::refreshed`]), including which
//!   findings are new since the previous one;
//! * what a failed refresh leaves behind ([`WorkspaceCache::failed`]): the old
//!   snapshot is kept, never replaced by an empty one, and the failure is
//!   recorded next to it;
//! * whether an entry may be trusted ([`WorkspaceCache::freshness`]): past the
//!   TTL it is unknown, and readers must not show it as healthy.
//!
//! A change to the shape of an entry needs a new [`SCHEMA_VERSION`]; the CLI
//! and a Worker are updated separately, so a reader that meets another version
//! treats the entry as missing rather than guessing.

use crate::audit::{diff, Finding};
use crate::types::{Issue, Project, User};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The version of the entry's shape: the crate-wide [`crate::SCHEMA_VERSION`],
/// since an entry holds issues, projects and findings.
pub use crate::SCHEMA_VERSION;

/// How long an entry stays trustworthy, in seconds.
pub const DEFAULT_TTL_SECS: u64 = 300;

/// How the last refresh went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RefreshStatus {
    Ok,
    Failed,
}

/// Why the last refresh failed, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Failure {
    pub at: DateTime<Utc>,
    pub message: String,
}

/// What the viewer looks at in one workspace, as of one successful fetch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Mine {
    /// The authenticated user ("me") in this workspace.
    pub viewer: User,
    /// Issues assigned to the viewer whose state type is `started`.
    pub issues: Vec<Issue>,
    /// The projects those issues belong to.
    pub projects: Vec<Project>,
    /// The result of the audit.
    pub findings: Vec<Finding>,
    /// The findings that were not in the previous snapshot's audit: what a
    /// notifier should report once.
    pub new_findings: Vec<Finding>,
}

/// What a successful fetch brings back.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub viewer: User,
    pub issues: Vec<Issue>,
    pub projects: Vec<Project>,
    pub findings: Vec<Finding>,
}

/// The cache entry of one workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCache {
    pub schema_version: u32,
    pub workspace: String,
    /// How the last attempt went.
    pub status: RefreshStatus,
    /// When the last attempt finished, whether it worked or not.
    pub attempted_at: DateTime<Utc>,
    /// When `data` was fetched. `None` while no fetch has ever succeeded.
    pub fetched_at: Option<DateTime<Utc>>,
    /// The last failure. Kept until a later refresh succeeds.
    pub failure: Option<Failure>,
    /// The last successful snapshot. `None` while no fetch has ever succeeded.
    pub data: Option<Mine>,
}

/// Whether an entry may be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    tag = "state",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Freshness {
    /// Fetched within the TTL.
    Fresh { age_secs: u64 },
    /// Fetched, but longer ago than the TTL: treat as unknown.
    Expired { age_secs: u64 },
    /// Never fetched successfully.
    Missing,
}

impl Freshness {
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh { .. })
    }
}

impl WorkspaceCache {
    /// The entry after a successful fetch.
    ///
    /// `previous` is the entry being replaced, if there was a readable one of
    /// this schema version; what its audit already reported is not new.
    pub fn refreshed(
        previous: Option<&WorkspaceCache>,
        workspace: &str,
        fetched: Fetched,
        now: DateTime<Utc>,
    ) -> Self {
        let before: &[Finding] = previous
            .and_then(|p| p.data.as_ref())
            .map_or(&[], |d| d.findings.as_slice());
        let new_findings = diff(before, &fetched.findings);
        Self {
            schema_version: SCHEMA_VERSION,
            workspace: workspace.to_owned(),
            status: RefreshStatus::Ok,
            attempted_at: now,
            fetched_at: Some(now),
            failure: None,
            data: Some(Mine {
                viewer: fetched.viewer,
                issues: fetched.issues,
                projects: fetched.projects,
                findings: fetched.findings,
                new_findings,
            }),
        }
    }

    /// The entry after a failed fetch: the previous snapshot stays as it was
    /// (an empty one would read as "nothing to do"), and the failure is
    /// recorded beside it.
    pub fn failed(
        previous: Option<&WorkspaceCache>,
        workspace: &str,
        message: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            workspace: workspace.to_owned(),
            status: RefreshStatus::Failed,
            attempted_at: now,
            fetched_at: previous.and_then(|p| p.fetched_at),
            failure: Some(Failure {
                at: now,
                message: message.into(),
            }),
            data: previous.and_then(|p| p.data.clone()),
        }
    }

    /// Is the snapshot within `ttl_secs` of `now`? A snapshot dated in the
    /// future (clock skew) counts as just fetched.
    pub fn freshness(&self, now: DateTime<Utc>, ttl_secs: u64) -> Freshness {
        let (Some(fetched_at), Some(_)) = (self.fetched_at, self.data.as_ref()) else {
            return Freshness::Missing;
        };
        let age_secs = (now - fetched_at).num_seconds().max(0) as u64;
        if age_secs <= ttl_secs {
            Freshness::Fresh { age_secs }
        } else {
            Freshness::Expired { age_secs }
        }
    }

    /// The snapshot, only while it is fresh. `None` means unknown.
    pub fn known(&self, now: DateTime<Utc>, ttl_secs: u64) -> Option<&Mine> {
        if self.freshness(now, ttl_secs).is_fresh() {
            self.data.as_ref()
        } else {
            None
        }
    }
}
