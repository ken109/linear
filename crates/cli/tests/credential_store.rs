//! `credential_store = "keyring"`: the credential lives in the OS keyring, with the
//! file as the fallback where there is none. The tests run the real binary, so the
//! keyring is replaced by a directory (`LINEAR_KEYRING_DIR`) or switched off
//! (`LINEAR_KEYRING=off`): they never touch the keyring of the machine they run on.

mod common;
use common::*;
use serde_json::Value;

const KEY: &str = "lin_api_KEYRINGTEST_do_not_leak_0123456789";

fn add_example(sb: &Sandbox) {
    let o = sb.run(
        &["workspace", "add", "example", "--url-key", "example"],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

fn keyring_dir(sb: &Sandbox) -> std::path::PathBuf {
    sb.root.path().join("keyring")
}

fn credentials_file(sb: &Sandbox) -> std::path::PathBuf {
    sb.config_dir().join("credentials").join("example.json")
}

fn config_text(sb: &Sandbox) -> String {
    std::fs::read_to_string(sb.config_dir().join("workspaces.toml")).unwrap()
}

fn listed_credentials(sb: &Sandbox, env: &[(&str, &str)]) -> Value {
    let o = sb.run(&["workspace", "list", "--json"], None, env);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    v[0]["credentials"].clone()
}

#[test]
fn login_with_keyring_stores_the_key_there_and_remembers_the_choice() {
    let sb = Sandbox::new();
    add_example(&sb);
    let dir = keyring_dir(&sb);
    let env = [("LINEAR_KEYRING_DIR", dir.to_str().unwrap())];
    let mock = Mock::start(vec![ok(WHOAMI_OK), ok(WHOAMI_OK)]);

    let o = sb.run_stdin(
        &["workspace", "login", "example", "--with-token", "--keyring"],
        Some(&mock),
        &env,
        Some(KEY),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Stored credentials in the OS keyring"),
        "{}",
        stderr(&o)
    );
    assert!(!stdout(&o).contains(KEY) && !stderr(&o).contains(KEY));

    // Same JSON as the file holds, and no file.
    let entry = std::fs::read_to_string(dir.join("example")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&entry).unwrap()["api_key"],
        KEY
    );
    assert!(!credentials_file(&sb).exists());
    assert!(
        config_text(&sb).contains("credential_store = \"keyring\""),
        "{}",
        config_text(&sb)
    );

    // Later runs find it there; `list` and `whoami` say so and never print it.
    assert_eq!(listed_credentials(&sb, &env), "keyring");
    let o = sb.run(&["workspace", "whoami", "--json"], Some(&mock), &env);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["credentials"], "keyring");
    assert!(!stdout(&o).contains(KEY));
    assert_eq!(mock.requests()[1].authorization.as_deref(), Some(KEY));
}

#[test]
fn without_a_keyring_login_falls_back_to_the_file_and_says_so() {
    let sb = Sandbox::new();
    add_example(&sb);
    let env = [("LINEAR_KEYRING", "off")];
    let mock = Mock::start(vec![ok(WHOAMI_OK), ok(WHOAMI_OK)]);

    let o = sb.run_stdin(
        &["workspace", "login", "example", "--with-token", "--keyring"],
        Some(&mock),
        &env,
        Some(KEY),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("OS keyring is not available"),
        "{}",
        stderr(&o)
    );
    assert!(credentials_file(&sb).is_file());
    // The workspace was not switched to a store this machine cannot use.
    assert!(!config_text(&sb).contains("credential_store"));
    assert_eq!(listed_credentials(&sb, &env), "file");

    // A workspace set to the keyring still works on a machine without one.
    let o = sb.run(
        &["workspace", "whoami", "--json"],
        Some(&mock),
        &[
            ("LINEAR_KEYRING", "off"),
            ("LINEAR_CREDENTIAL_STORE", "keyring"),
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["credentials"], "file");
}

#[test]
fn migrate_moves_the_credential_both_ways_and_updates_the_config() {
    let sb = Sandbox::new();
    add_example(&sb);
    let dir = keyring_dir(&sb);
    let env = [("LINEAR_KEYRING_DIR", dir.to_str().unwrap())];
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = sb.run_stdin(
        &["workspace", "login", "example", "--with-token"],
        Some(&mock),
        &env,
        Some(KEY),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(credentials_file(&sb).is_file());
    let before = std::fs::read_to_string(credentials_file(&sb)).unwrap();

    let o = sb.run(&["workspace", "migrate", "example", "--json"], None, &env);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        (v["from"].as_str(), v["to"].as_str()),
        (Some("file"), Some("keyring"))
    );
    assert!(!stdout(&o).contains(KEY));
    assert!(!credentials_file(&sb).exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("example")).unwrap(),
        before
    );
    assert!(config_text(&sb).contains("credential_store = \"keyring\""));
    assert_eq!(listed_credentials(&sb, &env), "keyring");

    let o = sb.run(
        &["workspace", "migrate", "example", "--to", "file"],
        None,
        &env,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(credentials_file(&sb)).unwrap(),
        before
    );
    assert!(!dir.join("example").exists());
    assert!(!config_text(&sb).contains("credential_store"));
    assert_eq!(listed_credentials(&sb, &env), "file");

    // Nothing left to move.
    let o = sb.run(
        &["workspace", "migrate", "example", "--to", "file"],
        None,
        &env,
    );
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("no credential in the OS keyring"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn migrate_to_a_missing_keyring_leaves_the_file_alone() {
    let sb = Sandbox::new();
    add_example(&sb);
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    sb.run_stdin(
        &["workspace", "login", "example", "--with-token"],
        Some(&mock),
        &[],
        Some(KEY),
    );

    let o = sb.run(
        &["workspace", "migrate", "example"],
        None,
        &[("LINEAR_KEYRING", "off")],
    );
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("nothing was moved"), "{}", stderr(&o));
    assert!(credentials_file(&sb).is_file());
    assert!(!config_text(&sb).contains("credential_store"));
}

#[test]
fn the_environment_can_choose_the_store_and_a_bad_value_is_refused() {
    let sb = Sandbox::new();
    add_example(&sb);
    let dir = keyring_dir(&sb);
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = sb.run_stdin(
        &["workspace", "login", "example", "--with-token"],
        Some(&mock),
        &[
            ("LINEAR_KEYRING_DIR", dir.to_str().unwrap()),
            ("LINEAR_CREDENTIAL_STORE", "keyring"),
        ],
        Some(KEY),
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(dir.join("example").is_file());
    assert!(!credentials_file(&sb).exists());

    let o = sb.run(
        &["workspace", "whoami"],
        Some(&mock),
        &[("LINEAR_CREDENTIAL_STORE", "vault")],
    );
    assert_eq!(code(&o), 2);
    assert!(
        stderr(&o).contains("LINEAR_CREDENTIAL_STORE"),
        "{}",
        stderr(&o)
    );
}
