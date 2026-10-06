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
}
