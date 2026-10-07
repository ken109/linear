//! `linear api --mutation`: sent only when the workspace's `allow_raw_mutation`
//! is true, warned about on stderr, and checked against the workspace first.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

const MUTATION: &str = "mutation { issueDelete(id: \"x\") { success } }";

/// A sandbox with workspace `example`, which does or does not allow raw mutations.
fn sandbox(allow: Option<bool>) -> Sandbox {
    let sb = workspace();
    if let Some(allow) = allow {
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&format!("\nallow_raw_mutation = {allow}\n"));
        std::fs::write(&path, text).unwrap();
    }
    sb
}

fn api(sb: &Sandbox, mock: &Mock, args: &[&str]) -> std::process::Output {
    let mut full = vec!["api"];
    full.extend_from_slice(args);
    linear(sb, mock, &full)
}

fn bodies(mock: &Mock) -> Vec<Value> {
    mock.requests()
        .iter()
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

#[test]
fn with_the_setting_on_the_mutation_is_sent_after_checking_the_workspace() {
    let sb = sandbox(Some(true));
    let mock = Mock::start(vec![
        ok(WHOAMI_OK),
        ok(r#"{"data":{"issueDelete":{"success":true}}}"#),
    ]);
    let o = api(&sb, &mock, &[MUTATION, "--mutation"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&o)).unwrap(),
        json!({ "issueDelete": { "success": true } })
    );

    // Whoever the key belongs to is checked first, then the document is sent as written.
    let sent = bodies(&mock);
    assert_eq!(sent.len(), 2);
    assert!(sent[0]["query"].as_str().unwrap().contains("viewer"));
    assert_eq!(sent[1]["query"], MUTATION);

    // The warning says what is bypassed and names the workspace.
    let warning = stderr(&o);
    assert!(warning.starts_with("warning:"), "{warning}");
    assert!(warning.contains("ownership rules"), "{warning}");
    assert!(warning.contains("validators"), "{warning}");
    assert!(warning.contains("\"example\""), "{warning}");
}

#[test]
fn json_and_quiet_get_the_response_and_no_warning() {
    for flag in ["--json", "--quiet"] {
        let sb = sandbox(Some(true));
        let mock = Mock::start(vec![
            ok(WHOAMI_OK),
            ok(r#"{"data":{"issueDelete":{"success":true}}}"#),
        ]);
        let o = api(&sb, &mock, &[MUTATION, "--mutation", flag]);
        assert_eq!(code(&o), 0, "{flag}: {}", stderr(&o));
        assert_eq!(stderr(&o), "", "{flag}: a script gets only the response");
        assert!(stdout(&o).contains("issueDelete"));
    }
}

#[test]
fn with_the_setting_off_or_missing_the_mutation_is_refused_and_names_the_setting() {
    for setting in [None, Some(false)] {
        let sb = sandbox(setting);
        let mock = Mock::start(vec![ok(WHOAMI_OK)]);
        let o = api(&sb, &mock, &[MUTATION, "--mutation", "--json"]);
        assert_eq!(code(&o), 4, "{setting:?}: {}", stderr(&o));
        assert_eq!(stdout(&o), "");
        let e: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
        assert_eq!(e["error"]["code"], "write_denied");
        let message = e["error"]["message"].as_str().unwrap();
        assert!(message.contains("allow_raw_mutation = true"), "{message}");
        assert!(message.contains("workspaces.toml"), "{message}");
        assert!(mock.requests().is_empty(), "nothing may reach Linear");
    }
}

#[test]
fn without_the_flag_a_mutation_is_refused_even_when_the_setting_is_on() {
    let sb = sandbox(Some(true));
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = api(&sb, &mock, &[MUTATION]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("--mutation"), "{}", stderr(&o));
    assert!(mock.requests().is_empty());

    // Hidden in a document with a query, or behind --operation-name, it still counts.
    let doc = format!(
        "query Q {{ viewer {{ id }} }}\n{}",
        "mutation M { issueDelete(id: \"x\") { success } }"
    );
    let o = api(&sb, &mock, &[&doc, "--operation-name", "Q"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}

#[test]
fn credentials_for_another_workspace_do_not_get_to_write() {
    let sb = sandbox(Some(true));
    let other = WHOAMI_OK.replace("\"urlKey\":\"example\"", "\"urlKey\":\"someone-else\"");
    let mock = Mock::start(vec![ok(&other), ok(r#"{"data":{"x":1}}"#)]);
    let o = api(&sb, &mock, &[MUTATION, "--mutation"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("someone-else"), "{}", stderr(&o));
    // Only the identity check was sent; the mutation never was, and no warning claimed it was.
    assert_eq!(mock.requests().len(), 1);
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
}

#[test]
fn the_flag_with_a_plain_query_just_runs_the_query_without_a_warning() {
    let sb = sandbox(Some(true));
    let mock = Mock::start(vec![ok(r#"{"data":{"a":1}}"#)]);
    let o = api(&sb, &mock, &["{ a }", "--mutation"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stderr(&o), "");
    assert_eq!(mock.requests().len(), 1, "no identity check for a query");
}

#[test]
fn subscriptions_stay_a_usage_error_with_the_setting_on() {
    let sb = sandbox(Some(true));
    let mock = Mock::start(vec![]);
    let o = api(&sb, &mock, &["subscription { a }", "--mutation"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}

#[test]
fn a_misspelled_setting_is_a_configuration_error_not_ignored() {
    let sb = workspace();
    let path = sb.config_dir().join("workspaces.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\nallow_raw_mutations = true\n");
    std::fs::write(&path, text).unwrap();
    let mock = Mock::start(vec![]);
    let o = api(&sb, &mock, &[MUTATION, "--mutation"]);
    assert_ne!(code(&o), 0);
    assert!(stderr(&o).contains("allow_raw_mutations"), "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}

#[test]
fn a_dry_run_shows_the_document_and_the_variables_and_sends_no_mutation() {
    let sb = sandbox(Some(true));
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = api(
        &sb,
        &mock,
        &[
            "mutation Drop($id: String!) { issueDelete(id: $id) { success } }",
            "--mutation",
            "--var",
            "id=x",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["command"], "api --mutation");
    assert_eq!(v["mutations"][0]["variables"], json!({ "id": "x" }));
    assert!(v["mutations"][0]["query"]
        .as_str()
        .unwrap()
        .contains("issueDelete"));
    // Only the workspace check was asked of Linear; the document was not sent.
    let sent = bodies(&mock);
    assert_eq!(sent.len(), 1);
    assert!(sent[0]["query"].as_str().unwrap().contains("viewer"));

    // The same gates as the real run: the setting (exit 4) and the flag.
    let off = sandbox(None);
    let mock = Mock::start(vec![]);
    let o = api(&off, &mock, &[MUTATION, "--mutation", "--dry-run"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
    let o = api(&sb, &mock, &[MUTATION, "--dry-run"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    // A query has nothing to preview.
    let o = api(
        &sb,
        &mock,
        &["{ viewer { id } }", "--mutation", "--dry-run"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}
