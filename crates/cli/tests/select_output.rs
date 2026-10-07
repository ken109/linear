//! `--fields` and `--id-only`: cutting down the output of the list and view commands.

mod common;
mod read_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};

/// A read command and the replies the mock gives it, in order.
struct Case {
    args: &'static [&'static str],
    replies: Vec<String>,
}

fn cases() -> Vec<Case> {
    let case = |args: &'static [&'static str], replies: Vec<String>| Case { args, replies };
    vec![
        case(&["issue", "list"], vec![fixture("issue_list")]),
        case(&["issue", "search", "x"], vec![fixture("issue_search")]),
        case(&["issue", "view", "EX-23"], vec![fixture("issue_view")]),
        case(&["project", "list"], vec![fixture("projects")]),
        case(
            &["project", "view", "aaaaaaaaaaaa"],
            vec![PROJECT_REFS.to_owned(), fixture("project_view")],
        ),
        case(&["team", "list"], vec![fixture("teams")]),
        case(&["team", "view", "ex"], vec![fixture("teams"); 2]),
        case(&["user", "list"], vec![fixture("users")]),
        case(&["label", "list"], vec![fixture("labels")]),
        case(&["initiative", "list"], vec![fixture("initiatives")]),
        case(&["user", "view", "me"], vec![fixture("users"); 2]),
        case(&["label", "view", "area/api"], vec![fixture("labels"); 3]),
        case(&["webhook", "list"], vec![fixture("webhooks")]),
        case(&["document", "list"], vec![fixture("documents")]),
        case(
            &["milestone", "list", "--project", "Fixture Project"],
            vec![PROJECT_REFS.to_owned(), fixture("milestones")],
        ),
        case(&["cycle", "list"], vec![fixture("cycle_infos"); 2]),
        case(
            &["template", "list"],
            vec![fixture("templates_sections"); 2],
        ),
    ]
}

fn run_case(sb: &Sandbox, case: &Case, extra: &[&str]) -> std::process::Output {
    let mock = Mock::start(case.replies.iter().map(|r| ok(r)).collect());
    let args: Vec<&str> = case.args.iter().chain(extra).copied().collect();
    linear(sb, &mock, &args)
}

fn rows(v: Value) -> Vec<Value> {
    match v {
        Value::Array(items) => items,
        other => vec![other],
    }
}

#[test]
fn fields_keeps_only_the_named_keys_on_every_read_command() {
    let sb = workspace();
    for case in cases() {
        let full = stdout_json(&run_case(&sb, &case, &["--json"]));
        let was_list = full.is_array();
        let full = rows(full);
        let o = run_case(&sb, &case, &["--json", "--fields", "id,workspace"]);
        assert_eq!(code(&o), 0, "{:?}: {}", case.args, stderr(&o));
        let cut = stdout_json(&o);
        assert_eq!(cut.is_array(), was_list, "{:?}", case.args);
        let cut = rows(cut);
        assert_eq!(cut.len(), full.len(), "{:?}", case.args);
        for (cut, full) in cut.iter().zip(&full) {
            let mut keys: Vec<&str> = cut
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(keys, ["id", "workspace"], "{:?}", case.args);
            assert_eq!(cut["id"], full["id"], "{:?}", case.args);
        }
    }
}

#[test]
fn id_only_prints_the_ids_one_per_line_and_as_an_array_with_json() {
    let sb = workspace();
    for case in cases() {
        let full = rows(stdout_json(&run_case(&sb, &case, &["--json"])));
        let ids: Vec<String> = full
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(!ids.is_empty(), "{:?}", case.args);

        let o = run_case(&sb, &case, &["--id-only"]);
        assert_eq!(code(&o), 0, "{:?}: {}", case.args, stderr(&o));
        assert_eq!(
            stdout(&o),
            format!("{}\n", ids.join("\n")),
            "{:?}",
            case.args
        );

        let o = run_case(&sb, &case, &["--json", "--id-only"]);
        assert_eq!(stdout_json(&o), json!(ids), "{:?}", case.args);
    }
}

#[test]
fn an_unknown_field_is_a_usage_error_that_lists_the_valid_ones() {
    let sb = workspace();
    let case = &cases()[0];
    let o = run_case(&sb, case, &["--json", "--fields", "identifier,nope,bogus"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    let err: Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(err["error"]["code"], "usage");
    let message = err["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("unknown fields \"nope\", \"bogus\""),
        "{message}"
    );
    // The valid names are the keys of the output, sorted.
    assert!(
        message.contains("the valid fields are: ") && message.contains("identifier, "),
        "{message}"
    );
    assert!(!message.contains("identifier,nope"), "{message}");

    // One name is singular.
    let o = run_case(&sb, case, &["--json", "--fields", "nope"]);
    assert!(
        stderr(&o).contains("unknown field \\\"nope\\\" for"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn fields_are_checked_against_the_output_of_the_command_that_ran() {
    let sb = workspace();
    // `slugId` is a project's key, not an issue's.
    let o = run_case(&sb, &cases()[0], &["--json", "--fields", "slugId"]);
    assert_eq!(code(&o), 2);
    let o = run_case(&sb, &cases()[3], &["--json", "--fields", "slugId,name"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        rows(stdout_json(&o))[0].as_object().unwrap().len(),
        2,
        "{}",
        stdout(&o)
    );
}

#[test]
fn a_view_with_fields_is_one_object_in_the_order_of_the_json() {
    let sb = workspace();
    let o = run_case(
        &sb,
        &cases()[2],
        &["--json", "--fields", "identifier,description"],
    );
    let v = stdout_json(&o);
    assert_eq!(v["identifier"], "EX-23");
    assert_eq!(v["description"], "Body of the fixture issue.");
    assert_eq!(v.as_object().unwrap().len(), 2);
}

#[test]
fn an_empty_list_stays_empty_with_either_flag() {
    let sb = workspace();
    let empty = r#"{"data":{"searchIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let case = Case {
        args: &["issue", "search", "nothing"],
        replies: vec![empty.to_owned(); 4],
    };
    let o = run_case(&sb, &case, &["--json", "--fields", "id"]);
    assert_eq!(
        (code(&o), stdout(&o).trim().to_owned()),
        (0, "[]".to_owned())
    );
    let o = run_case(&sb, &case, &["--id-only"]);
    assert_eq!((code(&o), stdout(&o)), (0, String::new()));
    let o = run_case(&sb, &case, &["--json", "--id-only"]);
    assert_eq!(stdout(&o).trim(), "[]");
}

#[test]
fn bad_combinations_are_usage_errors_before_anything_is_sent() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    for (extra, want) in [
        (&["--fields", "id"][..], "needs --json"),
        (&["--json", "--fields", "id,,name"], "empty name"),
        (
            &["--json", "--fields", "id", "--id-only"],
            "cannot be combined with --fields",
        ),
        (&["--id-only", "--quiet"], "--quiet"),
    ] {
        let args: Vec<&str> = ["issue", "list"].iter().chain(extra).copied().collect();
        let o = linear(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(want), "{args:?}: {}", stderr(&o));
    }
    assert!(mock.requests().is_empty());
}

#[test]
fn commands_that_cannot_honour_the_flags_refuse_them_before_any_request() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("issue_list"))]);
    for args in [
        &[
            "issue",
            "create",
            "--project",
            "p",
            "--title",
            "x",
            "--json",
            "--fields",
            "id",
        ][..],
        &["issue", "delete", "EX-1", "--id-only"],
        &["project", "update", "x", "--json", "--fields", "id"],
        &["workspace", "list", "--id-only"],
        &["audit", "--json", "--fields", "id"],
        &["api", "{ viewer { id } }", "--json", "--fields", "id"],
    ] {
        let o = linear(&sb, &mock, args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        let err = stderr(&o);
        assert!(
            err.contains("does not apply to this command")
                && err.contains("issue list|search|view"),
            "{args:?}: {err}"
        );
    }
    assert!(mock.requests().is_empty());
}

#[test]
fn without_the_flags_the_output_is_unchanged() {
    let sb = workspace();
    let case = &cases()[0];
    let plain = stdout(&run_case(&sb, case, &["--json"]));
    assert!(plain.contains("\"description\""), "{plain}");
    let o = run_case(&sb, case, &[]);
    assert!(stdout(&o).starts_with("ID"), "{}", stdout(&o));
}
