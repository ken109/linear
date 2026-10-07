//! `workspace login --oauth`: the PKCE authorization code flow with a local callback, and
//! the refresh of the token it returns. No real browser: the test reads the URL the command
//! prints and plays the browser's part by calling the callback itself. One mock serves
//! both the token endpoint and the GraphQL endpoint, answering in the order queued.

mod common;
use common::*;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

const CLIENT_ID: &str = "client-id-123";

fn token_answer(access: &str, refresh: Option<&str>, expires_in: u64) -> Reply {
    let mut v = serde_json::json!({
        "access_token": access, "token_type": "Bearer",
        "expires_in": expires_in, "scope": "read write",
    });
    if let Some(r) = refresh {
        v["refresh_token"] = r.into();
    }
    ok(&v.to_string())
}

fn add_oauth_workspace(sb: &Sandbox) {
    let o = sb.run(
        &[
            "workspace",
            "add",
            "example",
            "--url-key",
            "example",
            "--auth",
            "oauth",
            "--client-id",
            CLIENT_ID,
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

fn credentials_path(sb: &Sandbox) -> std::path::PathBuf {
    sb.config_dir().join("credentials").join("example.json")
}

fn stored(sb: &Sandbox) -> Value {
    serde_json::from_str(&std::fs::read_to_string(credentials_path(sb)).unwrap()).unwrap()
}

fn write_credential(sb: &Sandbox, json: &str) {
    let path = credentials_path(sb);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn env_for<'a>(mock: &'a Mock, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut env = vec![("LINEAR_OAUTH_TOKEN_URL", mock.url.clone())];
    env.extend(extra.iter().map(|(k, v)| (*k, (*v).to_owned())));
    env
}

/// A running `linear workspace login --oauth`.
struct Login {
    child: Child,
    stderr: BufReader<std::process::ChildStderr>,
    /// The authorization URL it printed.
    url: String,
}

impl Login {
    fn start(sb: &Sandbox, mock: &Mock, args: &[&str], extra: &[(&str, &str)]) -> Login {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_linear"));
        cmd.args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", sb.root.path())
            .env("LINEAR_CONFIG_DIR", sb.config_dir())
            .env("LINEAR_API_URL", &mock.url)
            .env("LINEAR_OAUTH_TOKEN_URL", &mock.url)
            .current_dir(sb.cwd())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn linear");
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let url;
        loop {
            let mut line = String::new();
            assert!(
                stderr.read_line(&mut line).unwrap() > 0,
                "the command ended without printing the URL"
            );
            if line
                .trim()
                .starts_with("https://linear.app/oauth/authorize?")
            {
                url = line.trim().to_owned();
                break;
            }
        }
        Login { child, stderr, url }
    }

    fn param(&self, name: &str) -> String {
        let query = self.url.split_once('?').unwrap().1;
        let value = query
            .split('&')
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {}", self.url));
        percent_decode(value)
    }

    /// What the browser does when Linear redirects it: a GET to the callback.
    fn callback(&self, port: u16, query: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET /callback?{query} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        (status, response)
    }

    /// End a login that is still waiting.
    fn abandon(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn finish(mut self) -> (i32, String, String) {
        let mut out = String::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        let mut err = String::new();
        self.stderr.read_to_string(&mut err).unwrap();
        let status = self.child.wait().unwrap();
        (status.code().unwrap(), out, err)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn form_field(body: &str, name: &str) -> Option<String> {
    body.split('&')
        .find_map(|p| p.strip_prefix(&format!("{name}=")))
        .map(percent_decode)
}

#[test]
fn login_exchanges_the_code_with_the_verifier_and_stores_the_tokens() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let mock = Mock::start(vec![
        token_answer("ACCESS-ONE", Some("REFRESH-ONE"), 86399),
        ok(WHOAMI_OK),
    ]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port.to_string(),
        ],
        &[],
    );

    // The URL asks for the code flow with PKCE, for this app, and calls back on this port.
    assert_eq!(login.param("client_id"), CLIENT_ID);
    assert_eq!(login.param("response_type"), "code");
    assert_eq!(login.param("code_challenge_method"), "S256");
    assert_eq!(login.param("scope"), "read,write");
    assert_eq!(
        login.param("redirect_uri"),
        format!("http://localhost:{port}/callback")
    );
    let state = login.param("state");
    let challenge = login.param("code_challenge");

    let (status, page) = login.callback(port, &format!("code=THE-CODE&state={state}"));
    assert_eq!(status, 200);
    assert!(page.contains("Logged in to Linear"), "{page}");
    let (exit, out, err) = login.finish();
    assert_eq!(exit, 0, "{err}");
    assert!(
        out.contains("Logged in to example as Alice Example"),
        "{out}"
    );
    for secret in ["ACCESS-ONE", "REFRESH-ONE", "THE-CODE"] {
        assert!(
            !out.contains(secret) && !err.contains(secret),
            "{secret} leaked"
        );
    }

    // The token request: the code, the redirect, and a verifier that matches the challenge.
    let requests = mock.requests();
    let token = &requests[0].body;
    assert_eq!(
        form_field(token, "grant_type").as_deref(),
        Some("authorization_code")
    );
    assert_eq!(form_field(token, "code").as_deref(), Some("THE-CODE"));
    assert_eq!(form_field(token, "client_id").as_deref(), Some(CLIENT_ID));
    assert_eq!(
        form_field(token, "redirect_uri").unwrap(),
        format!("http://localhost:{port}/callback")
    );
    let verifier = form_field(token, "code_verifier").unwrap();
    assert!(verifier.len() >= 43, "{verifier}");
    assert_eq!(linear_core::oauth::code_challenge(&verifier), challenge);
    assert!(!token.contains("client_secret"));
    // The new token is what reaches the API, as a bearer token.
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer ACCESS-ONE")
    );

    // The file keeps the established shape, with an expiry about a day away.
    let v = stored(&sb);
    assert_eq!(v["kind"], "oauth");
    assert_eq!(v["access_token"], "ACCESS-ONE");
    assert_eq!(v["refresh_token"], "REFRESH-ONE");
    let expires = chrono::DateTime::parse_from_rfc3339(v["expires_at"].as_str().unwrap()).unwrap();
    let left = expires.timestamp() - chrono::Utc::now().timestamp();
    assert!((86000..=86400).contains(&left), "{left}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(credentials_path(&sb))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&o)).unwrap()[0]["credentials"],
        "file"
    );
}

#[test]
fn a_callback_that_is_not_for_this_login_is_refused_and_the_wait_goes_on() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let mock = Mock::start(vec![
        token_answer("ACCESS-ONE", Some("R"), 3600),
        ok(WHOAMI_OK),
    ]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port.to_string(),
        ],
        &[],
    );
    let state = login.param("state");

    assert_eq!(login.callback(port, "code=EVIL&state=wrong").0, 400);
    assert_eq!(login.callback(port, "code=EVIL").0, 400);
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET /favicon.ico HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
    let mut r = String::new();
    s.read_to_string(&mut r).unwrap();
    assert!(r.starts_with("HTTP/1.1 404"), "{r}");
    assert!(mock.requests().is_empty(), "nothing was sent to Linear yet");

    assert_eq!(
        login.callback(port, &format!("code=GOOD&state={state}")).0,
        200
    );
    let (exit, _, err) = login.finish();
    assert_eq!(exit, 0, "{err}");
    assert_eq!(
        form_field(&mock.requests()[0].body, "code").as_deref(),
        Some("GOOD")
    );
}

#[test]
fn a_denied_login_stores_nothing() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let mock = Mock::start(vec![]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port.to_string(),
        ],
        &[],
    );
    let state = login.param("state");
    login.callback(
        port,
        &format!("error=access_denied&error_description=The+user+denied&state={state}"),
    );
    let (exit, _, err) = login.finish();
    assert_eq!(exit, 3, "{err}");
    assert!(err.contains("access_denied"), "{err}");
    assert!(!credentials_path(&sb).exists());
    assert!(mock.requests().is_empty());
}

#[test]
fn a_login_nobody_completes_times_out() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let mock = Mock::start(vec![]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port.to_string(),
        ],
        &[("LINEAR_OAUTH_TIMEOUT", "1")],
    );
    let (exit, _, err) = login.finish();
    assert_eq!(exit, 1, "{err}");
    assert!(err.contains("timed out"), "{err}");
    assert!(!credentials_path(&sb).exists());
}

#[test]
fn a_refused_code_is_an_auth_error_and_stores_nothing() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let mock = Mock::start(vec![Reply {
        status: 400,
        body: r#"{"error":"invalid_grant","error_description":"code expired"}"#.into(),
    }]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port.to_string(),
        ],
        &[],
    );
    let state = login.param("state");
    login.callback(port, &format!("code=OLD-CODE&state={state}"));
    let (exit, _, err) = login.finish();
    assert_eq!(exit, 3, "{err}");
    assert!(err.contains("invalid_grant"), "{err}");
    assert!(!err.contains("OLD-CODE"));
    assert!(!credentials_path(&sb).exists());
}

#[test]
fn the_callback_port_comes_from_the_flag_then_the_environment_then_the_config() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let (config_port, env_port, flag_port) = (free_port(), free_port(), free_port());
    let path = sb.config_dir().join("workspaces.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}oauth_port = {config_port}\n")).unwrap();
    let mock = Mock::start(vec![]);
    let redirect = |login: &Login| login.param("redirect_uri");

    let login = Login::start(
        &sb,
        &mock,
        &["workspace", "login", "example", "--no-browser"],
        &[],
    );
    assert_eq!(
        redirect(&login),
        format!("http://localhost:{config_port}/callback")
    );
    login.abandon();

    let env = env_port.to_string();
    let login = Login::start(
        &sb,
        &mock,
        &["workspace", "login", "example", "--no-browser"],
        &[("LINEAR_OAUTH_PORT", &env)],
    );
    assert_eq!(
        redirect(&login),
        format!("http://localhost:{env_port}/callback")
    );
    login.abandon();

    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &flag_port.to_string(),
        ],
        &[("LINEAR_OAUTH_PORT", &env)],
    );
    assert_eq!(
        redirect(&login),
        format!("http://localhost:{flag_port}/callback")
    );
    login.abandon();

    // Not a port.
    let o = sb.run(
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            "0",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = sb.run(
        &["workspace", "login", "example", "--no-browser"],
        None,
        &[("LINEAR_OAUTH_PORT", "http")],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
fn a_port_in_use_says_how_to_choose_another() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port().to_string();
    let o = sb.run(
        &[
            "workspace",
            "login",
            "example",
            "--no-browser",
            "--port",
            &port,
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("--port"), "{}", stderr(&o));
    assert!(stderr(&o).contains("redirect URI"), "{}", stderr(&o));
}

#[test]
fn oauth_needs_a_client_id_and_the_oauth_flags_need_oauth() {
    let sb = Sandbox::new();
    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "example"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0);

    // An API key workspace, no client id anywhere.
    let o = sb.run(
        &["workspace", "login", "example", "--oauth", "--no-browser"],
        None,
        &[],
    );
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("no client id"), "{}", stderr(&o));

    // The flags of the OAuth login without --oauth are a mistake, not ignored.
    let o = sb.run(
        &["workspace", "login", "example", "--port", "4601"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--oauth"), "{}", stderr(&o));
    let o = sb.run(
        &["workspace", "login", "example", "--oauth", "--with-token"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);

    // --client-id is for an OAuth app, not an API key.
    let o = sb.run(
        &[
            "workspace",
            "add",
            "k",
            "--url-key",
            "k",
            "--client-id",
            "x",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
    let o = sb.run(
        &[
            "workspace",
            "add",
            "p",
            "--url-key",
            "p",
            "--oauth-port",
            "4610",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
}

#[test]
fn login_with_the_oauth_flag_switches_the_workspace_and_remembers_the_client_id() {
    let sb = Sandbox::new();
    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "example"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0);
    let mock = Mock::start(vec![
        token_answer("ACCESS-ONE", Some("R"), 3600),
        ok(WHOAMI_OK),
    ]);
    let port = free_port();
    let login = Login::start(
        &sb,
        &mock,
        &[
            "workspace",
            "login",
            "example",
            "--oauth",
            "--no-browser",
            "--client-id",
            CLIENT_ID,
            "--port",
            &port.to_string(),
        ],
        &[],
    );
    let state = login.param("state");
    login.callback(port, &format!("code=C&state={state}"));
    let (exit, _, err) = login.finish();
    assert_eq!(exit, 0, "{err}");

    let config = std::fs::read_to_string(sb.config_dir().join("workspaces.toml")).unwrap();
    assert!(config.contains("auth = \"oauth\""), "{config}");
    assert!(
        config.contains(&format!("client_id = \"{CLIENT_ID}\"")),
        "{config}"
    );
    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        (v[0]["auth"].as_str(), v[0]["credentials"].as_str()),
        (Some("oauth"), Some("file"))
    );
}

// ------------------------------------------------------------------ refresh

const EXPIRED: &str = r#"{"kind":"oauth","access_token":"OLD-ACCESS","refresh_token":"OLD-REFRESH","expires_at":"2020-01-01T00:00:00Z"}"#;

fn run_whoami(sb: &Sandbox, mock: &Mock, extra: &[(&str, &str)]) -> std::process::Output {
    let env = env_for(mock, extra);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    sb.run(&["workspace", "whoami", "--json"], Some(mock), &env)
}

#[test]
fn an_expired_token_is_refreshed_before_use_and_the_new_tokens_are_stored() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    write_credential(&sb, EXPIRED);
    let mock = Mock::start(vec![
        token_answer("NEW-ACCESS", Some("NEW-REFRESH"), 86399),
        ok(WHOAMI_OK),
        ok(WHOAMI_OK),
    ]);

    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    for secret in ["OLD-ACCESS", "OLD-REFRESH", "NEW-ACCESS", "NEW-REFRESH"] {
        assert!(!stdout(&o).contains(secret) && !stderr(&o).contains(secret));
    }
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let form = &requests[0].body;
    assert_eq!(
        form_field(form, "grant_type").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(
        form_field(form, "refresh_token").as_deref(),
        Some("OLD-REFRESH")
    );
    assert_eq!(form_field(form, "client_id").as_deref(), Some(CLIENT_ID));
    assert!(!form.contains("client_secret"));
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer NEW-ACCESS")
    );
    let v = stored(&sb);
    assert_eq!(v["access_token"], "NEW-ACCESS");
    assert_eq!(v["refresh_token"], "NEW-REFRESH");

    // The next run finds a token in date and does not ask again.
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[2].authorization.as_deref(),
        Some("Bearer NEW-ACCESS")
    );
}

#[test]
fn a_refresh_that_names_no_new_refresh_token_keeps_the_old_one() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    write_credential(&sb, EXPIRED);
    let mock = Mock::start(vec![token_answer("NEW-ACCESS", None, 3600), ok(WHOAMI_OK)]);
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stored(&sb);
    assert_eq!(v["access_token"], "NEW-ACCESS");
    assert_eq!(v["refresh_token"], "OLD-REFRESH");
}

#[test]
fn a_refused_refresh_asks_for_a_new_login_and_leaves_the_file_alone() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    write_credential(&sb, EXPIRED);
    let mock = Mock::start(vec![Reply {
        status: 400,
        body: r#"{"error":"invalid_grant","error_description":"refresh token revoked"}"#.into(),
    }]);
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 3);
    let err = stderr(&o);
    assert!(err.contains("has expired"), "{err}");
    assert!(err.contains("linear workspace login example"), "{err}");
    assert!(!err.contains("OLD-REFRESH"));
    assert_eq!(stored(&sb)["access_token"], "OLD-ACCESS");
    assert_eq!(mock.requests().len(), 1, "no request reached the API");
}

#[test]
fn an_expired_token_without_a_refresh_token_asks_for_a_new_login() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    write_credential(
        &sb,
        r#"{"kind":"oauth","access_token":"OLD-ACCESS","expires_at":"2020-01-01T00:00:00Z"}"#,
    );
    let mock = Mock::start(vec![]);
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 3);
    assert!(
        stderr(&o).contains("linear workspace login example"),
        "{}",
        stderr(&o)
    );
    assert!(mock.requests().is_empty());
}

#[test]
fn a_token_without_an_expiry_is_used_as_it_is() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    write_credential(&sb, r#"{"kind":"oauth","access_token":"LONG-LIVED"}"#);
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.requests()[0].authorization.as_deref(),
        Some("Bearer LONG-LIVED")
    );
}

#[test]
fn a_refresh_that_cannot_reach_linear_fails_but_a_token_still_in_date_is_used() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    // 30 seconds left: inside the refresh margin, still valid.
    let soon = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    write_credential(
        &sb,
        &format!(
            r#"{{"kind":"oauth","access_token":"STILL-GOOD","refresh_token":"R","expires_at":"{soon}"}}"#
        ),
    );
    // The token endpoint is down (503); the API works.
    let mock = Mock::start(vec![
        Reply {
            status: 503,
            body: "{}".into(),
        },
        ok(WHOAMI_OK),
    ]);
    let o = run_whoami(&sb, &mock, &[("LINEAR_RETRIES", "0")]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.requests()[1].authorization.as_deref(),
        Some("Bearer STILL-GOOD")
    );

    // A token that has run out cannot be used: the error is the network's, not "log in again".
    write_credential(&sb, EXPIRED);
    let mock = Mock::start(vec![Reply {
        status: 503,
        body: "{}".into(),
    }]);
    let o = run_whoami(&sb, &mock, &[]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(!stderr(&o).contains("has expired"), "{}", stderr(&o));
    assert_eq!(stored(&sb)["access_token"], "OLD-ACCESS");
}

#[test]
fn a_refreshed_token_goes_back_to_the_keyring_it_came_from() {
    let sb = Sandbox::new();
    add_oauth_workspace(&sb);
    let path = sb.config_dir().join("workspaces.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}credential_store = \"keyring\"\n")).unwrap();
    let dir = sb.root.path().join("keyring");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("example"), EXPIRED).unwrap();
    let mock = Mock::start(vec![
        token_answer("NEW-ACCESS", Some("NEW-REFRESH"), 3600),
        ok(WHOAMI_OK),
    ]);

    let o = run_whoami(&sb, &mock, &[("LINEAR_KEYRING_DIR", dir.to_str().unwrap())]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("example")).unwrap()).unwrap();
    assert_eq!(v["access_token"], "NEW-ACCESS");
    assert_eq!(v["refresh_token"], "NEW-REFRESH");
    assert!(!credentials_path(&sb).exists());
}
