//! Integration tests for `linear workspace ...` against a mock Linear server.

mod common;
use common::*;

const KEY: &str = "lin_api_TESTKEY_do_not_leak_0123456789";

fn add_example(sb: &Sandbox, name: &str, url_key: &str) {
    let o = sb.run(
        &[
            "workspace",
            "add",
            name,
            "--url-key",
            url_key,
            "--team",
            "EX",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn list_is_empty_before_any_workspace_is_added() {
    let sb = Sandbox::new();
    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o).trim(), "[]");

    let o = sb.run(&["workspace", "list"], None, &[]);
    assert!(stdout(&o).contains("No workspaces configured"));
}

#[test]
fn add_then_list_shows_the_workspace_and_marks_the_first_as_default() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    add_example(&sb, "other", "other-co");

    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["name"], "example");
    assert_eq!(v[0]["default"], true);
    assert_eq!(v[0]["urlKey"], "example");
    assert_eq!(v[0]["defaultTeam"], "EX");
    assert_eq!(v[0]["auth"], "api-key");
    assert_eq!(v[0]["credentials"], serde_json::Value::Null);
    assert_eq!(v[1]["default"], false);

    let o = sb.run(&["workspace", "list", "--quiet"], None, &[]);
    assert_eq!(stdout(&o), "example\nother\n");
}

#[test]
fn add_rejects_duplicates_and_unsafe_names() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");

    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "x"],
        None,
        &[],
    );
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("already exists"));

    for bad in ["../evil", "Upper", "a/b", ""] {
        let o = sb.run(&["workspace", "add", bad, "--url-key", "x"], None, &[]);
        assert_ne!(code(&o), 0, "{bad:?} must be rejected");
    }
}

#[test]
fn login_verifies_the_key_stores_it_privately_and_never_prints_it() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);

    let o = sb.run_stdin(
        &["workspace", "login", "example", "--with-token"],
        Some(&mock),
        &[],
        Some(&format!("{KEY}\n")),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("Logged in to example as Alice Example"));
    assert!(!stdout(&o).contains(KEY) && !stderr(&o).contains(KEY));

    // The key reached Linear as a bare Authorization header (no "Bearer").
    let seen = mock.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].authorization.as_deref(), Some(KEY));

    let file = sb.config_dir().join("credentials").join("example.json");
    assert!(file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let dir = std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir, 0o700);
    }

    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v[0]["credentials"], "file");
    assert!(!stdout(&o).contains(KEY));
}

#[test]
fn login_refuses_a_key_for_the_wrong_workspace_and_saves_nothing() {
    let sb = Sandbox::new();
    add_example(&sb, "mine", "someone-elses");
    let mock = Mock::start(vec![ok(WHOAMI_OK)]); // answers as workspace "example"

    let o = sb.run_stdin(
        &["workspace", "login", "mine", "--with-token"],
        Some(&mock),
        &[],
        Some(KEY),
    );
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("belong to workspace"));
    assert!(!stderr(&o).contains(KEY));
    assert!(!sb
        .config_dir()
        .join("credentials")
        .join("mine.json")
        .exists());
}

#[test]
fn login_without_a_terminal_or_token_flag_is_a_usage_error() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let o = sb.run(&["workspace", "login", "example"], None, &[]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("--with-token"));
}

#[test]
fn whoami_uses_the_stored_credentials() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let mock = Mock::start(vec![ok(WHOAMI_OK), ok(WHOAMI_OK)]);
    sb.run_stdin(
        &["workspace", "login", "example", "--with-token"],
        Some(&mock),
        &[],
        Some(KEY),
    );

    let o = sb.run(&["workspace", "whoami", "--json"], Some(&mock), &[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["user"]["email"], "alice@example.com");
    assert_eq!(v["organization"]["urlKey"], "example");
    assert_eq!(v["credentials"], "file");
    assert!(!stdout(&o).contains(KEY));
    assert_eq!(mock.requests()[1].authorization.as_deref(), Some(KEY));
}

#[test]
fn environment_variable_overrides_the_stored_key() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);

    let o = sb.run(
        &["workspace", "whoami", "--json"],
        Some(&mock),
        &[("LINEAR_API_KEY_EXAMPLE", "from-env-key")],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["credentials"], "env");
    assert_eq!(
        mock.requests()[0].authorization.as_deref(),
        Some("from-env-key")
    );
}

#[test]
fn whoami_without_credentials_is_an_auth_error_with_a_json_body() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");

    let o = sb.run(&["workspace", "whoami", "--json"], None, &[]);
    assert_eq!(code(&o), 3);
    assert_eq!(stdout(&o), "");
    let e: serde_json::Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(e["error"]["code"], "auth");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("linear workspace login"));
}

#[test]
fn a_401_from_linear_is_an_auth_error() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let mock = Mock::start(vec![Reply {
        status: 401,
        body: r#"{"errors":[{"message":"Authentication required, not authenticated","extensions":{"type":"authentication error","code":"AUTHENTICATION_ERROR"}}]}"#.into(),
    }]);
    let o = sb.run(
        &["workspace", "whoami"],
        Some(&mock),
        &[("LINEAR_API_KEY_EXAMPLE", KEY)],
    );
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).starts_with("error: "));
    assert!(!stderr(&o).contains(KEY));
}

#[test]
fn a_credentials_file_readable_by_others_is_refused() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let sb = Sandbox::new();
        add_example(&sb, "example", "example");
        let dir = sb.config_dir().join("credentials");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("example.json");
        std::fs::write(&f, format!(r#"{{"kind":"api-key","api_key":"{KEY}"}}"#)).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();

        let o = sb.run(&["workspace", "whoami"], None, &[]);
        assert_eq!(code(&o), 3);
        assert!(stderr(&o).contains("chmod 600"));
        assert!(!stderr(&o).contains(KEY));
    }
}

#[test]
fn workspace_resolution_order_is_flag_env_repo_file_default() {
    let sb = Sandbox::new();
    add_example(&sb, "alpha", "example"); // becomes the default
    add_example(&sb, "beta", "example");
    add_example(&sb, "gamma", "example");
    add_example(&sb, "delta", "example");
    let env = [
        ("LINEAR_API_KEY_ALPHA", "k"),
        ("LINEAR_API_KEY_BETA", "k"),
        ("LINEAR_API_KEY_GAMMA", "k"),
        ("LINEAR_API_KEY_DELTA", "k"),
    ];
    let who = |args: &[&str], extra: &[(&str, &str)]| -> String {
        let mock = Mock::start(vec![ok(WHOAMI_OK)]);
        let mut e = env.to_vec();
        e.extend_from_slice(extra);
        let o = sb.run(args, Some(&mock), &e);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
        v["workspace"].as_str().unwrap().to_owned()
    };

    // 4. config default
    assert_eq!(who(&["workspace", "whoami", "--json"], &[]), "alpha");
    // 3. repo file beats the default
    std::fs::write(sb.cwd().join(".linear.toml"), "workspace = \"beta\"\n").unwrap();
    assert_eq!(who(&["workspace", "whoami", "--json"], &[]), "beta");
    // 2. environment beats the repo file
    assert_eq!(
        who(
            &["workspace", "whoami", "--json"],
            &[("LINEAR_WORKSPACE", "gamma")]
        ),
        "gamma"
    );
    // 1. the flag beats everything
    assert_eq!(
        who(
            &["--workspace", "delta", "workspace", "whoami", "--json"],
            &[("LINEAR_WORKSPACE", "gamma")]
        ),
        "delta"
    );
}

#[test]
fn an_unknown_workspace_is_a_usage_error_that_lists_the_known_ones() {
    let sb = Sandbox::new();
    add_example(&sb, "example", "example");
    let o = sb.run(&["-w", "nope", "workspace", "whoami"], None, &[]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("unknown workspace"));
    assert!(stderr(&o).contains("example"));
}
