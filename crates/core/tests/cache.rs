//! What a refresh stores, what a failed one leaves, and when an entry is
//! unknown. `now` is passed in everywhere.

mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use linear_core::audit::{audit, AuditConfig, Finding};
use linear_core::cache::{
    Fetched, Freshness, RefreshStatus, WorkspaceCache, DEFAULT_TTL_SECS, SCHEMA_VERSION,
};
use linear_core::types::User;
use serde_json::json;

fn viewer() -> User {
    serde_json::from_value(json!({
        "id": "u-me", "name": "Me", "displayName": "me",
        "email": "me@example.com", "active": true, "isMe": true,
    }))
    .unwrap()
}

/// Findings for issues `ids`, each overdue (so each yields one finding).
fn findings(ids: &[&str]) -> Vec<Finding> {
    let issues = ids
        .iter()
        .map(|id| issue(id).mine().due("2026-10-01").build())
        .collect();
    audit(&snapshot(issues, vec![]), &AuditConfig::default(), now()).findings
}

fn fetched(findings: Vec<Finding>) -> Fetched {
    Fetched {
        viewer: viewer(),
        issues: vec![issue("KK-1").mine().state("started").build()],
        projects: vec![],
        findings,
    }
}

fn at(minutes: i64) -> DateTime<Utc> {
    now() + Duration::minutes(minutes)
}

fn idents(fs: &[Finding]) -> Vec<&str> {
    fs.iter().map(|f| f.target.identifier.as_str()).collect()
}

// ------------------------------------------------------------------ refreshed

#[test]
fn the_first_refresh_reports_everything_as_new() {
    let c = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1", "KK-2"])), now());
    assert_eq!(c.schema_version, SCHEMA_VERSION);
    assert_eq!(c.workspace, WS);
    assert_eq!(c.status, RefreshStatus::Ok);
    assert_eq!(c.fetched_at, Some(now()));
    assert_eq!(c.attempted_at, now());
    assert!(c.failure.is_none());
    let data = c.data.unwrap();
    assert_eq!(data.viewer.id.inner(), "u-me");
    assert_eq!(data.issues.len(), 1);
    assert_eq!(idents(&data.findings), ["KK-1", "KK-2"]);
    assert_eq!(idents(&data.new_findings), ["KK-1", "KK-2"]);
}

#[test]
fn only_findings_that_were_not_there_before_are_new() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let second = WorkspaceCache::refreshed(
        Some(&first),
        WS,
        fetched(findings(&["KK-1", "KK-2"])),
        at(5),
    );
    let data = second.data.unwrap();
    assert_eq!(idents(&data.findings), ["KK-1", "KK-2"]);
    assert_eq!(idents(&data.new_findings), ["KK-2"]);
}

#[test]
fn an_unchanged_audit_has_nothing_new_even_when_the_message_moved_on() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    // The same finding a day later: "1 day overdue" becomes "2 days overdue".
    let mut later = findings(&["KK-1"]);
    later[0].message = "KK-1 is 2 days overdue".to_owned();
    assert_ne!(later, first.data.as_ref().unwrap().findings);
    let second = WorkspaceCache::refreshed(Some(&first), WS, fetched(later), at(5));
    assert!(second.data.unwrap().new_findings.is_empty());
}

#[test]
fn a_finding_that_went_away_and_came_back_is_new_again() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let gone = WorkspaceCache::refreshed(Some(&first), WS, fetched(vec![]), at(5));
    assert!(gone.data.as_ref().unwrap().new_findings.is_empty());
    let back = WorkspaceCache::refreshed(Some(&gone), WS, fetched(findings(&["KK-1"])), at(10));
    assert_eq!(idents(&back.data.unwrap().new_findings), ["KK-1"]);
}

#[test]
fn a_refresh_clears_an_earlier_failure() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let failed = WorkspaceCache::failed(Some(&first), WS, "boom", at(5));
    let ok = WorkspaceCache::refreshed(Some(&failed), WS, fetched(findings(&["KK-1"])), at(10));
    assert_eq!(ok.status, RefreshStatus::Ok);
    assert!(ok.failure.is_none());
    // The snapshot kept through the failure is what the new one is compared with.
    assert!(ok.data.unwrap().new_findings.is_empty());
}

// ---------------------------------------------------------------------- failed

#[test]
fn a_failed_refresh_keeps_the_old_snapshot_and_records_the_failure() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let failed = WorkspaceCache::failed(Some(&first), WS, "HTTP 500", at(5));

    assert_eq!(failed.status, RefreshStatus::Failed);
    assert_eq!(failed.attempted_at, at(5));
    // The snapshot and its time are the old ones, untouched.
    assert_eq!(failed.fetched_at, Some(now()));
    assert_eq!(failed.data, first.data);
    let failure = failed.failure.unwrap();
    assert_eq!(failure.at, at(5));
    assert_eq!(failure.message, "HTTP 500");
}

#[test]
fn a_failure_with_nothing_before_it_leaves_no_snapshot() {
    let failed = WorkspaceCache::failed(None, WS, "no credentials", now());
    assert!(failed.data.is_none());
    assert!(failed.fetched_at.is_none());
    assert_eq!(
        failed.freshness(now(), DEFAULT_TTL_SECS),
        Freshness::Missing
    );
}

#[test]
fn a_second_failure_replaces_the_first_one_but_not_the_snapshot() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let one = WorkspaceCache::failed(Some(&first), WS, "one", at(5));
    let two = WorkspaceCache::failed(Some(&one), WS, "two", at(10));
    assert_eq!(two.failure.unwrap().message, "two");
    assert_eq!(two.fetched_at, Some(now()));
    assert_eq!(two.data, first.data);
}

// ------------------------------------------------------------------- freshness

#[test]
fn an_entry_is_fresh_until_the_ttl_has_passed() {
    let c = WorkspaceCache::refreshed(None, WS, fetched(vec![]), now());
    let ttl = DEFAULT_TTL_SECS;
    assert_eq!(ttl, 300);
    assert_eq!(c.freshness(now(), ttl), Freshness::Fresh { age_secs: 0 });
    assert_eq!(
        c.freshness(now() + Duration::seconds(300), ttl),
        Freshness::Fresh { age_secs: 300 }
    );
    assert_eq!(
        c.freshness(now() + Duration::seconds(301), ttl),
        Freshness::Expired { age_secs: 301 }
    );
}

#[test]
fn an_expired_entry_is_unknown_not_healthy() {
    let c = WorkspaceCache::refreshed(None, WS, fetched(vec![]), now());
    assert!(c.known(at(4), DEFAULT_TTL_SECS).is_some());
    assert!(c.known(at(6), DEFAULT_TTL_SECS).is_none());
    // The data is still there for `cache show`; it just is not vouched for.
    assert!(c.data.is_some());
}

#[test]
fn a_failed_refresh_does_not_make_an_old_snapshot_look_new() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(vec![]), now());
    let failed = WorkspaceCache::failed(Some(&first), WS, "boom", at(10));
    assert_eq!(
        failed.freshness(at(10), DEFAULT_TTL_SECS),
        Freshness::Expired { age_secs: 600 }
    );
}

#[test]
fn the_ttl_is_a_parameter() {
    let c = WorkspaceCache::refreshed(None, WS, fetched(vec![]), now());
    assert!(c.known(at(1), 30).is_none());
    assert!(c.known(at(1), 61).is_some());
}

#[test]
fn a_time_in_the_future_counts_as_just_fetched() {
    let c = WorkspaceCache::refreshed(None, WS, fetched(vec![]), at(3));
    assert_eq!(c.freshness(now(), 300), Freshness::Fresh { age_secs: 0 });
}

/// `decide_refresh` (what a Worker and `cache refresh` ask) and the entry's own
/// `freshness` (what `--cached` reads ask) are one calculation: they agree at
/// every age, around the TTL and under clock skew.
#[test]
fn the_entry_and_the_refresh_decision_agree_on_freshness() {
    use linear_core::refresh::{decide_refresh, RefreshEvent, RefreshMeta};
    let c = WorkspaceCache::refreshed(None, WS, fetched(vec![]), now());
    for ttl in [30, 300] {
        for secs in [-120, 0, 29, 30, 31, 299, 300, 301, 100_000] {
            let at = now() + Duration::seconds(secs);
            let meta = RefreshMeta {
                ttl_secs: Some(ttl),
                ..RefreshMeta::from(&c)
            };
            assert_eq!(
                decide_refresh(&meta, &RefreshEvent::Read, at).freshness,
                c.freshness(at, ttl),
                "ttl {ttl}, {secs}s later"
            );
        }
    }
}

// ----------------------------------------------------------------------- shape

#[test]
fn the_entry_round_trips_through_json_with_snake_case_names() {
    let first = WorkspaceCache::refreshed(None, WS, fetched(findings(&["KK-1"])), now());
    let failed = WorkspaceCache::failed(Some(&first), WS, "boom", at(5));
    for c in [first, failed] {
        let text = serde_json::to_string(&c).unwrap();
        let back: WorkspaceCache = serde_json::from_str(&text).unwrap();
        assert_eq!(back, c);
    }

    let v = serde_json::to_value(WorkspaceCache::failed(None, WS, "boom", now())).unwrap();
    assert_eq!(v["schemaVersion"], SCHEMA_VERSION);
    assert_eq!(v["status"], "failed");
    assert_eq!(v["fetchedAt"], serde_json::Value::Null);
    assert_eq!(v["failure"]["message"], "boom");
    assert_eq!(v["workspace"], WS);
}

#[test]
fn freshness_serializes_with_a_state_tag() {
    let v = serde_json::to_value(Freshness::Fresh { age_secs: 4 }).unwrap();
    assert_eq!(v, json!({ "state": "fresh", "ageSecs": 4 }));
    let v = serde_json::to_value(Freshness::Missing).unwrap();
    assert_eq!(v, json!({ "state": "missing" }));
}
