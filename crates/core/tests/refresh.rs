//! `decide_refresh`: TTL, events, backoff and schema changes. `now` is fixed.

use chrono::{DateTime, Duration, TimeZone, Utc};
use linear_core::cache::{Failure, RefreshStatus, WorkspaceCache};
use linear_core::refresh::{
    decide_refresh, Freshness, RefreshEvent, RefreshMeta, RefreshReason, DEFAULT_TTL_SECS,
};
use linear_core::SCHEMA_VERSION;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}

fn fetched_ago(secs: i64) -> RefreshMeta {
    RefreshMeta {
        fetched_at: Some(now() - Duration::seconds(secs)),
        ..RefreshMeta::empty()
    }
}

fn webhook(resource_type: &str) -> RefreshEvent {
    RefreshEvent::Webhook {
        resource_type: resource_type.into(),
        action: "update".into(),
    }
}

fn fresh(age_secs: u64) -> Freshness {
    Freshness::Fresh { age_secs }
}

fn expired(age_secs: u64) -> Freshness {
    Freshness::Expired { age_secs }
}

fn decision(meta: &RefreshMeta, event: &RefreshEvent) -> (bool, RefreshReason, Freshness) {
    let d = decide_refresh(meta, event, now());
    (d.refresh, d.reason, d.freshness)
}

#[test]
fn nothing_cached_is_refreshed_whatever_the_event() {
    for event in [RefreshEvent::Read, RefreshEvent::Cron, webhook("Comment")] {
        assert_eq!(
            decision(&RefreshMeta::empty(), &event),
            (true, RefreshReason::NeverFetched, Freshness::Missing),
            "{event:?}"
        );
    }
}

#[test]
fn a_cache_from_another_schema_version_is_refreshed_and_missing() {
    let meta = RefreshMeta {
        schema_version: SCHEMA_VERSION + 1,
        ..fetched_ago(1)
    };
    assert_eq!(
        decision(&meta, &RefreshEvent::Read),
        (true, RefreshReason::SchemaChanged, Freshness::Missing)
    );
}

#[test]
fn a_cache_dated_in_the_future_counts_as_just_fetched() {
    // The same rule as `WorkspaceCache::freshness`.
    assert_eq!(
        decision(&fetched_ago(-30), &RefreshEvent::Read),
        (false, RefreshReason::Fresh, fresh(0))
    );
}

#[test]
fn a_read_refreshes_only_after_the_ttl() {
    let ttl = DEFAULT_TTL_SECS as i64;
    // Within the TTL means up to and including it, as in `WorkspaceCache::freshness`.
    assert_eq!(
        decision(&fetched_ago(ttl), &RefreshEvent::Read),
        (false, RefreshReason::Fresh, fresh(ttl as u64))
    );
    assert_eq!(
        decision(&fetched_ago(ttl + 1), &RefreshEvent::Read),
        (true, RefreshReason::TtlExpired, expired(ttl as u64 + 1))
    );
}

#[test]
fn the_ttl_can_be_set() {
    let meta = RefreshMeta {
        ttl_secs: Some(30),
        ..fetched_ago(31)
    };
    assert_eq!(
        decision(&meta, &RefreshEvent::Read),
        (true, RefreshReason::TtlExpired, expired(31))
    );
    let meta = RefreshMeta {
        ttl_secs: Some(3600),
        ..fetched_ago(600)
    };
    assert_eq!(
        decision(&meta, &RefreshEvent::Read),
        (false, RefreshReason::Fresh, fresh(600))
    );
}

#[test]
fn cron_refreshes_from_half_the_ttl() {
    let half = DEFAULT_TTL_SECS as i64 / 2;
    assert_eq!(
        decision(&fetched_ago(half - 1), &RefreshEvent::Cron),
        (false, RefreshReason::Fresh, fresh(half as u64 - 1))
    );
    assert_eq!(
        decision(&fetched_ago(half), &RefreshEvent::Cron),
        (true, RefreshReason::CronDue, fresh(half as u64))
    );
}

#[test]
fn a_relevant_webhook_refreshes_even_a_fresh_cache() {
    for ty in [
        "Issue",
        "IssueLabel",
        "Attachment",
        "Project",
        "ProjectMilestone",
        "ProjectUpdate",
        "Initiative",
    ] {
        assert_eq!(
            decision(&fetched_ago(1), &webhook(ty)),
            (true, RefreshReason::Webhook, fresh(1)),
            "{ty}"
        );
    }
}

#[test]
fn an_irrelevant_webhook_changes_nothing() {
    for ty in ["Comment", "Reaction", "Document", "Cycle", "SomethingNew"] {
        assert_eq!(
            decision(&fetched_ago(1), &webhook(ty)),
            (false, RefreshReason::IrrelevantEvent, fresh(1)),
            "{ty}"
        );
    }
    // Staleness is still reported: the webhook did not make the cache any newer.
    assert_eq!(
        decision(&fetched_ago(10_000), &webhook("Comment")),
        (false, RefreshReason::IrrelevantEvent, expired(10_000))
    );
}

#[test]
fn manual_always_refreshes() {
    assert_eq!(
        decision(&fetched_ago(1), &RefreshEvent::Manual),
        (true, RefreshReason::Manual, fresh(1))
    );
}

#[test]
fn a_recent_failure_backs_off_except_for_a_person() {
    let failed_ago = |secs: i64| RefreshMeta {
        last_failure_at: Some(now() - Duration::seconds(secs)),
        ..fetched_ago(10_000)
    };
    for event in [RefreshEvent::Read, RefreshEvent::Cron, webhook("Issue")] {
        assert_eq!(
            decision(&failed_ago(59), &event),
            (false, RefreshReason::BackingOff, expired(10_000)),
            "{event:?}"
        );
        assert!(decision(&failed_ago(60), &event).0, "{event:?}");
    }
    assert_eq!(
        decision(&failed_ago(1), &RefreshEvent::Manual),
        (true, RefreshReason::Manual, expired(10_000))
    );
}

#[test]
fn the_backoff_can_be_set() {
    let meta = RefreshMeta {
        last_failure_at: Some(now() - Duration::seconds(100)),
        failure_backoff_secs: Some(300),
        ..fetched_ago(10_000)
    };
    assert_eq!(
        decision(&meta, &RefreshEvent::Read),
        (false, RefreshReason::BackingOff, expired(10_000))
    );
}

#[test]
fn a_failure_does_not_make_an_old_cache_look_fresh() {
    // The snapshot is a week old; whatever happened since, it is expired.
    let meta = RefreshMeta {
        last_failure_at: Some(now() - Duration::seconds(5)),
        ..fetched_ago(7 * 24 * 3600)
    };
    let d = decide_refresh(&meta, &RefreshEvent::Read, now());
    assert_eq!(d.freshness, expired(7 * 24 * 3600));
}

#[test]
fn the_meta_of_a_cache_entry_carries_its_fetch_and_failure_times() {
    let mut entry = WorkspaceCache {
        schema_version: SCHEMA_VERSION,
        workspace: "ws".into(),
        status: RefreshStatus::Failed,
        attempted_at: now(),
        fetched_at: Some(now() - Duration::hours(1)),
        failure: Some(Failure {
            at: now() - Duration::seconds(5),
            message: "down".into(),
        }),
        data: None,
    };
    let meta = RefreshMeta::from(&entry);
    assert_eq!(meta.fetched_at, entry.fetched_at);
    assert_eq!(meta.last_failure_at, Some(now() - Duration::seconds(5)));
    assert_eq!(
        decision(&meta, &RefreshEvent::Read),
        (false, RefreshReason::BackingOff, expired(3600))
    );

    entry.failure = None;
    assert_eq!(RefreshMeta::from(&entry).last_failure_at, None);
}

#[test]
fn the_event_and_meta_parse_from_json() {
    let meta: RefreshMeta =
        serde_json::from_str(r#"{"schemaVersion": 1, "fetchedAt": "2026-10-20T11:00:00Z"}"#)
            .unwrap();
    assert_eq!(meta.fetched_at, Some(now() - Duration::hours(1)));
    let event: RefreshEvent =
        serde_json::from_str(r#"{"kind":"webhook","resourceType":"Issue","action":"create"}"#)
            .unwrap();
    assert_eq!(
        event,
        RefreshEvent::Webhook {
            resource_type: "Issue".into(),
            action: "create".into()
        }
    );
    let event: RefreshEvent = serde_json::from_str(r#"{"kind":"read"}"#).unwrap();
    assert_eq!(event, RefreshEvent::Read);
}

#[test]
fn the_decision_serializes_with_the_freshness_the_cache_uses() {
    let d = decide_refresh(&fetched_ago(7), &RefreshEvent::Read, now());
    assert_eq!(
        serde_json::to_value(d).unwrap(),
        serde_json::json!({
            "refresh": false,
            "reason": "fresh",
            "freshness": { "state": "fresh", "ageSecs": 7 },
        })
    );
}
