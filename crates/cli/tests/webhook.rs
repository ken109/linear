//! `linear webhook list|create|delete|verify` against a mock Linear that
//! answers by operation name, and `verify` offline.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const HOOK: &str = "00000000-0000-4000-8000-0000000000c1";
const OTHER_HOOK: &str = "00000000-0000-4000-8000-0000000000c2";
const TEAM_EX: &str = "00000000-0000-4000-8000-000000000004";

fn webhooks() -> Reply {
    ok(&fixture("webhooks"))
}

fn assert_no_mutation(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter()
            .all(|o| o != "WebhookCreate" && o != "WebhookDelete"),
        "a mutation was sent: {ops:?}"
    );
}

// ------------------------------------------------------------------ list

#[test]
fn list_shows_each_webhook_with_its_scope_and_no_secret() {
    let sb = workspace();
    let mock = Routed::start(vec![("Webhooks", vec![webhooks()])]);
    let o = run(&sb, &mock, &["webhook", "list"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("ID"), "{out}");
    assert!(
        out.contains("Deploy hook") && out.contains("https://hooks.example.com/linear"),
        "{out}"
    );
    assert!(
        out.contains("EX") && out.contains("all public teams"),
        "{out}"
    );
    assert!(out.contains("Issue,Comment"), "{out}");
    assert!(!out.contains("secret"), "{out}");
    assert_no_leak(&o);
    mock.assert_read_only();
    assert_no_mutation(&mock);
}

#[test]
fn list_json_is_tagged_with_the_workspace() {
    let sb = workspace();
    let mock = Routed::start(vec![("Webhooks", vec![webhooks()])]);
    let o = run(&sb, &mock, &["webhook", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["workspace"], "example");
    assert_eq!(v[0]["id"], HOOK);
    assert_eq!(v[0]["resourceTypes"], json!(["Issue", "Comment"]));
    assert_eq!(v[0]["team"]["key"], "EX");
    assert_eq!(v[1]["label"], Value::Null);
    assert_eq!(v[1]["allPublicTeams"], true);
    assert!(v[0].get("secret").is_none());
}

#[test]
fn list_quiet_prints_only_ids() {
    let sb = workspace();
    let mock = Routed::start(vec![("Webhooks", vec![webhooks()])]);
    let o = run(&sb, &mock, &["webhook", "list", "-q"]);
    assert_eq!(stdout(&o), format!("{HOOK}\n{OTHER_HOOK}\n"));
}

// ------------------------------------------------------------------ create

fn created(secret: Option<&str>) -> Reply {
    data(json!({ "webhookCreate": {
        "success": true,
        "webhook": {
            "id": "00000000-0000-4000-8000-0000000000c3", "label": "CI", "url": "https://ci.example.com/hook",
            "enabled": true, "resourceTypes": ["Issue", "Comment"], "allPublicTeams": false,
            "team": { "id": TEAM_EX, "key": "EX", "name": "Example" },
            "createdAt": "2026-10-07T00:00:00.000Z"
        },
        "signing": { "secret": secret }
    }}))
}

fn create_routes(reply: Reply) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("WebhookCreate", vec![reply]),
    ]
}

#[test]
fn create_for_a_team_resolves_the_team_and_prints_the_secret() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(Some("lin_wh_secret"))));
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://ci.example.com/hook",
            "--resource-types",
            "Issue,Comment",
            "--team",
            "ex",
            "--label",
            "CI",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["secret"], "lin_wh_secret");
    assert_eq!(v["label"], "CI");
    assert_eq!(v["team"]["key"], "EX");

    let ops = mock.ops();
    assert_eq!(ops.last().unwrap(), "WebhookCreate", "{ops:?}");
    assert_eq!(
        mock.of("WebhookCreate")[0]["input"],
        json!({
            "url": "https://ci.example.com/hook",
            "resourceTypes": ["Issue", "Comment"],
            "label": "CI",
            // The team's id, not what was typed.
            "teamId": TEAM_EX,
        })
    );
}

#[test]
fn create_for_all_public_teams_names_no_team() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(Some("lin_wh_secret"))));
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://ci.example.com/hook",
            "--resource-types",
            "Issue",
            "--all-public-teams",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("WebhookCreate")[0]["input"],
        json!({
            "url": "https://ci.example.com/hook",
            "resourceTypes": ["Issue"],
            "allPublicTeams": true,
        })
    );
    // The human output carries the secret; the note about it goes to stderr.
    let out = stdout(&o);
    assert!(
        out.contains("Secret:") && out.contains("lin_wh_secret"),
        "{out}"
    );
    assert!(stderr(&o).contains("signing secret"), "{}", stderr(&o));
}

#[test]
fn create_quiet_prints_the_id_and_no_note() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(Some("lin_wh_secret"))));
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://ci.example.com/hook",
            "--resource-types",
            "Issue",
            "--team",
            "EX",
            "-q",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "00000000-0000-4000-8000-0000000000c3\n");
    assert_eq!(stderr(&o), "");
}

#[test]
fn create_needs_a_team_or_all_public_teams_and_never_both() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(None)));
    let base = [
        "webhook",
        "create",
        "--url",
        "https://ci.example.com/hook",
        "--resource-types",
        "Issue",
    ];
    let o = run(&sb, &mock, &base);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let mut both = base.to_vec();
    both.extend(["--team", "EX", "--all-public-teams"]);
    let o = run(&sb, &mock, &both);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "nothing was sent: {:?}", mock.ops());
}

#[test]
fn create_with_an_unknown_team_sends_no_mutation() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(None)));
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://ci.example.com/hook",
            "--resource-types",
            "Issue",
            "--team",
            "NOPE",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("NOPE"), "{}", stderr(&o));
    assert_no_mutation(&mock);
}

#[test]
fn create_refuses_an_empty_url_or_resource_type_before_any_request() {
    let sb = workspace();
    let mock = Routed::start(create_routes(created(None)));
    for args in [
        ["--url", " ", "--resource-types", "Issue"],
        ["--url", "https://x.test", "--resource-types", "Issue,"],
    ] {
        let mut full = vec!["webhook", "create", "--all-public-teams"];
        full.extend(args);
        let o = run(&sb, &mock, &full);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

#[test]
fn create_passes_linears_refusal_on() {
    let sb = workspace();
    let mock = Routed::start(create_routes(graphql_error("Forbidden: admin only")));
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://ci.example.com/hook",
            "--resource-types",
            "Issue",
            "--all-public-teams",
        ],
    );
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("admin only"), "{}", stderr(&o));
}

// ------------------------------------------------------------------ delete

fn delete_routes() -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("Webhooks", vec![webhooks()]),
        (
            "WebhookDelete",
            vec![data(json!({ "webhookDelete": { "success": true } }))],
        ),
    ]
}

#[test]
fn delete_finds_a_webhook_by_label_and_deletes_its_id() {
    let sb = workspace();
    let mock = Routed::start(delete_routes());
    let o = run(&sb, &mock, &["webhook", "delete", "deploy hook", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["deleted"], true);
    assert_eq!(v["id"], HOOK);
    assert_eq!(v["label"], "Deploy hook");
    assert_eq!(mock.of("WebhookDelete"), vec![json!({ "id": HOOK })]);
}

#[test]
fn delete_finds_a_webhook_by_url_or_id() {
    let sb = workspace();
    let mock = Routed::start(delete_routes());
    let o = run(
        &sb,
        &mock,
        &["webhook", "delete", "https://other.example.com/hook", "-q"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), format!("{OTHER_HOOK}\n"));

    let mock = Routed::start(delete_routes());
    let o = run(&sb, &mock, &["webhook", "delete", HOOK]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("(deleted)"), "{}", stdout(&o));
    assert_eq!(mock.of("WebhookDelete"), vec![json!({ "id": HOOK })]);
}

#[test]
fn delete_of_an_unknown_webhook_deletes_nothing() {
    let sb = workspace();
    let mock = Routed::start(delete_routes());
    let o = run(&sb, &mock, &["webhook", "delete", "no such hook"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Deploy hook"),
        "lists what exists: {}",
        stderr(&o)
    );
    assert_no_mutation(&mock);
}

// ------------------------------------------------------------------ verify

const SECRET: &str = "lin_wh_testsecret";

/// Same body and signature as `crates/core/tests/webhook.rs`
/// (`printf %s "$BODY" | openssl dgst -sha256 -hmac "$SECRET"`).
const BODY: &str = r#"{"action":"update","type":"Issue","organizationId":"org-1","webhookTimestamp":1760000000000}"#;
const SIGNATURE: &str = "19882a77e4dae983f01a21fbe0a53dd8142d0d699c37633bc9258a6237b13830";
const SENT_MS: &str = "1760000000000";

fn verify(args: &[&str], body: &str, env: &[(&str, &str)]) -> std::process::Output {
    // No workspace, no config, no network: `verify` must not need any.
    let sb = Sandbox::new();
    let mut full = vec!["webhook", "verify"];
    full.extend(args);
    sb.run_stdin(&full, None, env, Some(body))
}

#[test]
fn a_valid_delivery_exits_0_and_describes_the_event() {
    let o = verify(
        &["--signature", SIGNATURE, "--at", SENT_MS],
        BODY,
        &[("LINEAR_WEBHOOK_SECRET", SECRET)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.starts_with("valid") && out.contains("update Issue"),
        "{out}"
    );

    let o = verify(
        &["--signature", SIGNATURE, "--at", SENT_MS, "--json"],
        BODY,
        &[("LINEAR_WEBHOOK_SECRET", SECRET)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout_json(&o),
        json!({"status": "valid", "event": {
            "action": "update", "type": "Issue", "organizationId": "org-1",
            "webhookTimestamp": 1760000000000_i64
        }})
    );
}

#[test]
fn the_signature_is_checked_against_the_exact_body() {
    let o = verify(
        &["--signature", SIGNATURE, "--at", SENT_MS, "--json"],
        &format!("{BODY}\n"),
        &[("LINEAR_WEBHOOK_SECRET", SECRET)],
    );
    assert_eq!(code(&o), 1);
    assert_eq!(
        stdout_json(&o),
        json!({"status": "invalid", "reason": "signature-mismatch"})
    );
    // The reason is also the error on stderr, as every failure is.
    assert!(stderr(&o).contains("does not match"), "{}", stderr(&o));
}

#[test]
fn a_wrong_secret_a_malformed_signature_and_a_stale_delivery_are_rejected() {
    let reason = |args: &[&str], secret: &str| {
        let mut full = vec!["--json"];
        full.extend(args);
        let o = verify(&full, BODY, &[("LINEAR_WEBHOOK_SECRET", secret)]);
        assert_eq!(code(&o), 1, "{}", stderr(&o));
        stdout_json(&o)["reason"].as_str().unwrap().to_owned()
    };
    assert_eq!(
        reason(&["--signature", SIGNATURE, "--at", SENT_MS], "other"),
        "signature-mismatch"
    );
    assert_eq!(
        reason(&["--signature", "not-hex", "--at", SENT_MS], SECRET),
        "malformed-signature"
    );
    // Right signature, but judged an hour after it was sent (and, with no --at, years after).
    assert_eq!(
        reason(&["--signature", SIGNATURE, "--at", "1760003600000"], SECRET),
        "stale-timestamp"
    );
    assert_eq!(
        reason(&["--signature", SIGNATURE], SECRET),
        "stale-timestamp"
    );
}

#[test]
fn the_secret_may_come_from_a_file_and_the_body_from_a_file() {
    let sb = Sandbox::new();
    let dir = sb.cwd();
    std::fs::write(dir.join("secret"), format!("{SECRET}\n")).unwrap();
    std::fs::write(dir.join("body.json"), BODY).unwrap();
    let o = sb.run(
        &[
            "webhook",
            "verify",
            "--signature",
            SIGNATURE,
            "--at",
            SENT_MS,
            "--secret-file",
            "secret",
            "--body-file",
            "body.json",
            "-q",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "valid\n");
}

#[test]
fn no_secret_is_a_usage_error_not_a_rejection() {
    let o = verify(&["--signature", SIGNATURE], BODY, &[]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("LINEAR_WEBHOOK_SECRET"),
        "{}",
        stderr(&o)
    );

    let o = verify(
        &["--signature", SIGNATURE],
        BODY,
        &[("LINEAR_WEBHOOK_SECRET", "  ")],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn a_missing_body_file_is_a_usage_error() {
    let o = verify(
        &["--signature", SIGNATURE, "--body-file", "nope.json"],
        "",
        &[("LINEAR_WEBHOOK_SECRET", SECRET)],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_of_create_and_delete_plans_the_mutation() {
    let sb = workspace();
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("Webhooks", vec![webhooks()]),
    ]);
    let v = plan(
        &run(
            &sb,
            &mock,
            &[
                "webhook",
                "create",
                "--url",
                "https://ci.example.com/hook",
                "--resource-types",
                "Issue,Comment",
                "--team",
                "ex",
                "--dry-run",
                "--json",
            ],
        ),
        &mock,
    );
    assert_eq!(v["command"], "webhook create");
    assert_eq!(planned(&v), ["WebhookCreate"]);
    assert_eq!(
        v["mutations"][0]["variables"]["input"],
        json!({
            "url": "https://ci.example.com/hook",
            "resourceTypes": ["Issue", "Comment"],
            "teamId": TEAM_EX,
        })
    );
    // No secret exists, so none is printed.
    assert!(!stdout(&run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://x.test/h",
            "--resource-types",
            "Issue",
            "--all-public-teams",
            "--dry-run"
        ]
    ))
    .contains("secret"));

    // The same usage errors as the real run: an unknown team (2), both a team and all teams (2).
    let o = run(
        &sb,
        &mock,
        &[
            "webhook",
            "create",
            "--url",
            "https://x.test/h",
            "--resource-types",
            "Issue",
            "--team",
            "nope",
            "--dry-run",
        ],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    let v = plan(
        &run(
            &sb,
            &mock,
            &["webhook", "delete", "Deploy hook", "--dry-run", "--json"],
        ),
        &mock,
    );
    assert_eq!(v["command"], "webhook delete");
    assert_eq!(planned(&v), ["WebhookDelete"]);
    assert_eq!(v["mutations"][0]["variables"], json!({ "id": HOOK }));
    let o = run(&sb, &mock, &["webhook", "delete", "nope", "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    mock.assert_read_only();
}
