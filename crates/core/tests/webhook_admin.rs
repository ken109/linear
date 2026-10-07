//! Listing, creating and deleting webhooks: the requests, the responses they
//! decode, and how a reference picks one webhook. (Checking a delivery's
//! signature is `webhook.rs`.)

use chrono::{TimeZone, Utc};
use linear_core::inputs::{self, WebhookCreate, WebhookCreateInput, WebhookDelete};
use linear_core::matching::match_webhook;
use linear_core::read::{self, Webhooks};
use linear_core::types::{PageVars, Webhook};
use linear_core::wire::{build_request, parse_response, ResponseMeta};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(body: &str) -> T {
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    parse_response(&meta, body, now).unwrap()
}

fn webhooks() -> Vec<Webhook> {
    let list: Webhooks = parse(&fixture("webhooks"));
    list.webhooks.nodes
}

#[test]
fn the_list_query_pages_and_never_asks_for_the_secret() {
    let req = build_request(&read::webhooks(PageVars {
        first: 50,
        after: Some("cursor".into()),
    }));
    assert_eq!(req.operation_name.as_deref(), Some("Webhooks"));
    assert!(
        req.query.contains("webhooks(first: $first, after: $after)"),
        "{}",
        req.query
    );
    assert!(!req.query.contains("secret"), "{}", req.query);
    assert_eq!(req.variables["first"], 50);
    assert_eq!(req.variables["after"], "cursor");
}

#[test]
fn a_list_decodes_with_and_without_a_label_or_team() {
    let all = webhooks();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].label.as_deref(), Some("Deploy hook"));
    assert_eq!(all[0].team.as_ref().map(|t| t.key.as_str()), Some("EX"));
    assert_eq!(all[0].resource_types, ["Issue", "Comment"]);
    assert!(all[0].enabled && !all[0].all_public_teams);
    assert_eq!(all[1].label, None);
    assert!(all[1].team.is_none() && all[1].all_public_teams && !all[1].enabled);

    // `--json` is the same camelCase shape as every other entity.
    let json = serde_json::to_value(&all[0]).unwrap();
    assert_eq!(json["resourceTypes"][1], "Comment");
    assert_eq!(json["allPublicTeams"], false);
    assert!(json.get("secret").is_none());
}

#[test]
fn a_create_sends_only_what_was_given_and_reads_the_secret_under_an_alias() {
    let req = build_request(&inputs::webhook_create(WebhookCreateInput {
        url: "https://hooks.example.com/linear".into(),
        resource_types: vec!["Issue".into()],
        label: None,
        team_id: Some("team-id".into()),
        all_public_teams: None,
    }));
    assert_eq!(req.operation_name.as_deref(), Some("WebhookCreate"));
    assert_eq!(
        req.variables["input"],
        serde_json::json!({
            "url": "https://hooks.example.com/linear",
            "resourceTypes": ["Issue"],
            "teamId": "team-id",
        })
    );
    assert!(req.query.contains("signing: webhook"), "{}", req.query);

    let created: WebhookCreate = parse(
        r#"{"data":{"webhookCreate":{"success":true,
            "webhook":{"id":"w1","label":null,"url":"https://hooks.example.com/linear",
              "enabled":true,"resourceTypes":["Issue"],"allPublicTeams":false,"team":null,
              "createdAt":"2026-10-07T00:00:00.000Z"},
            "signing":{"secret":"lin_wh_abc"}}}}"#,
    );
    assert!(created.webhook_create.success);
    assert_eq!(
        created.webhook_create.signing.secret.as_deref(),
        Some("lin_wh_abc")
    );
}

#[test]
fn a_delete_names_the_webhook_by_id() {
    let req = build_request(&inputs::webhook_delete("w1"));
    assert_eq!(req.operation_name.as_deref(), Some("WebhookDelete"));
    assert_eq!(req.variables["id"], "w1");
    let done: WebhookDelete = parse(r#"{"data":{"webhookDelete":{"success":true}}}"#);
    assert!(done.webhook_delete.success);
}

// ------------------------------------------------------------------ matching

#[test]
fn a_webhook_is_found_by_id_label_or_url() {
    let all = webhooks();
    for reference in [
        "00000000-0000-4000-8000-0000000000c1",
        "Deploy hook",
        "deploy HOOK",
        "https://hooks.example.com/linear",
    ] {
        assert_eq!(
            match_webhook(&all, reference).unwrap().id.inner(),
            all[0].id.inner(),
            "{reference}"
        );
    }
    assert_eq!(
        match_webhook(&all, "https://other.example.com/hook")
            .unwrap()
            .id
            .inner(),
        all[1].id.inner()
    );
}

#[test]
fn an_unknown_or_ambiguous_reference_matches_nothing() {
    let mut all = webhooks();
    let err = match_webhook(&all, "nope").unwrap_err().to_string();
    assert!(err.contains("nope") && err.contains("Deploy hook"), "{err}");

    // Two webhooks delivering to one URL: the URL alone cannot say which.
    all[1].url = all[0].url.clone();
    let err = match_webhook(&all, "https://hooks.example.com/linear")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("ambiguous") && err.contains(all[1].id.inner()),
        "{err}"
    );
    // ... but the id still can.
    assert!(match_webhook(&all, all[1].id.inner()).is_ok());
}
