//! `--force`: a write the ownership rules refuse (exit 4) goes through in a workspace that sets
//! `allow_force`, and says so. What it never opens: a workspace without the setting, a
//! validator, a missing `--yes`, a raw mutation.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const COMMENT: &str = "00000000-0000-4000-8000-000000000017";

fn bot() -> Value {
    json!({ "id": BOT, "name": "Linear", "displayName": "linear",
            "email": "linear@example.com", "active": true, "isMe": false })
}

fn view_data() -> Value {
    serde_json::from_str::<Value>(&fixture("project_view")).unwrap()["data"].clone()
}

/// What `project_view` answers when `lead` leads the project.
fn project_led_by(lead: Value) -> Reply {
    let mut v = view_data();
    v["project"]["lead"] = lead;
    data(v)
}

fn my_lead() -> Value {
    view_data()["project"]["lead"].clone()
}

/// The routes of `project update --summary`, for a project led by `lead`.
fn project_routes(lead: Value) -> Vec<(&'static str, Vec<Reply>)> {
    vec![
        ("Whoami", vec![whoami()]),
        ("ProjectRefs", vec![project_refs()]),
        ("ProjectView", vec![project_led_by(lead)]),
        (
            "ProjectUpdate",
            vec![data(
                json!({ "projectUpdate": { "success": true, "project": view_data()["project"] } }),
            )],
        ),
    ]
}

const UPDATE: [&str; 5] = [
    "project",
    "update",
    "Fixture Project",
    "--summary",
    "A new summary",
];

fn with(args: &[&str], extra: &[&str]) -> Vec<String> {
    args.iter().chain(extra).map(|s| s.to_string()).collect()
}

fn run_args(sb: &Sandbox, mock: &Routed, args: &[String]) -> std::process::Output {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(sb, mock, &args)
}

fn forcing() -> Sandbox {
    workspace_with_setting(&[], "allow_force = true")
}

fn written(mock: &Routed, op: &str) -> usize {
    mock.of(op).len()
}

// ------------------------------------------------------------------ the setting

#[test]
fn force_without_allow_force_is_a_usage_error_and_nothing_is_sent() {
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(project_routes(bot()));

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("allow_force"), "{}", stderr(&o));
    assert!(stderr(&o).contains("example"), "{}", stderr(&o));
    // Not even the check of who the key belongs to.
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());

    // `allow_force = false` is the same as leaving it out.
    let sb = workspace_with_setting(&[], "allow_force = false");
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

#[test]
fn allow_force_alone_changes_nothing_without_the_flag() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));
    let o = run_args(&sb, &mock, &with(&UPDATE, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("not the lead"), "{}", stderr(&o));
    assert_eq!(written(&mock, "ProjectUpdate"), 0);
}

// ------------------------------------------------------------------ an override

#[test]
fn force_writes_a_project_somebody_else_leads_and_says_so() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    assert_eq!(written(&mock, "ProjectUpdate"), 1);

    let v = stdout_json(&o);
    assert_eq!(v["forced"], true);
    let overridden = v["overridden"].as_array().unwrap();
    assert_eq!(overridden.len(), 1, "{v}");
    let o0 = &overridden[0];
    assert_eq!(o0["target"], "project \"Fixture Project\"");
    assert_eq!(o0["operation"], "project_update");
    assert_eq!(o0["reason"], "project_not_led");
    assert!(o0["message"].as_str().unwrap().contains("not the lead"));
    assert_eq!(o0["held"], json!([{ "role": "lead", "user": BOT }]));
    // The command's own output is still there.
    assert_eq!(v["workspace"], "example");
    assert!(v["changed"].as_array().is_some());
}

#[test]
fn the_override_is_reported_on_stderr_before_the_write() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("warning: --force overrides the ownership rules"),
        "{err}"
    );
    assert!(err.contains("project \"Fixture Project\""), "{err}");
    assert!(err.contains("project_update"), "{err}");
    assert!(err.contains(&format!("held by: lead {BOT}")), "{err}");

    // Nobody leads it: the report says so.
    let mock = Routed::start(project_routes(Value::Null));
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("held by: lead nobody"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn json_and_quiet_keep_stderr_clear_of_the_report() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!stderr(&o).contains("--force overrides"), "{}", stderr(&o));

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--quiet"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
}

#[test]
fn force_on_a_write_the_rules_allow_reports_nothing() {
    let sb = forcing();
    let mock = Routed::start(project_routes(my_lead()));

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(written(&mock, "ProjectUpdate"), 1);
    let v = stdout_json(&o);
    assert!(v.get("forced").is_none(), "{v}");
    assert!(v.get("overridden").is_none(), "{v}");
    assert!(!stderr(&o).contains("overrides"), "{}", stderr(&o));
}

#[test]
fn an_unforced_write_has_no_forced_field() {
    let sb = forcing();
    let mock = Routed::start(project_routes(my_lead()));
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout_json(&o).get("forced").is_none());
}

#[test]
fn a_lenient_workspace_still_refuses_projects_and_force_opens_them() {
    let sb = workspace_with_setting(&[], "ownership = \"lenient\"\nallow_force = true");
    let mock = Routed::start(project_routes(bot()));

    let o = run_args(&sb, &mock, &with(&UPDATE, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(written(&mock, "ProjectUpdate"), 0);

    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["forced"], true);
    assert_eq!(written(&mock, "ProjectUpdate"), 1);
}

// ------------------------------------------------------------------ comments

fn comment_routes(author: Option<&str>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut c: Value = serde_json::from_str::<Value>(&fixture("issue_comments")).unwrap()["data"]
        ["issue"]["comments"]["nodes"][0]
        .clone();
    c["user"] = match author {
        Some(id) => {
            let mut u = c["user"].clone();
            u["id"] = json!(id);
            u
        }
        None => Value::Null,
    };
    c["issue"] = json!({ "id": "id-1", "identifier": "EX-23", "url": "https://example.com" });
    vec![
        ("Whoami", vec![whoami()]),
        ("CommentQuery", vec![data(json!({ "comment": c }))]),
        (
            "CommentDelete",
            vec![data(json!({ "commentDelete": { "success": true } }))],
        ),
    ]
}

#[test]
fn force_deletes_somebody_elses_comment_even_in_a_lenient_workspace() {
    let sb = workspace_with_setting(&[], "ownership = \"lenient\"\nallow_force = true");
    let mock = Routed::start(comment_routes(Some(BOT)));
    let delete = ["comment", "delete", COMMENT, "--yes"];

    let o = run_args(&sb, &mock, &with(&delete, &[]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(written(&mock, "CommentDelete"), 0);

    let o = run_args(&sb, &mock, &with(&delete, &["--force", "--json"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(written(&mock, "CommentDelete"), 1);
    let v = stdout_json(&o);
    assert_eq!(v["forced"], true);
    assert_eq!(v["deleted"], true);
    assert_eq!(v["overridden"][0]["reason"], "comment_not_owned");
    assert_eq!(
        v["overridden"][0]["held"],
        json!([{ "role": "author", "user": BOT }])
    );
}

#[test]
fn force_does_not_stand_in_for_yes() {
    let sb = forcing();
    let mock = Routed::start(comment_routes(Some(BOT)));
    let o = run(&sb, &mock, &["comment", "delete", COMMENT, "--force"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--yes"), "{}", stderr(&o));
    assert_eq!(written(&mock, "CommentDelete"), 0);
}

// ------------------------------------------------------------------ what it does not open

#[test]
fn a_validator_still_refuses_a_forced_write() {
    let sb = workspace_with_setting(&["template-sections"], "allow_force = true");
    let body = write_file(&sb, "body.md", "## Goal\n\nShip it.\n");
    let mock = Routed::start(project_routes(bot()));

    let mut args = with(&UPDATE[..3], &["--body-file", &body, "--force"]);
    args.push("--json".into());
    let o = run_args(&sb, &mock, &args);
    // Replacing a body without `--template` is refused by the rule (exit 5), not by ownership.
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(written(&mock, "ProjectUpdate"), 0);
}

#[test]
fn force_never_opens_a_raw_mutation() {
    let sb = forcing();
    let mock = Mock::start(vec![ok(WHOAMI_OK)]);
    let o = linear(
        &sb,
        &mock,
        &[
            "api",
            "mutation { issueDelete(id: \"x\") { success } }",
            "--mutation",
        ],
    );
    // `allow_force` is not `allow_raw_mutation`.
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("allow_raw_mutation"), "{}", stderr(&o));
    assert!(mock.requests().is_empty());

    // And `api` takes no `--force`.
    let o = linear(&sb, &mock, &["api", "{ viewer { id } }", "--force"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.requests().is_empty());
}

/// Every command that asks the ownership rules takes `--force`.
const FORCEABLE: [&[&str]; 27] = [
    &["issue", "create"],
    &["issue", "update"],
    &["issue", "comment"],
    &["issue", "link-pr"],
    &["issue", "unlink"],
    &["issue", "attach-file"],
    &["issue", "relate"],
    &["issue", "unrelate"],
    &["issue", "reorder"],
    &["issue", "delete"],
    &["issue", "archive"],
    &["issue", "unarchive"],
    &["comment", "update"],
    &["comment", "delete"],
    &["project", "create"],
    &["project", "update"],
    &["project", "reorder"],
    &["project", "status-update"],
    &["project", "delete"],
    &["project", "unarchive"],
    &["milestone", "create"],
    &["milestone", "update"],
    &["milestone", "delete"],
    &["initiative", "add-project"],
    &["initiative", "remove-project"],
    &["document", "create"],
    &["document", "update"],
];

/// Commands that write but have no ownership rule: nothing to force.
const NOT_FORCEABLE: [&[&str]; 8] = [
    &["initiative", "create"],
    &["initiative", "update"],
    &["initiative", "status-update"],
    &["initiative", "archive"],
    &["file", "upload"],
    &["label", "create"],
    &["template", "create"],
    &["webhook", "create"],
];

#[test]
fn force_is_a_flag_of_exactly_the_commands_that_ask_the_ownership_rules() {
    let sb = forcing();
    let mock = Routed::start(vec![]);
    for command in FORCEABLE {
        let mut args = command.to_vec();
        args.push("--help");
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{command:?}: {}", stderr(&o));
        assert!(
            stdout(&o).contains("--force"),
            "{command:?}: {}",
            stdout(&o)
        );
    }
    for command in NOT_FORCEABLE {
        let mut args = command.to_vec();
        args.push("--help");
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 0, "{command:?}: {}", stderr(&o));
        assert!(
            !stdout(&o).contains("--force"),
            "{command:?}: {}",
            stdout(&o)
        );
    }
    assert!(mock.ops().is_empty());
}

// ------------------------------------------------------------------ --dry-run

#[test]
fn a_dry_run_with_force_plans_the_override_and_sends_nothing() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));

    let o = run_args(
        &sb,
        &mock,
        &with(&UPDATE, &["--force", "--dry-run", "--json"]),
    );
    let v = plan(&o, &mock);
    assert_eq!(planned(&v), ["ProjectUpdate"]);
    assert_eq!(v["forced"], true);
    // The same object the real run reports under `overridden`.
    let overridden = v["overridden"].as_array().unwrap();
    assert_eq!(overridden.len(), 1, "{v}");
    assert_eq!(overridden[0]["target"], "project \"Fixture Project\"");
    assert_eq!(overridden[0]["operation"], "project_update");
    assert_eq!(overridden[0]["reason"], "project_not_led");
    assert_eq!(
        overridden[0]["held"],
        json!([{ "role": "lead", "user": BOT }])
    );
    // Reported on stderr before anything, as in the real run (but not with --json).
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--dry-run"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("--force overrides the ownership rules"),
        "{}",
        stderr(&o)
    );
    assert!(
        stdout(&o).contains("forced: --force would override"),
        "{}",
        stdout(&o)
    );
    assert_eq!(written(&mock, "ProjectUpdate"), 0);
}

#[test]
fn a_dry_run_without_force_is_refused_where_the_real_run_is() {
    let sb = forcing();
    let mock = Routed::start(project_routes(bot()));
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--dry-run"]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    mock.assert_read_only();

    // --force in a workspace that does not allow it: usage error, as for the real run.
    let sb = workspace_with_rules(&[]);
    let mock = Routed::start(project_routes(bot()));
    let o = run_args(&sb, &mock, &with(&UPDATE, &["--force", "--dry-run"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
}

#[test]
fn a_dry_run_with_force_on_a_write_the_rules_allow_overrides_nothing() {
    let sb = forcing();
    let mock = Routed::start(project_routes(my_lead()));
    let o = run_args(
        &sb,
        &mock,
        &with(&UPDATE, &["--force", "--dry-run", "--json"]),
    );
    let v = plan(&o, &mock);
    assert_eq!(v["forced"], false);
    assert_eq!(v["overridden"], json!([]));
}
