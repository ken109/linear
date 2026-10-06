//! Retrying reads, never writes, and the request time limit (`--timeout`),
//! against a server that plays a script: the n-th request gets the n-th step
//! (the last step repeats).

mod common;

use common::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: &str = "lin_api_RETRYTEST_do_not_leak_0123456789";
const RATE_LIMITED: &str = r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED","type":"ratelimited"}}]}"#;

enum Step {
    Reply {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: String,
    },
    /// Answer only after this long (the client is expected to have given up).
    Hang(Duration),
}

fn reply(status: u16, body: &str) -> Step {
    Step::Reply {
        status,
        headers: vec![],
        body: body.to_owned(),
    }
}

fn whoami() -> Step {
    reply(200, WHOAMI_OK)
}

struct Script {
    url: String,
    seen: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl Script {
    fn start(steps: Vec<Step>) -> Script {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let url = format!(
            "http://{}/graphql",
            server.server_addr().to_ip().expect("ip")
        );
        let seen = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let steps = Arc::new(steps);
        let (count, log) = (Arc::clone(&seen), Arc::clone(&bodies));
        std::thread::spawn(move || {
            // One thread per request, so a request that hangs does not hold up the next.
            for mut req in server.incoming_requests() {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                log.lock().unwrap().push(body);
                let steps = Arc::clone(&steps);
                std::thread::spawn(move || {
                    let step = &steps[n.min(steps.len() - 1)];
                    let (status, headers, body) = match step {
                        Step::Reply {
                            status,
                            headers,
                            body,
                        } => (*status, headers.clone(), body.clone()),
                        Step::Hang(d) => {
                            std::thread::sleep(*d);
                            (200, vec![], WHOAMI_OK.to_owned())
                        }
                    };
                    let mut response = tiny_http::Response::from_string(body)
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("content-type", "application/json")
                                .unwrap(),
                        );
                    for (k, v) in headers {
                        response = response.with_header(
                            tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap(),
                        );
                    }
                    let _ = req.respond(response);
                });
            }
        });
        Script { url, seen, bodies }
    }

    fn requests(&self) -> usize {
        self.seen.load(Ordering::SeqCst)
    }
}

fn workspace() -> Sandbox {
    let sb = Sandbox::new();
    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "example"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    sb
}

/// Run `linear <args>` against the script, retrying quickly (1 ms before the first retry).
fn run(
    sb: &Sandbox,
    script: &Script,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::process::Output {
    let mut env = vec![
        ("LINEAR_API_URL", script.url.as_str()),
        ("LINEAR_API_KEY_EXAMPLE", KEY),
        ("LINEAR_RETRY_BASE_MS", "1"),
    ];
    env.extend_from_slice(extra_env);
    sb.run(args, None, &env)
}

const WHO: [&str; 2] = ["workspace", "whoami"];

// ------------------------------------------------------------------ retrying reads

#[test]
fn a_read_that_gets_a_5xx_is_tried_again() {
    let sb = workspace();
    let script = Script::start(vec![reply(502, "bad gateway"), reply(503, ""), whoami()]);
    let o = run(&sb, &script, &WHO, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(script.requests(), 3);
    assert!(!stderr(&o).contains(KEY) && !stdout(&o).contains(KEY));
}

#[test]
fn the_retries_are_bounded_and_the_last_error_is_reported() {
    let sb = workspace();
    let script = Script::start(vec![reply(503, "unavailable")]);
    let o = run(&sb, &script, &WHO, &[]);
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 3, "the request and two retries");
    assert!(stderr(&o).contains("HTTP 503"), "{}", stderr(&o));

    // LINEAR_RETRIES changes the bound; 0 turns retrying off.
    let script = Script::start(vec![reply(503, "unavailable")]);
    let o = run(&sb, &script, &WHO, &[("LINEAR_RETRIES", "0")]);
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 1);

    let script = Script::start(vec![reply(503, "unavailable")]);
    let o = run(&sb, &script, &WHO, &[("LINEAR_RETRIES", "4")]);
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 5);
}

#[test]
fn errors_a_second_try_cannot_fix_are_not_retried() {
    let sb = workspace();
    for step in [
        reply(400, "bad request"),
        reply(404, "not found"),
        reply(
            401,
            r#"{"errors":[{"message":"nope","extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#,
        ),
        reply(200, r#"{"errors":[{"message":"no such thing"}]}"#),
    ] {
        let script = Script::start(vec![step, whoami()]);
        let o = run(&sb, &script, &WHO, &[]);
        assert_ne!(code(&o), 0);
        assert_eq!(script.requests(), 1, "{}", stderr(&o));
    }
}

// ------------------------------------------------------------------ rate limits

#[test]
fn the_hourly_rate_limit_stops_the_command_without_retrying() {
    let sb = workspace();
    let script = Script::start(vec![
        Step::Reply {
            status: 429,
            headers: vec![("Retry-After", "2400")],
            body: RATE_LIMITED.to_owned(),
        },
        whoami(),
    ]);
    let started = Instant::now();
    let o = run(&sb, &script, &WHO, &[]);
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 1);
    assert!(stderr(&o).contains("rate limited"), "{}", stderr(&o));
    assert!(started.elapsed() < Duration::from_secs(5));

    // No hint about how long it lasts: stop as well.
    let script = Script::start(vec![reply(429, RATE_LIMITED), whoami()]);
    let o = run(&sb, &script, &WHO, &[]);
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 1);
}

#[test]
fn a_rate_limit_that_ends_at_once_is_waited_out() {
    let sb = workspace();
    let script = Script::start(vec![
        Step::Reply {
            status: 429,
            headers: vec![("Retry-After", "0")],
            body: RATE_LIMITED.to_owned(),
        },
        whoami(),
    ]);
    let o = run(&sb, &script, &WHO, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(script.requests(), 2);
}

// ------------------------------------------------------------------ writes

#[test]
fn a_write_is_never_sent_twice() {
    let sb = workspace();
    let path = sb.config_dir().join("workspaces.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}allow_raw_mutation = true\n")).unwrap();

    // The verification of the workspace is a read; the mutation then fails once.
    let script = Script::start(vec![whoami(), reply(502, "bad gateway"), whoami()]);
    let o = run(
        &sb,
        &script,
        &[
            "api",
            "--mutation",
            "mutation { issueDelete(id: \"x\") { success } }",
        ],
        &[],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert_eq!(
        script.requests(),
        2,
        "the whoami and one mutation, no retry"
    );
    let bodies = script.bodies.lock().unwrap();
    assert!(bodies[1].contains("issueDelete"), "{}", bodies[1]);
}

// ------------------------------------------------------------------ the time limit

#[test]
fn a_slow_read_times_out_and_is_tried_again() {
    let sb = workspace();
    let script = Script::start(vec![Step::Hang(Duration::from_secs(4)), whoami()]);
    let started = Instant::now();
    let o = run(
        &sb,
        &script,
        &["--timeout", "1", "workspace", "whoami"],
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(script.requests(), 2);
    let took = started.elapsed();
    assert!(took >= Duration::from_secs(1), "{took:?}");
    assert!(took < Duration::from_secs(4), "{took:?}");
}

#[test]
fn a_timeout_that_keeps_happening_says_how_to_raise_the_limit() {
    let sb = workspace();
    let script = Script::start(vec![Step::Hang(Duration::from_secs(4))]);
    let o = run(
        &sb,
        &script,
        &["workspace", "whoami"],
        &[("LINEAR_TIMEOUT", "1"), ("LINEAR_RETRIES", "0")],
    );
    assert_eq!(code(&o), 1);
    assert_eq!(script.requests(), 1);
    let err = stderr(&o);
    assert!(err.contains("request to Linear failed"), "{err}");
    assert!(
        err.contains("--timeout") && err.contains("LINEAR_TIMEOUT"),
        "{err}"
    );
    assert!(!err.contains(KEY));
}

#[test]
fn the_flag_beats_the_environment_and_bad_values_are_usage_errors() {
    let sb = workspace();
    // The environment asks for 1 s, the flag for 30 s: the slow answer (2 s) is waited for.
    let script = Script::start(vec![Step::Hang(Duration::from_secs(2))]);
    let o = run(
        &sb,
        &script,
        &["--timeout", "30", "workspace", "whoami"],
        &[("LINEAR_TIMEOUT", "1"), ("LINEAR_RETRIES", "0")],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    let script = Script::start(vec![whoami()]);
    for (args, env) in [
        (vec!["--timeout", "0", "workspace", "whoami"], vec![]),
        (
            vec!["workspace", "whoami"],
            vec![("LINEAR_TIMEOUT", "soon")],
        ),
        (vec!["workspace", "whoami"], vec![("LINEAR_TIMEOUT", "0")]),
        (vec!["workspace", "whoami"], vec![("LINEAR_RETRIES", "99")]),
    ] {
        let o = run(&sb, &script, &args, &env);
        assert_eq!(code(&o), 2, "{args:?} {env:?}: {}", stderr(&o));
    }
    assert_eq!(script.requests(), 0);
}
