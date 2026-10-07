//! The golden audit cases through the real `linear` binary.
//!
//! `crates/wasm/tests/golden/audit/*.json` hold a snapshot, a config, options
//! and a fixed `now`, with the answer of `linear_core::audit`. The wasm core
//! and the native core are held to them in `crates/wasm`; here the binary is
//! too: a mock Linear serves the snapshot, `linear audit --now <now>` audits
//! it, and the findings must be the golden ones. That covers what the other two
//! do not: how the CLI turns `workspaces.toml` into an audit config, what it
//! fetches, and what it prints.
//!
//! The cases that expect an error are about the JSON boundary (an unknown key
//! in a JSON config, a snapshot that is not one) and have no counterpart in the
//! CLI, so they are not run here.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use std::path::PathBuf;

const NOT_FOUND: &str = r#"{"errors":[{"message":"Entity not found: Issue - Could not find referenced Issue.","extensions":{"type":"invalid input","code":"INPUT_ERROR"}}]}"#;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wasm/tests/golden/audit")
}

fn cases() -> Vec<(String, Value)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(golden_dir())
        .expect("the golden audit directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let case = serde_json::from_str(&std::fs::read_to_string(&p).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            (p.file_stem().unwrap().to_string_lossy().into_owned(), case)
        })
        .collect()
}

/// The `workspaces.toml` that gives the CLI the audit config of a golden case.
fn workspaces_toml(name: &str, config: &Value) -> String {
    let mut toml = format!("[workspaces.{name}]\nurl_key = \"example\"\n");
    let list = |key: &str| -> Option<String> {
        let items = config[key].as_array().filter(|a| !a.is_empty())?;
        let quoted: Vec<String> = items.iter().map(|v| v.to_string()).collect();
        Some(format!("[{}]", quoted.join(", ")))
    };
    if let Some(rules) = list("validators") {
        toml.push_str(&format!("rules = {rules}\n"));
    }
    if let Some(kinds) = list("sourceKinds") {
        toml.push_str(&format!("source_kinds = {kinds}\n"));
    }
    let days = [
        ("staleDays", "stale_days"),
        ("statusUpdateDays", "status_update_days"),
        ("prOpenDays", "pr_open_days"),
    ];
    let set: Vec<String> = days
        .iter()
        .filter_map(|(json, key)| config[*json].as_u64().map(|n| format!("{key} = {n}\n")))
        .collect();
    if !set.is_empty() {
        toml.push_str(&format!("[workspaces.{name}.audit]\n{}", set.join("")));
    }
    toml
}

fn page(field: &str, nodes: &Value) -> String {
    json!({"data": {field: {
        "nodes": nodes,
        "pageInfo": {"hasNextPage": false, "endCursor": null}
    }}})
    .to_string()
}

/// What the audit asks for, in order: who am I, the issues, each named issue
/// the listing does not hold, the projects and (for `template-sections`) the
/// templates.
fn replies(snapshot: &Value, options: &Value, templates: bool) -> Vec<Reply> {
    let issues = &snapshot["issues"];
    let has = |identifier: &str| {
        issues.as_array().unwrap().iter().any(|i| {
            i["identifier"]
                .as_str()
                .is_some_and(|id| id.eq_ignore_ascii_case(identifier.trim()))
        })
    };
    let mut replies = vec![ok(&fixture("whoami")), ok(&page("issues", issues))];
    for named in options["issues"].as_array().into_iter().flatten() {
        if !has(named.as_str().unwrap()) {
            replies.push(ok(NOT_FOUND));
        }
    }
    replies.push(ok(&page("projects", &snapshot["projects"])));
    if templates {
        replies.push(ok(
            &json!({"data": {"templates": snapshot["templates"]}}).to_string()
        ));
    }
    replies
}

fn audit_args(now: &str, options: &Value) -> Vec<String> {
    let mut args = vec![
        "audit".to_owned(),
        "--json".to_owned(),
        "--now".to_owned(),
        now.to_owned(),
    ];
    if let Some(issues) = options["issues"].as_array() {
        let ids: Vec<&str> = issues.iter().map(|i| i.as_str().unwrap()).collect();
        args.push("--issues".to_owned());
        args.push(ids.join(","));
    }
    if let Some(since) = options["since"].as_str() {
        args.push("--since".to_owned());
        args.push(since.to_owned());
    }
    args
}

#[test]
fn the_binary_gives_the_golden_audit_answers() {
    let mut ran = Vec::new();
    for (name, case) in cases() {
        if case["expected"]["ok"] != true {
            continue;
        }
        let input = &case["input"];
        let snapshot = &input["snapshot"];
        let workspace = snapshot["workspace"].as_str().unwrap();
        let config = if input["config"].is_null() {
            &Value::Null
        } else {
            &input["config"]
        };

        let sb = Sandbox::new();
        std::fs::create_dir_all(sb.config_dir()).unwrap();
        std::fs::write(
            sb.config_dir().join("workspaces.toml"),
            workspaces_toml(workspace, config),
        )
        .unwrap();
        let templates = config["validators"]
            .as_array()
            .is_some_and(|v| v.contains(&json!("template-sections")));
        let mock = Mock::start(replies(snapshot, &input["options"], templates));

        let args = audit_args(input["now"].as_str().unwrap(), &input["options"]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let key = format!("LINEAR_API_KEY_{}", workspace.to_uppercase());
        let o = sb.run(&args, Some(&mock), &[(key.as_str(), KEY)]);
        assert_eq!(code(&o), 0, "{name}: {}", stderr(&o));
        assert_no_leak(&o);

        let got = stdout_json(&o);
        let want = &case["expected"]["data"];
        assert_eq!(got["findings"], want["findings"], "{name}: findings");
        assert_eq!(
            got["unresolved_issues"], want["unresolvedIssues"],
            "{name}: unresolved issues"
        );
        assert_eq!(got["failed_workspaces"], json!([]), "{name}");
        ran.push(name);
    }
    assert!(
        ran.len() >= 8,
        "only {} golden cases ran: {ran:?}",
        ran.len()
    );
    for needed in ["scoped-to-issues-since", "validators-on-existing-issues"] {
        assert!(ran.iter().any(|n| n == needed), "{needed} did not run");
    }
}

#[test]
fn the_golden_cases_do_not_depend_on_the_clock() {
    // Without --now the same snapshot is audited as of today, and the answer
    // differs: the golden cases pass because of the pinned time.
    let (name, case) = cases()
        .into_iter()
        .find(|(n, _)| n == "rules-default")
        .unwrap();
    let input = &case["input"];
    let workspace = input["snapshot"]["workspace"].as_str().unwrap();
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.config_dir()).unwrap();
    std::fs::write(
        sb.config_dir().join("workspaces.toml"),
        workspaces_toml(workspace, &Value::Null),
    )
    .unwrap();
    let mock = Mock::start(replies(&input["snapshot"], &Value::Null, false));
    let key = format!("LINEAR_API_KEY_{}", workspace.to_uppercase());
    let o = sb.run(&["audit", "--json"], Some(&mock), &[(key.as_str(), KEY)]);
    assert_eq!(code(&o), 0, "{name}: {}", stderr(&o));
    assert_ne!(
        stdout_json(&o)["findings"],
        case["expected"]["data"]["findings"],
        "the case does not depend on `now`, so it does not test --now"
    );
}

#[test]
fn now_is_hidden_and_must_be_a_time() {
    let sb = workspace();
    let o = sb.run(&["audit", "--help"], None, &[]);
    assert!(!stdout(&o).contains("--now"), "{}", stdout(&o));

    let mock = Mock::start(vec![]);
    let o = linear(&sb, &mock, &["audit", "--now", "yesterday"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}
