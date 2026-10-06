//! Credential types: no leaks through `Debug`, header construction, file format.

use chrono::{TimeZone, Utc};
use linear_core::auth::*;

const KEY: &str = "lin_api_SECRETVALUE";

#[test]
fn secrets_are_redacted_in_debug_output() {
    let s = Secret::new(KEY);
    assert!(!format!("{s:?}").contains(KEY));

    let c = Credential::ApiKey { api_key: s };
    assert!(!format!("{c:?}").contains(KEY));
    assert!(!format!("{c:#?}").contains(KEY));

    let o = Credential::Oauth {
        access_token: Secret::new("access-secret"),
        refresh_token: Some(Secret::new("refresh-secret")),
        expires_at: None,
    };
    let dbg = format!("{o:?}");
    assert!(!dbg.contains("access-secret") && !dbg.contains("refresh-secret"));
}

#[test]
fn api_keys_are_sent_bare_and_oauth_tokens_as_bearer() {
    let k = Credential::ApiKey {
        api_key: Secret::new(KEY),
    };
    assert_eq!(k.authorization(), KEY);
    assert_eq!(k.method(), AuthMethod::ApiKey);

    let o = Credential::Oauth {
        access_token: Secret::new("tok"),
        refresh_token: None,
        expires_at: None,
    };
    assert_eq!(o.authorization(), "Bearer tok");
    assert_eq!(o.method(), AuthMethod::Oauth);
}

#[test]
fn credential_file_format_is_stable() {
    let k = Credential::ApiKey {
        api_key: Secret::new("k"),
    };
    let v = serde_json::to_value(&k).unwrap();
    assert_eq!(v, serde_json::json!({ "kind": "api-key", "api_key": "k" }));
    let back: Credential = serde_json::from_value(v).unwrap();
    assert_eq!(back, k);

    let o: Credential = serde_json::from_str(
        r#"{"kind":"oauth","access_token":"a","refresh_token":"r","expires_at":"2026-10-06T12:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(o.method(), AuthMethod::Oauth);

    // A file with no recognisable kind is rejected, not guessed at.
    assert!(serde_json::from_str::<Credential>(r#"{"api_key":"k"}"#).is_err());
    assert!(serde_json::from_str::<Credential>(r#"{"kind":"password","x":"y"}"#).is_err());
}

#[test]
fn oauth_expiry_is_judged_against_the_given_now() {
    let t = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    let c = Credential::Oauth {
        access_token: Secret::new("a"),
        refresh_token: None,
        expires_at: Some(t),
    };
    assert!(!c.is_expired(t - chrono::Duration::seconds(1)));
    assert!(c.is_expired(t));
    assert!(c.is_expired(t + chrono::Duration::hours(1)));

    let never = Credential::ApiKey {
        api_key: Secret::new("k"),
    };
    assert!(!never.is_expired(t));
}

#[test]
fn env_var_names_follow_the_workspace_name() {
    assert_eq!(api_key_env_var("sandbox"), "LINEAR_API_KEY_SANDBOX");
    assert_eq!(api_key_env_var("lt-three"), "LINEAR_API_KEY_LT_THREE");
    assert_eq!(api_key_env_var("my_ws2"), "LINEAR_API_KEY_MY_WS2");

    assert_eq!(CLIENT_SECRET_ENV, "LINEAR_CLIENT_SECRET");
    assert_eq!(CLIENT_ID_ENV, "LINEAR_CLIENT_ID");
    assert_eq!(
        client_secret_env_var("lt-three"),
        "LINEAR_CLIENT_SECRET_LT_THREE"
    );
    assert_eq!(client_id_env_var("lt-three"), "LINEAR_CLIENT_ID_LT_THREE");
}

// ------------------------------------------------------------ client credentials

const SECRET: &str = "lin_oauth_SECRETVALUE";

fn app() -> Credential {
    Credential::ClientCredentials {
        client_id: "the-id".into(),
        client_secret: Secret::new(SECRET),
    }
}

#[test]
fn client_credentials_never_leak_and_never_reach_a_file() {
    let c = app();
    assert_eq!(c.method(), AuthMethod::ClientCredentials);
    assert!(!format!("{c:?}").contains(SECRET));
    assert!(!format!("{c:#?}").contains(SECRET));
    // No header of its own: the secret cannot be sent by mistake.
    assert_eq!(c.authorization(), "");
    assert!(!c.is_expired(Utc::now()));

    // It cannot be written to a credentials file, nor read from one.
    assert!(serde_json::to_string(&c).is_err());
    assert!(serde_json::from_str::<Credential>(
        r#"{"kind":"client-credentials","client_id":"x","client_secret":"y"}"#
    )
    .is_err());
}

#[test]
fn the_grant_is_a_form_with_the_scopes_the_old_tool_used() {
    let body = client_credentials_form("the-id", &Secret::new("s e&cret/+="), APP_SCOPE);
    assert_eq!(
        body.expose(),
        "grant_type=client_credentials&client_id=the-id&client_secret=s%20e%26cret%2F%2B%3D\
         &scope=read%2Cwrite%2Cinitiative%3Awrite"
    );
    // The body holds the secret, so it is a Secret too.
    assert!(!format!("{body:?}").contains("client_secret"));
    assert_eq!(APP_SCOPE, "read,write,initiative:write");
    assert_eq!(TOKEN_URL, "https://api.linear.app/oauth/token");
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 0, 0, 0).unwrap()
}

#[test]
fn a_token_answer_becomes_a_token_with_an_expiry() {
    let secret = Secret::new(SECRET);
    let t = parse_token_response(
        200,
        r#"{"access_token":"tok","token_type":"Bearer","expires_in":2592000,"scope":"read"}"#,
        now(),
        &secret,
    )
    .unwrap();
    assert_eq!(t.bearer(), "Bearer tok");
    assert_eq!(t.expires_at(), now() + chrono::Duration::seconds(2_592_000));
    assert!(!format!("{t:?}").contains("tok\""));

    // No lifetime given: an hour is assumed, not forever.
    let t = parse_token_response(200, r#"{"access_token":"tok"}"#, now(), &secret).unwrap();
    assert_eq!(t.expires_at(), now() + chrono::Duration::hours(1));
}

#[test]
fn a_token_is_replaced_a_minute_before_it_expires() {
    let t = AccessToken::new(Secret::new("tok"), now() + chrono::Duration::seconds(120));
    assert!(!t.needs_replacing(now()));
    assert!(!t.needs_replacing(now() + chrono::Duration::seconds(59)));
    assert!(t.needs_replacing(now() + chrono::Duration::seconds(60)));
    assert!(t.needs_replacing(now() + chrono::Duration::seconds(121)));
}

#[test]
fn a_failed_token_request_is_an_error_that_does_not_carry_the_secret() {
    use linear_core::{Error, ErrorCode};
    let secret = Secret::new(SECRET);

    // The server echoes the secret (raw and form-encoded) in its message.
    let body = format!(
        r#"{{"error":"invalid_client","error_description":"bad {SECRET} / {}"}}"#,
        "lin_oauth_SECRETVALUE"
    );
    for status in [400, 401, 403] {
        let e = parse_token_response(status, &body, now(), &secret).unwrap_err();
        assert!(matches!(e, Error::Auth(_)), "{e:?}");
        assert_eq!(e.code(), ErrorCode::Auth);
        let text = e.to_string();
        assert!(text.contains("invalid_client"), "{text}");
        assert!(!text.contains(SECRET), "{text}");
    }

    // A 2xx without a token is refused too.
    for body in [r#"{}"#, r#"{"access_token":""}"#, "not json", ""] {
        let e = parse_token_response(200, body, now(), &secret).unwrap_err();
        assert!(matches!(e, Error::Auth(_)), "{body:?} -> {e:?}");
    }

    // The raw body of a server error is not repeated, only a parsed message.
    let e = parse_token_response(503, "<html>secret page</html>", now(), &secret).unwrap_err();
    assert!(matches!(e, Error::Http { status: 503, .. }), "{e:?}");
    assert!(!e.to_string().contains("secret page"));
    let e = parse_token_response(429, "{}", now(), &secret).unwrap_err();
    assert!(matches!(e, Error::RateLimited { .. }));
}
