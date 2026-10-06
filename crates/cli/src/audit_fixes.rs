//! Every `fix` the audit prints is a command this CLI really accepts.
//!
//! The audit lives in `linear-core` and only knows the fix as text, so nothing
//! stops a rule from naming a flag that does not exist (it did: `project
//! status-update --body`, `issue update --labels`). This test drives the audit
//! over the cases of the golden files (which make every rule fire), expands the
//! placeholders of each `fix`, and parses the result with the real clap
//! definition: a missing subcommand, a missing flag, a missing required
//! argument or a value the flag refuses is a failure.

use crate::cli::Cli;
use chrono::DateTime;
use clap::Parser;
use linear_core::audit::{audit_scoped, AuditConfig, AuditOptions, Snapshot};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Every fix the audit prints for the golden inputs, by the rule that printed it.
fn fixes_by_rule() -> BTreeMap<String, BTreeSet<String>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wasm/tests/golden/audit");
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut cases = 0;
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let case: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        // The cases that are meant to be refused have no report to read.
        if case["expected"]["ok"] != true {
            continue;
        }
        let input = &case["input"];
        let snapshot: Snapshot = serde_json::from_value(input["snapshot"].clone()).unwrap();
        let config: AuditConfig = if input["config"].is_null() {
            AuditConfig::default()
        } else {
            serde_json::from_value(input["config"].clone()).unwrap()
        };
        let options: AuditOptions = if input["options"].is_null() {
            AuditOptions::default()
        } else {
            serde_json::from_value(input["options"].clone()).unwrap()
        };
        let now = DateTime::parse_from_rfc3339(input["now"].as_str().unwrap())
            .unwrap()
            .to_utc();
        let report = audit_scoped(&snapshot, &config, &options, now).unwrap();
        for finding in report.findings {
            found
                .entry(finding.rule.as_str().to_owned())
                .or_default()
                .insert(finding.fix);
            cases += 1;
        }
    }
    assert!(cases > 0, "the golden cases produced no finding");
    found
}

/// A fix as the argument list `clap` would get from a shell: the placeholders
/// (`<state>`, `<onTrack|atRisk|offTrack>`, `kind=<kind>`) become a value.
fn arguments(fix: &str) -> Vec<String> {
    fix.split_whitespace().map(fill_placeholders).collect()
}

fn fill_placeholders(word: &str) -> String {
    let mut out = String::new();
    let mut rest = word;
    while let Some(open) = rest.find('<') {
        let Some(len) = rest[open..].find('>') else {
            break;
        };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..open + len];
        out.push_str(match name {
            "date" => "2026-01-01",
            // A choice: any of them is fine.
            _ => name.split('|').next().unwrap_or(name),
        });
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
}

/// What is wrong with a fix, or `None` when the CLI accepts it.
fn refusal(fix: &str) -> Option<String> {
    let args = arguments(fix);
    if args.first().map(String::as_str) != Some("linear") {
        return Some("it does not start with `linear`".to_owned());
    }
    Cli::try_parse_from(&args)
        .err()
        .map(|e| e.render().to_string().trim().to_owned())
}

#[test]
fn every_fix_the_audit_prints_is_a_command_the_cli_accepts() {
    let mut refused = Vec::new();
    for (rule, fixes) in fixes_by_rule() {
        for fix in fixes {
            if let Some(why) = refusal(&fix) {
                refused.push(format!("{rule}: {fix}\n    {why}"));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "the audit names commands the CLI does not have:\n{}",
        refused.join("\n")
    );
}

#[test]
fn the_golden_cases_reach_the_fix_of_every_rule() {
    // Otherwise a rule whose fix is wrong could hide behind a golden that never fires it.
    let seen = fixes_by_rule();
    for rule in [
        "project-state-vs-issues",
        "overdue",
        "issue-without-milestone",
        "project-without-lead",
        "stale-in-progress",
        "status-update-outdated",
        "template-sections",
        "source-attachment",
        "label-groups-exclusive",
        "not-updated-since",
    ] {
        assert!(seen.contains_key(rule), "no golden case has a {rule} fix");
    }
    // The fixes are spread over the commands the audit points at.
    let commands: BTreeSet<String> = seen
        .values()
        .flatten()
        .map(|fix| {
            let words: Vec<&str> = fix.split_whitespace().take(3).collect();
            words.join(" ")
        })
        .collect();
    for command in [
        "linear issue update",
        "linear project update",
        "linear project status-update",
        "linear milestone update",
    ] {
        assert!(commands.contains(command), "no fix runs `{command}`");
    }
}

#[test]
fn the_check_itself_notices_a_command_or_flag_that_does_not_exist() {
    // What the audit used to say.
    assert!(refusal("linear project status-update abc --body <text> -w w").is_some());
    assert!(refusal("linear issue update KK-1 --no-such-flag <x> -w w").is_some());
    assert!(refusal("linear issue nope KK-1 -w w").is_some());
    // A required flag left out.
    assert!(refusal("linear milestone update m1 --target-date <date> -w w").is_some());
    // A value the flag refuses.
    assert!(refusal("linear project status-update abc --health fine --body-file <file>").is_some());
    // And ones that exist.
    assert_eq!(
        refusal("linear issue update KK-1 --state <state> -w w"),
        None
    );
    assert_eq!(
        refusal(
            "linear project status-update abc --health <onTrack|atRisk|offTrack> --body-file <file> -w w"
        ),
        None
    );
}
