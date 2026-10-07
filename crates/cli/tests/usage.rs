//! `linear usage` and `linear <group> usage`: they need no configuration, credentials or
//! network, and are short enough for an agent to read first.

mod common;

use common::*;

fn usage(args: &[&str]) -> std::process::Output {
    // No mock server and no credentials: the command must not try to reach Linear.
    Sandbox::new().run(
        args,
        None,
        &[("LINEAR_API_URL", "http://127.0.0.1:9/graphql")],
    )
}

const GROUPS: &[&str] = &[
    "workspace",
    "issue",
    "comment",
    "project",
    "milestone",
    "initiative",
    "file",
    "template",
    "label",
    "team",
    "user",
    "cycle",
    "document",
    "cache",
    "webhook",
];

#[test]
fn the_overview_is_short_and_needs_nothing() {
    let o = usage(&["usage"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    // Four characters to a token is the usual estimate; the ticket's limit is 1000 tokens.
    assert!(
        text.chars().count() < 4000,
        "{} characters",
        text.chars().count()
    );
    for want in [
        "COMMANDS",
        "GLOBAL FLAGS",
        "EXIT CODES",
        "SAFETY",
        "--workspace",
        "--json",
        "ownership",
        "--yes",
        "4 ownership rules refused the write",
        "5 a validator refused the write",
    ] {
        assert!(text.contains(want), "no {want:?} in:\n{text}");
    }
    for group in GROUPS {
        assert!(
            text.contains(&format!("  {group} ")),
            "no {group} in:\n{text}"
        );
    }
    assert_eq!(stderr(&o), "");
}

#[test]
fn every_group_prints_its_commands_and_flags() {
    for group in GROUPS {
        let o = usage(&[group, "usage"]);
        assert_eq!(code(&o), 0, "{group}: {}", stderr(&o));
        let text = stdout(&o);
        assert!(text.starts_with(&format!("linear {group}: ")), "{text}");
        assert!(
            text.contains(&format!("\nlinear {group} ")),
            "{group}:\n{text}"
        );
    }
    let text = stdout(&usage(&["issue", "usage"]));
    for want in [
        "\nlinear issue create [write",
        "--source <URL>",
        "\nlinear issue unlink [write, needs --yes",
        "\nlinear issue list [read]",
        "--state-type <triage|backlog|",
    ] {
        assert!(text.contains(want), "no {want:?} in:\n{text}");
    }
    // `cycle` takes a date as well as subcommands.
    assert!(stdout(&usage(&["cycle", "usage"])).contains("linear cycle <DATE> [read]"));
}

#[test]
fn json_wraps_the_text_in_one_key() {
    let o = usage(&["usage", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(v["usage"].as_str().unwrap().starts_with("linear: "));
}

#[test]
fn usage_is_not_a_command_to_cut_down() {
    let o = usage(&["usage", "--json", "--fields", "usage"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}
