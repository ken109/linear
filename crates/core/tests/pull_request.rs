//! Reading GitHub pull requests off an issue's attachments.

use chrono::{TimeZone, Utc};
use linear_core::pull_request::{pull_request_number, PullRequestStatus};
use linear_core::types::{Attachment, AttachmentNodes};
use serde_json::{json, Value};

fn fixture() -> Vec<Attachment> {
    let text = include_str!("fixtures/attachments_github.json");
    let nodes: AttachmentNodes = serde_json::from_str(text).unwrap();
    nodes.iter().cloned().collect()
}

fn attachment(url: &str, source_type: &str, metadata: Value) -> Attachment {
    serde_json::from_value(json!({
        "id": "a-1", "title": "A title", "subtitle": null, "url": url,
        "sourceType": source_type, "metadata": metadata, "createdAt": "2026-10-03T09:05:00Z",
    }))
    .unwrap()
}

#[test]
fn only_attachments_of_the_integration_that_point_at_a_pull_request_count() {
    let numbers: Vec<Option<u64>> = fixture()
        .iter()
        .map(|a| a.pull_request().map(|p| p.number))
        .collect();
    // The synced GitHub issue and the plain link to a pull request are not pull requests.
    assert_eq!(
        numbers,
        [None, Some(41), Some(42), Some(43), Some(44), Some(45), None]
    );
}

#[test]
fn the_status_is_what_the_metadata_says() {
    let statuses: Vec<PullRequestStatus> = fixture()
        .iter()
        .filter_map(|a| a.pull_request())
        .map(|p| p.status)
        .collect();
    assert_eq!(
        statuses,
        [
            PullRequestStatus::Open,
            PullRequestStatus::Draft,
            PullRequestStatus::Merged,
            PullRequestStatus::Closed,
            PullRequestStatus::Unknown,
        ]
    );
}

#[test]
fn times_come_from_the_metadata_and_the_attachment_stands_in_for_the_opening() {
    let all = fixture();
    let merged = all[3].pull_request().unwrap();
    assert_eq!(
        merged.opened_at,
        Utc.with_ymd_and_hms(2026, 10, 3, 9, 0, 0).unwrap()
    );
    assert_eq!(
        merged.merged_at,
        Some(Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap())
    );
    let unknown = all[5].pull_request().unwrap();
    assert_eq!(
        unknown.opened_at,
        Utc.with_ymd_and_hms(2026, 10, 3, 9, 5, 0).unwrap()
    );
    assert_eq!(unknown.merged_at, None);
}

#[test]
fn the_title_is_the_metadatas_else_the_attachments() {
    let all = fixture();
    assert_eq!(all[1].pull_request().unwrap().title, "Add the widget");
    assert_eq!(all[4].pull_request().unwrap().title, "Abandon the widget");
    let a = attachment(
        "https://github.com/o/r/pull/1",
        "github",
        json!({"title": "  "}),
    );
    assert_eq!(a.pull_request().unwrap().title, "A title");
}

#[test]
fn timestamps_and_states_are_read_in_the_forms_they_may_come_in() {
    // Epoch milliseconds, a state in capitals, `isDraft`.
    let a = attachment(
        "https://github.com/o/r/pull/7",
        "github",
        json!({"status": "Open", "isDraft": true, "createdAt": 1_790_000_000_000_i64}),
    );
    let p = a.pull_request().unwrap();
    assert_eq!(p.status, PullRequestStatus::Draft);
    assert_eq!(p.opened_at.timestamp_millis(), 1_790_000_000_000);

    // No `status`, but a merge time.
    let b = attachment(
        "https://github.com/o/r/pull/8",
        "github",
        json!({"mergedAt": "2026-10-05T09:00:00Z"}),
    );
    assert_eq!(b.pull_request().unwrap().status, PullRequestStatus::Merged);

    // Nonsense is unknown, never a guess.
    let c = attachment(
        "https://github.com/o/r/pull/9",
        "github",
        json!({"status": 3, "createdAt": "yesterday"}),
    );
    let p = c.pull_request().unwrap();
    assert_eq!(p.status, PullRequestStatus::Unknown);
    assert_eq!(
        p.opened_at,
        Utc.with_ymd_and_hms(2026, 10, 3, 9, 5, 0).unwrap()
    );
}

#[test]
fn the_number_comes_from_the_url() {
    for (url, want) in [
        ("https://github.com/o/r/pull/12", Some(12)),
        ("https://github.com/o/r/pull/12/files", Some(12)),
        ("https://github.com/o/r/pull/12?diff=split#x", Some(12)),
        ("https://ghe.example.com/o/r/pull/3", Some(3)),
        ("https://github.com/o/r/issues/12", None),
        ("https://github.com/o/r/pull/", None),
        ("https://github.com/o/r/pull/x1", None),
        ("https://github.com/o/r/pull/0", None),
        ("https://github.com/o/r/pulls", None),
        ("https://github.com/o/r", None),
        ("not a url", None),
    ] {
        assert_eq!(pull_request_number(url), want, "{url}");
    }
}

#[test]
fn a_pull_request_url_is_reduced_to_its_canonical_form() {
    use linear_core::pull_request::parse_pull_request_url;
    assert_eq!(
        parse_pull_request_url("  https://github.com/o/r/pull/12/files?x=1#y "),
        Some(("https://github.com/o/r/pull/12".to_owned(), 12))
    );
    assert_eq!(parse_pull_request_url("ftp://github.com/o/r/pull/12"), None);
    assert_eq!(parse_pull_request_url("github.com/o/r/pull/12"), None);
    assert_eq!(parse_pull_request_url("https:///o/r/pull/12"), None);
}
