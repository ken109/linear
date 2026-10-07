//! Deciding whether a cached snapshot should be refreshed.
//!
//! [`crate::cache`] says what an entry holds and whether it can be trusted;
//! this module answers the question a caller asks next: given what just
//! happened, should it fetch from Linear now? The CLI (`linear cache refresh`,
//! the statusline) and a Worker (webhooks and Cron) use the one decision. It is
//! a pure function of what is known about the cache ([`RefreshMeta`]), what
//! happened ([`RefreshEvent`]) and `now`: it never reads the clock, a file or a
//! KV namespace, and it does not fetch. The caller acts on the answer and
//! records the outcome.
//!
//! A stale value is never turned into a fresh one here: [`Freshness`] follows
//! from the age of the last *successful* fetch only, as in
//! [`crate::cache::WorkspaceCache::freshness`].

use crate::cache::WorkspaceCache;
use crate::SCHEMA_VERSION;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::cache::{Freshness, DEFAULT_TTL_SECS};

/// How long to leave Linear alone after a failed refresh.
pub const DEFAULT_FAILURE_BACKOFF_SECS: u64 = 60;

/// Webhook resource types that change what a snapshot holds (the issues,
/// projects, milestones, status updates and labels `audit` looks at). Anything
/// else (comments, reactions, documents, cycles, ...) leaves it as it was.
pub const RELEVANT_WEBHOOK_TYPES: &[&str] = &[
    "Issue",
    "IssueLabel",
    "Attachment",
    "Project",
    "ProjectMilestone",
    "ProjectUpdate",
    "Initiative",
];

/// What is known about a cached snapshot (a cache entry without its data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshMeta {
    /// The `schemaVersion` the snapshot was written with.
    pub schema_version: u32,
    /// When the last *successful* fetch finished. `None`: never fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<DateTime<Utc>>,
    /// When the most recent fetch failed, if the latest attempt failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<DateTime<Utc>>,
    /// Seconds a snapshot stays fresh. Default [`DEFAULT_TTL_SECS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
    /// Seconds to wait after a failure. Default [`DEFAULT_FAILURE_BACKOFF_SECS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_backoff_secs: Option<u64>,
}

impl RefreshMeta {
    /// The meta of a cache that does not exist yet.
    pub fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            fetched_at: None,
            last_failure_at: None,
            ttl_secs: None,
            failure_backoff_secs: None,
        }
    }

    fn ttl(&self) -> u64 {
        self.ttl_secs.unwrap_or(DEFAULT_TTL_SECS)
    }

    fn backoff(&self) -> u64 {
        self.failure_backoff_secs
            .unwrap_or(DEFAULT_FAILURE_BACKOFF_SECS)
    }
}

impl From<&WorkspaceCache> for RefreshMeta {
    fn from(entry: &WorkspaceCache) -> Self {
        Self {
            schema_version: entry.schema_version,
            fetched_at: entry.fetched_at,
            last_failure_at: entry.failure.as_ref().map(|f| f.at),
            ttl_secs: None,
            failure_backoff_secs: None,
        }
    }
}

/// What prompted the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum RefreshEvent {
    /// A reader wants the snapshot (a hook, the statusline, a page load).
    Read,
    /// A scheduled run. It is a safety net behind webhooks, so it refreshes
    /// once the snapshot is half way to stale.
    Cron,
    /// A verified webhook delivery arrived.
    Webhook {
        /// The delivery's resource type (`WebhookEvent::resource_type`).
        resource_type: String,
        action: String,
    },
    /// A person asked for a refresh (`linear cache refresh`).
    Manual,
}

/// Why the decision is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshReason {
    /// Refresh: there is nothing cached.
    NeverFetched,
    /// Refresh: the cache was written by a different `schemaVersion`, so it
    /// is treated as missing.
    SchemaChanged,
    /// Refresh: a person asked.
    Manual,
    /// Refresh: a webhook says something the snapshot holds changed.
    Webhook,
    /// Refresh: the snapshot is older than the TTL.
    TtlExpired,
    /// Refresh: a scheduled run found the snapshot half way to stale.
    CronDue,
    /// Leave it: the snapshot is within the TTL.
    Fresh,
    /// Leave it: the webhook is about something a snapshot does not hold.
    IrrelevantEvent,
    /// Leave it: the last fetch failed a moment ago; try again later.
    BackingOff,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshDecision {
    /// Fetch from Linear now.
    pub refresh: bool,
    pub reason: RefreshReason,
    /// What a reader should make of the *current* contents, whatever is
    /// decided about refreshing them.
    pub freshness: Freshness,
}

/// Decide whether to refresh.
///
/// Order of precedence: a cache that cannot be used is always refreshed; then
/// a person's [`RefreshEvent::Manual`] request; then a recent failure backs off
/// (so a failing Linear is not hammered); then the event decides.
pub fn decide_refresh(
    meta: &RefreshMeta,
    event: &RefreshEvent,
    now: DateTime<Utc>,
) -> RefreshDecision {
    let decide = |refresh, reason, freshness| RefreshDecision {
        refresh,
        reason,
        freshness,
    };

    if meta.schema_version != SCHEMA_VERSION {
        return decide(true, RefreshReason::SchemaChanged, Freshness::Missing);
    }
    let Some(fetched_at) = meta.fetched_at else {
        return decide(true, RefreshReason::NeverFetched, Freshness::Missing);
    };

    let ttl = meta.ttl();
    let freshness = Freshness::of_age(fetched_at, now, ttl);
    let age_secs = freshness.age_secs().unwrap_or_default();

    if *event == RefreshEvent::Manual {
        return decide(true, RefreshReason::Manual, freshness);
    }
    if let Some(failed) = meta.last_failure_at {
        let since_failure = (now - failed).num_seconds().max(0) as u64;
        if since_failure < meta.backoff() {
            return decide(false, RefreshReason::BackingOff, freshness);
        }
    }

    match event {
        RefreshEvent::Manual => unreachable!("handled above"),
        RefreshEvent::Webhook { resource_type, .. } => {
            if RELEVANT_WEBHOOK_TYPES.contains(&resource_type.as_str()) {
                decide(true, RefreshReason::Webhook, freshness)
            } else {
                decide(false, RefreshReason::IrrelevantEvent, freshness)
            }
        }
        RefreshEvent::Read if age_secs > ttl => decide(true, RefreshReason::TtlExpired, freshness),
        RefreshEvent::Cron if age_secs >= ttl / 2 => {
            decide(true, RefreshReason::CronDue, freshness)
        }
        RefreshEvent::Read | RefreshEvent::Cron => decide(false, RefreshReason::Fresh, freshness),
    }
}
