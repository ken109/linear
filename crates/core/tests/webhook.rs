//! Webhook signature verification: known signatures, rejections, and the
//! timestamp window. `now` is fixed.

use chrono::{DateTime, TimeZone, Utc};
use linear_core::webhook::{sign, verify_webhook, Rejection, Verification, TOLERANCE_MS};

const SECRET: &str = "lin_wh_testsecret";
const BODY: &str = r#"{"action":"update","type":"Issue","organizationId":"org-1","webhookTimestamp":1760000000000}"#;
/// `printf %s "$BODY" | openssl dgst -sha256 -hmac "$SECRET"`
const SIGNATURE: &str = "19882a77e4dae983f01a21fbe0a53dd8142d0d699c37633bc9258a6237b13830";

fn at(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).unwrap()
}

fn sent() -> DateTime<Utc> {
    at(1_760_000_000_000)
}

fn rejected(body: &str, sig: &str, now: DateTime<Utc>) -> Rejection {
    match verify_webhook(body, sig, SECRET, now).unwrap() {
        Verification::Invalid { reason } => reason,
        Verification::Valid { .. } => panic!("expected a rejection"),
    }
}

#[test]
fn sign_matches_rfc_4231_test_case_1() {
    // Key is 20 bytes of 0x0b; it is valid UTF-8 as 20 vertical tabs.
    let key = "\u{0b}".repeat(20);
    assert_eq!(
        sign(&key, "Hi There"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn sign_matches_a_signature_made_by_openssl() {
    assert_eq!(sign(SECRET, BODY), SIGNATURE);
}

#[test]
fn a_valid_delivery_is_accepted_and_described() {
    let got = verify_webhook(BODY, SIGNATURE, SECRET, sent()).unwrap();
    let Verification::Valid { event } = got else {
        panic!("expected a valid delivery");
    };
    assert_eq!(event.action, "update");
    assert_eq!(event.resource_type, "Issue");
    assert_eq!(event.organization_id.as_deref(), Some("org-1"));
    assert_eq!(event.webhook_timestamp, 1_760_000_000_000);
}

#[test]
fn the_signature_may_be_upper_case_or_padded() {
    for sig in [SIGNATURE.to_uppercase(), format!(" {SIGNATURE}\n")] {
        assert!(matches!(
            verify_webhook(BODY, &sig, SECRET, sent()).unwrap(),
            Verification::Valid { .. }
        ));
    }
}

#[test]
fn a_changed_body_is_a_signature_mismatch() {
    let tampered = BODY.replace("update", "remove");
    assert_eq!(
        rejected(&tampered, SIGNATURE, sent()),
        Rejection::SignatureMismatch
    );
    // Re-serialized JSON is not the body that was signed.
    let pretty = BODY.replace(',', ", ");
    assert_eq!(
        rejected(&pretty, SIGNATURE, sent()),
        Rejection::SignatureMismatch
    );
}

#[test]
fn another_secret_is_a_signature_mismatch() {
    assert_eq!(
        match verify_webhook(BODY, SIGNATURE, "other", sent()).unwrap() {
            Verification::Invalid { reason } => reason,
            Verification::Valid { .. } => panic!("accepted a wrong secret"),
        },
        Rejection::SignatureMismatch
    );
}

#[test]
fn a_signature_that_is_not_64_hex_digits_is_malformed() {
    for sig in [
        "",
        "zz",
        &SIGNATURE[..62],
        &format!("{SIGNATURE}00"),
        "sha256=abc",
    ] {
        assert_eq!(
            rejected(BODY, sig, sent()),
            Rejection::MalformedSignature,
            "{sig:?}"
        );
    }
    // 64 characters, but not hex.
    assert_eq!(
        rejected(BODY, &"g".repeat(64), sent()),
        Rejection::MalformedSignature
    );
}

#[test]
fn the_timestamp_window_is_a_minute_either_way() {
    let ok = |now: DateTime<Utc>| {
        matches!(
            verify_webhook(BODY, SIGNATURE, SECRET, now).unwrap(),
            Verification::Valid { .. }
        )
    };
    assert!(ok(at(1_760_000_000_000 + TOLERANCE_MS)));
    assert!(ok(at(1_760_000_000_000 - TOLERANCE_MS)));
    assert_eq!(
        rejected(BODY, SIGNATURE, at(1_760_000_000_000 + TOLERANCE_MS + 1)),
        Rejection::StaleTimestamp
    );
    // A delivery "from the future" is as suspect as an old one.
    assert_eq!(
        rejected(BODY, SIGNATURE, at(1_760_000_000_000 - TOLERANCE_MS - 1)),
        Rejection::StaleTimestamp
    );
}

#[test]
fn a_correctly_signed_body_that_is_not_a_payload_is_rejected() {
    let not_json = "not json";
    assert_eq!(
        rejected(not_json, &sign(SECRET, not_json), sent()),
        Rejection::MalformedBody
    );
    let no_timestamp = r#"{"action":"create","type":"Issue"}"#;
    assert_eq!(
        rejected(no_timestamp, &sign(SECRET, no_timestamp), sent()),
        Rejection::MissingTimestamp
    );
}

#[test]
fn a_bad_signature_is_reported_before_the_body_is_read() {
    // Nothing about an unauthenticated body is revealed.
    assert_eq!(
        rejected("not json", SIGNATURE, sent()),
        Rejection::SignatureMismatch
    );
}

#[test]
fn an_empty_secret_is_a_usage_error() {
    let err = verify_webhook(BODY, SIGNATURE, "", sent()).unwrap_err();
    assert_eq!(err.code().as_str(), "usage");
}

#[test]
fn the_verification_serializes_as_a_tagged_object() {
    let valid = verify_webhook(BODY, SIGNATURE, SECRET, sent()).unwrap();
    assert_eq!(
        serde_json::to_value(valid).unwrap(),
        serde_json::json!({
            "status": "valid",
            "event": {
                "action": "update",
                "type": "Issue",
                "organizationId": "org-1",
                "webhookTimestamp": 1_760_000_000_000_i64,
            }
        })
    );
    let invalid = verify_webhook(BODY, "zz", SECRET, sent()).unwrap();
    assert_eq!(
        serde_json::to_value(invalid).unwrap(),
        serde_json::json!({ "status": "invalid", "reason": "malformed-signature" })
    );
}
