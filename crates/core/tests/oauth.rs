//! OAuth authorization code flow with PKCE: the pure half.

use chrono::{Duration, TimeZone, Utc};
use linear_core::auth::{Credential, Secret};
use linear_core::oauth::*;
use linear_core::{Error, ErrorCode};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 0, 0, 0).unwrap()
}

#[test]
fn the_challenge_is_the_s256_of_the_verifier() {
    // The example of RFC 7636, appendix B.
    assert_eq!(
        code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn base64url_is_unpadded_and_url_safe() {
    assert_eq!(base64url(b""), "");
    assert_eq!(base64url(b"f"), "Zg");
    assert_eq!(base64url(b"fo"), "Zm8");
    assert_eq!(base64url(b"foo"), "Zm9v");
    assert_eq!(base64url(b"foob"), "Zm9vYg");
    assert_eq!(base64url(&[0xfb, 0xff, 0xfe]), "-__-");
}

#[test]
fn the_redirect_uri_names_localhost_and_the_port() {
    assert_eq!(redirect_uri(4601), "http://localhost:4601/callback");
    assert_eq!(DEFAULT_PORT, 4601);
}

#[test]
fn the_authorization_url_carries_the_challenge_and_the_state() {
    let url = authorization_url(
        "client-1",
        &redirect_uri(4601),
        SCOPE,
        "st ate",
        "chal-lenge_x",
    );
    assert_eq!(
        url,
        "https://linear.app/oauth/authorize?client_id=client-1\
         &redirect_uri=http%3A%2F%2Flocalhost%3A4601%2Fcallback&response_type=code\
         &scope=read%2Cwrite&state=st%20ate&code_challenge=chal-lenge_x\
         &code_challenge_method=S256"
    );
    assert!(!url.contains("secret"));
}

#[test]
fn the_token_requests_are_forms_that_never_debug_print() {
    let code = Secret::new("co/de");
    let verifier = Secret::new("veri-fier");
    let body = authorization_code_form("client-1", &code, &redirect_uri(4601), &verifier);
    assert_eq!(
        body.expose(),
        "grant_type=authorization_code&code=co%2Fde\
         &redirect_uri=http%3A%2F%2Flocalhost%3A4601%2Fcallback&client_id=client-1\
         &code_verifier=veri-fier"
    );
    assert!(!format!("{body:?}").contains("co/de"));

    let body = refresh_form("client-1", &Secret::new("re+fresh"));
    assert_eq!(
        body.expose(),
        "grant_type=refresh_token&refresh_token=re%2Bfresh&client_id=client-1"
    );
}

#[test]
fn a_token_answer_becomes_a_credential_with_an_expiry() {
    let t = parse_token_response(
        200,
        r#"{"access_token":"ACCVAL","token_type":"Bearer","expires_in":86399,"scope":"read write","refresh_token":"ref"}"#,
        now(),
        &[],
    )
    .unwrap();
    assert_eq!(t.expires_at, Some(now() + Duration::seconds(86399)));
    assert!(!format!("{t:?}").contains("ACCVAL"));
    let c = t.into_credential(None);
    assert_eq!(
        c,
        Credential::Oauth {
            access_token: Secret::new("ACCVAL"),
            refresh_token: Some(Secret::new("ref")),
            expires_at: Some(now() + Duration::seconds(86399)),
        }
    );
    assert_eq!(c.authorization(), "Bearer ACCVAL");
}

#[test]
fn a_refresh_that_names_no_new_refresh_token_keeps_the_old_one() {
    let t = parse_token_response(
        200,
        r#"{"access_token":"acc2","expires_in":60}"#,
        now(),
        &[],
    )
    .unwrap();
    assert!(t.refresh_token.is_none());
    let old = Secret::new("old-ref");
    let Credential::Oauth { refresh_token, .. } = t.into_credential(Some(&old)) else {
        panic!("not an OAuth credential");
    };
    assert_eq!(refresh_token, Some(old));

    // No lifetime in the answer: the token is not known to expire.
    let t = parse_token_response(200, r#"{"access_token":"a"}"#, now(), &[]).unwrap();
    assert_eq!(t.expires_at, None);
}

#[test]
fn a_token_is_refreshed_a_minute_before_it_expires() {
    let c = |secs| Credential::Oauth {
        access_token: Secret::new("a"),
        refresh_token: None,
        expires_at: Some(now() + Duration::seconds(secs)),
    };
    assert!(!c(120).needs_refresh(now()));
    assert!(c(60).needs_refresh(now()));
    assert!(c(-5).needs_refresh(now()));
    let forever = Credential::Oauth {
        access_token: Secret::new("a"),
        refresh_token: None,
        expires_at: None,
    };
    assert!(!forever.needs_refresh(now()));
    assert!(!Credential::ApiKey {
        api_key: Secret::new("k")
    }
    .needs_refresh(now()));
}

#[test]
fn a_refused_token_request_is_an_auth_error_without_the_secrets() {
    let verifier = Secret::new("VERIFIER-SECRET");
    let code = Secret::new("CODE-SECRET");
    let body =
        r#"{"error":"invalid_grant","error_description":"bad VERIFIER-SECRET and CODE-SECRET"}"#;
    for status in [400, 401, 403] {
        let e = parse_token_response(status, body, now(), &[&verifier, &code]).unwrap_err();
        assert!(matches!(e, Error::Auth(_)), "{e:?}");
        assert_eq!(e.code(), ErrorCode::Auth);
        let text = e.to_string();
        assert!(text.contains("invalid_grant"), "{text}");
        assert!(!text.contains("SECRET"), "{text}");
    }

    for body in ["{}", r#"{"access_token":""}"#, "not json", ""] {
        let e = parse_token_response(200, body, now(), &[]).unwrap_err();
        assert!(matches!(e, Error::Auth(_)), "{body:?} -> {e:?}");
    }
    let e = parse_token_response(503, "<html>secret page</html>", now(), &[]).unwrap_err();
    assert!(matches!(e, Error::Http { status: 503, .. }), "{e:?}");
    assert!(!e.to_string().contains("secret page"));
    let e = parse_token_response(429, "", now(), &[]).unwrap_err();
    assert!(matches!(e, Error::RateLimited { .. }));
}

#[test]
fn the_callback_gives_the_code_when_the_state_matches() {
    assert_eq!(
        parse_callback("/callback?code=ab%2Fc&state=xyz", "xyz"),
        Callback::Code(Secret::new("ab/c"))
    );
    // Order does not matter, and other parameters are ignored.
    assert_eq!(
        parse_callback("/callback?state=xyz&foo=1&code=c1", "xyz"),
        Callback::Code(Secret::new("c1"))
    );
}

#[test]
fn a_callback_for_another_login_or_without_a_code_is_rejected_not_trusted() {
    for target in [
        "/callback?code=c1&state=other",
        "/callback?code=c1",
        "/callback",
        "/callback?code=c1&state=",
    ] {
        assert!(
            matches!(parse_callback(target, "xyz"), Callback::Rejected(_)),
            "{target}"
        );
    }
    assert!(matches!(
        parse_callback("/callback?state=xyz", "xyz"),
        Callback::Rejected(_)
    ));
    assert!(matches!(
        parse_callback("/callback?code=&state=xyz", "xyz"),
        Callback::Rejected(_)
    ));
}

#[test]
fn something_that_is_not_the_callback_is_ignored() {
    for target in [
        "/favicon.ico",
        "/",
        "/callbackx?code=c&state=xyz",
        "/other?state=xyz",
    ] {
        assert_eq!(parse_callback(target, "xyz"), Callback::Ignore, "{target}");
    }
}

#[test]
fn a_refusal_in_the_callback_ends_the_login() {
    assert_eq!(
        parse_callback(
            "/callback?error=access_denied&error_description=The+user+denied&state=xyz",
            "xyz"
        ),
        Callback::Denied("access_denied: The user denied".into())
    );
    // An error with a wrong state is not believed either.
    assert!(matches!(
        parse_callback("/callback?error=access_denied&state=other", "xyz"),
        Callback::Rejected(_)
    ));
}
