//! `auth = "client_credentials"`: the app token is fetched from the id and
//! secret in the environment, kept for the run, replaced when it is about to
//! expire or Linear refuses it, and never printed. A mock serves both the
//! token endpoint and the GraphQL endpoint.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::stdout_json;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use write_support::*;

const SECRET: &str = "lin_oauth_SECRET_do_not_leak_0123456789";
const CLIENT_ID: &str = "client-id-123";

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    authorization: Option<String>,
    body: String,
}

/// What a request gets back: status and body.
type Answer = (u16, String);
/// Decides the answer to a request on the token endpoint (given its number, from 1).
type TokenHandler = Box<dyn Fn(&Seen, usize) -> Answer + Send>;
/// Decides the answer to a GraphQL request (given its operation name and the
/// `Authorization` header it carried).
type GraphqlHandler = Box<dyn Fn(&str, Option<&str>) -> Answer + Send>;

struct AppServer {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl AppServer {
    fn start(token: TokenHandler, graphql: GraphqlHandler) -> AppServer {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", server.server_addr().to_ip().expect("ip"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            let mut tokens = 0;
            for mut req in server.incoming_requests() {
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                let s = Seen {
                    path: req.url().to_owned(),
                    authorization: req
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("authorization"))
                        .map(|h| h.value.to_string()),
                    body,
                };
                log.lock().unwrap().push(s.clone());
                let (status, body) = if s.path == "/oauth/token" {
                    tokens += 1;
                    token(&s, tokens)
                } else {
                    let op = serde_json::from_str::<Value>(&s.body)
                        .ok()
                        .and_then(|v| v["operationName"].as_str().map(str::to_owned))
                        .unwrap_or_default();
                    graphql(&op, s.authorization.as_deref())
                };
                let response = tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes("content-type", "application/json").unwrap(),
                    );
                let _ = req.respond(response);
            }
        });
        AppServer { base, seen }
    }

    fn on_path(&self, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }

    fn token_requests(&self) -> Vec<Seen> {
        self.on_path("/oauth/token")
    }

    fn graphql_requests(&self) -> Vec<Seen> {
        self.on_path("/graphql")
    }
}

fn token_ok(n: usize, expires_in: u64) -> Answer {
    (
        200,
        json!({
            "access_token": format!("tok-{n}"),
            "token_type": "Bearer",
            "expires_in": expires_in,
            "scope": "read write initiative:write",
        })
        .to_string(),
    )
}

/// Answers each operation of `routes` with its reply, whatever the token.
fn by_operation(routes: Vec<(&'static str, Reply)>) -> GraphqlHandler {
    let routes: HashMap<&'static str, (u16, String)> = routes
        .into_iter()
        .map(|(op, r)| (op, (r.status, r.body)))
        .collect();
    Box::new(move |op, _| {
        routes.get(op).cloned().unwrap_or_else(|| {
            (
                200,
                json!({"errors": [{"message": format!("mock: no route for {op}")}]}).to_string(),
            )
        })
    })
}

fn comment_routes(view: View) -> Vec<(&'static str, Reply)> {
    vec![
        ("Whoami", whoami()),
        ("IssueWriteView", view.reply()),
        ("CommentCreate", comment_ok()),
    ]
}

fn assigned_to_me() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

/// A workspace `example` that authenticates with client credentials.
fn app_workspace(extra: &str) -> Sandbox {
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "example",
            "--url-key",
            "example",
            "--auth",
            "client-credentials",
            "--client-id",
            CLIENT_ID,
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    if !extra.is_empty() {
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(extra);
        text.push('\n');
        std::fs::write(&path, text).unwrap();
    }
    sb
}

fn run(sb: &Sandbox, server: &AppServer, args: &[&str]) -> std::process::Output {
    run_env(sb, server, args, &[("LINEAR_CLIENT_SECRET", SECRET)])
}

fn run_env(
    sb: &Sandbox,
    server: &AppServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    let graphql = format!("{}/graphql", server.base);
    let token = format!("{}/oauth/token", server.base);
    let mut all = vec![
        ("LINEAR_API_URL", graphql.as_str()),
        ("LINEAR_OAUTH_TOKEN_URL", token.as_str()),
        ("LINEAR_RETRY_BASE_MS", "1"),
    ];
    all.extend_from_slice(env);
    sb.run(args, None, &all)
}

/// Nothing a command printed contains the secret or a token.
fn assert_no_secret(o: &std::process::Output) {
    for text in [stdout(o), stderr(o)] {
        assert!(!text.contains(SECRET), "the secret leaked: {text}");
        assert!(!text.contains("tok-"), "a token leaked: {text}");
    }
}

fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_owned(), decode(v)))
        .collect()
}

fn decode(v: &str) -> String {
    let bytes = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(&v[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

// ------------------------------------------------------------------ the grant

#[test]
fn the_token_comes_from_the_grant_and_is_sent_as_a_bearer_token() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run(&sb, &server, &["workspace", "whoami", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    let v = stdout_json(&o);
    assert_eq!(v["auth"], "client_credentials");
    assert_eq!(v["credentials"], "env");

    let tokens = server.token_requests();
    assert_eq!(tokens.len(), 1);
    let f = form(&tokens[0].body);
    assert_eq!(f["grant_type"], "client_credentials");
    assert_eq!(f["client_id"], CLIENT_ID);
    assert_eq!(f["client_secret"], SECRET);
    assert_eq!(f["scope"], "read,write,initiative:write");
    // The grant is not sent with a header of its own.
    assert_eq!(tokens[0].authorization, None);

    let graphql = server.graphql_requests();
    assert_eq!(graphql.len(), 1);
    assert_eq!(graphql[0].authorization.as_deref(), Some("Bearer tok-1"));
}

#[test]
fn the_token_is_fetched_once_for_a_run_that_makes_several_requests() {
    let sb = app_workspace("ownership = \"lenient\"");
    let comment = write_file(&sb, "c.md", "hello");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(comment_routes(assigned_to_me())),
    );
    let o = run(
        &sb,
        &server,
        &[
            "issue",
            "comment",
            "EX-23",
            "--body-file",
            &comment,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    assert_eq!(server.graphql_requests().len(), 3);
    assert_eq!(server.token_requests().len(), 1);
    assert!(server
        .graphql_requests()
        .iter()
        .all(|r| r.authorization.as_deref() == Some("Bearer tok-1")));
}

#[test]
fn a_token_that_is_about_to_expire_is_replaced_before_the_next_request() {
    let sb = app_workspace("ownership = \"lenient\"");
    let comment = write_file(&sb, "c.md", "hello");
    // 30 s left is inside the one-minute margin: every request gets a fresh token.
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 30)),
        by_operation(comment_routes(assigned_to_me())),
    );
    let o = run(
        &sb,
        &server,
        &[
            "issue",
            "comment",
            "EX-23",
            "--body-file",
            &comment,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    assert_eq!(server.token_requests().len(), 3);
    let auth: Vec<_> = server
        .graphql_requests()
        .into_iter()
        .map(|r| r.authorization.unwrap())
        .collect();
    assert_eq!(auth, ["Bearer tok-1", "Bearer tok-2", "Bearer tok-3"]);
}

#[test]
fn a_token_linear_refuses_is_replaced_and_the_request_sent_again() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        // The first token has been revoked; the second works.
        Box::new(|_op, auth| {
            if auth == Some("Bearer tok-1") {
                (
                    401,
                    r#"{"errors":[{"message":"Authentication required","extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#.to_owned(),
                )
            } else {
                (200, WHOAMI_OK.to_owned())
            }
        }),
    );
    let o = run(&sb, &server, &["workspace", "whoami", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    assert_eq!(server.token_requests().len(), 2);
    let auth: Vec<_> = server
        .graphql_requests()
        .into_iter()
        .map(|r| r.authorization.unwrap())
        .collect();
    assert_eq!(auth, ["Bearer tok-1", "Bearer tok-2"]);
}

#[test]
fn a_token_that_keeps_being_refused_is_an_auth_error_after_one_replacement() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        Box::new(|_, _| {
            (
                401,
                r#"{"errors":[{"message":"Authentication required","extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#.to_owned(),
            )
        }),
    );
    let o = run(&sb, &server, &["workspace", "whoami"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_no_secret(&o);
    assert_eq!(server.token_requests().len(), 2);
    assert_eq!(server.graphql_requests().len(), 2);
}

// ------------------------------------------------------------------ failures

#[test]
fn a_refused_grant_is_an_auth_error_that_never_shows_the_secret() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, _| {
            (
                400,
                // A server that echoes what it was sent.
                json!({
                    "error": "invalid_client",
                    "error_description": format!("Client authentication failed for {SECRET}"),
                })
                .to_string(),
            )
        }),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run(&sb, &server, &["workspace", "whoami"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert_no_secret(&o);
    let err = stderr(&o);
    assert!(err.contains("invalid_client"), "{err}");
    // Nothing was asked of the API without a token.
    assert!(server.graphql_requests().is_empty());
    assert_eq!(server.token_requests().len(), 1);
}

#[test]
fn a_token_endpoint_that_fails_with_a_5xx_is_tried_again() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| {
            if n == 1 {
                (503, "unavailable".to_owned())
            } else {
                token_ok(n, 2_592_000)
            }
        }),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run(&sb, &server, &["workspace", "whoami"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(server.token_requests().len(), 2);
    assert_no_secret(&o);
}

#[test]
fn a_response_without_a_token_is_an_error() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, _| (200, r#"{"token_type":"Bearer"}"#.to_owned())),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run(&sb, &server, &["workspace", "whoami"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(server.graphql_requests().is_empty());
}

#[test]
fn a_missing_secret_or_client_id_says_what_to_set() {
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(vec![("Whoami", whoami())]),
    );

    // No secret.
    let sb = app_workspace("");
    let o = run_env(&sb, &server, &["workspace", "whoami"], &[]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("LINEAR_CLIENT_SECRET"),
        "{}",
        stderr(&o)
    );

    // A secret but no client id anywhere.
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "example",
            "--url-key",
            "example",
            "--auth",
            "client-credentials",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = run(&sb, &server, &["workspace", "whoami"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("client_id") && err.contains("LINEAR_CLIENT_ID"),
        "{err}"
    );
    assert!(!err.contains(SECRET));

    // ... which the environment can supply, and which beats the file.
    let o = run_env(
        &sb,
        &server,
        &["workspace", "whoami"],
        &[
            ("LINEAR_CLIENT_SECRET", SECRET),
            ("LINEAR_CLIENT_ID", "id-from-env"),
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let f = form(&server.token_requests().last().unwrap().body);
    assert_eq!(f["client_id"], "id-from-env");
    assert_no_secret(&o);
    assert_eq!(
        server.token_requests().len(),
        1,
        "only the last run asked for a token"
    );
}

#[test]
fn the_secret_can_be_set_for_one_workspace() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run_env(
        &sb,
        &server,
        &["workspace", "whoami"],
        &[
            ("LINEAR_CLIENT_SECRET", "the-global-secret"),
            ("LINEAR_CLIENT_SECRET_EXAMPLE", SECRET),
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        form(&server.token_requests()[0].body)["client_secret"],
        SECRET
    );
}

// ------------------------------------------------------------------ the rest of the CLI

#[test]
fn list_shows_the_auth_method_and_whether_the_secret_is_set_but_not_the_secret() {
    let sb = app_workspace("");
    let o = sb.run(
        &["workspace", "list", "--json"],
        None,
        &[("LINEAR_CLIENT_SECRET", SECRET)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    let v = stdout_json(&o);
    assert_eq!(v[0]["auth"], "client_credentials");
    assert_eq!(v[0]["credentials"], "env");

    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    assert_eq!(stdout_json(&o)[0]["credentials"], Value::Null);

    let o = sb.run(
        &["workspace", "list"],
        None,
        &[("LINEAR_CLIENT_SECRET", SECRET)],
    );
    assert!(stdout(&o).contains("client_credentials"), "{}", stdout(&o));
    assert_no_secret(&o);
}

#[test]
fn login_checks_the_grant_and_stores_nothing() {
    let sb = app_workspace("");
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(vec![("Whoami", whoami())]),
    );
    let o = run(&sb, &server, &["workspace", "login"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_secret(&o);
    assert!(!sb.config_dir().join("credentials").exists());
    assert_eq!(server.token_requests().len(), 1);
}

#[test]
fn the_ownership_rules_apply_to_the_app_user_like_to_any_viewer() {
    // The viewer the API reports is the one that counts: an issue of somebody
    // else's, in somebody else's project, is refused unless the workspace is lenient.
    let sb = app_workspace("");
    let comment = write_file(&sb, "c.md", "hello");
    let foreign = || {
        view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT))
    };
    let server = AppServer::start(
        Box::new(|_, n| token_ok(n, 2_592_000)),
        by_operation(comment_routes(foreign())),
    );
    let o = run(
        &sb,
        &server,
        &["issue", "comment", "EX-23", "--body-file", &comment],
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_no_secret(&o);

    let sb = app_workspace("ownership = \"lenient\"");
    let o = run(
        &sb,
        &server,
        &[
            "issue",
            "comment",
            "EX-23",
            "--body-file",
            &comment,
            "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn a_client_id_needs_client_credentials() {
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "example",
            "--url-key",
            "example",
            "--client-id",
            "x",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("client_id"), "{}", stderr(&o));
}
