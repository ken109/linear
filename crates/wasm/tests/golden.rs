//! Golden files: one input and the answer the native core gives, in
//! `tests/golden/<function>/<case>.json`. The same files are run through the
//! wasm in Node (`packages/linear-wasm/test/golden.test.ts`), so the native core
//! (what the CLI runs) and the wasm are held to the same answers. See
//! `tests/golden/README.md`.
//!
//! `UPDATE_GOLDEN=1 cargo test -p linear-wasm --test golden` rewrites the
//! `expected` of every case from the native results; read the diff before
//! committing it.

use chrono::DateTime;
use linear_core::audit::{audit_scoped, AuditConfig, AuditOptions, Snapshot};
use linear_wasm::api;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const KINDS: [&str; 6] = ["audit", "diff", "refresh", "webhook", "parse", "build"];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn core_fixture(name: &str) -> String {
    fs::read_to_string(format!(
        "{}/../core/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// Epoch milliseconds of an RFC 3339 time, as the boundary takes them.
fn ms(now: &Value) -> f64 {
    let text = now.as_str().expect("now is a string");
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .timestamp_millis() as f64
}

/// A JSON input as the boundary takes it: compact text, or empty for `null`.
fn text(v: &Value) -> String {
    if v.is_null() {
        String::new()
    } else {
        v.to_string()
    }
}

/// What the boundary answers for a case's input, parsed.
fn answer(kind: &str, input: &Value) -> Value {
    let out = match kind {
        "audit" => {
            let options = (!input["options"].is_null()).then(|| input["options"].to_string());
            api::audit(
                &input["snapshot"].to_string(),
                &text(&input["config"]),
                ms(&input["now"]),
                options.as_deref(),
            )
        }
        "diff" => api::diff(&text(&input["previous"]), &input["current"].to_string()),
        "refresh" => api::decide_refresh(
            &text(&input["meta"]),
            &input["event"].to_string(),
            ms(&input["now"]),
        ),
        "webhook" => api::verify_webhook(
            input["body"].as_str().unwrap(),
            input["signature"].as_str().unwrap(),
            input["secret"].as_str().unwrap(),
            ms(&input["now"]),
        ),
        "parse" => {
            let body = match input["fixture"].as_str() {
                Some(name) => core_fixture(name),
                None => input["body"].as_str().unwrap().to_owned(),
            };
            let mut meta = json!({ "status": input["status"] });
            for key in ["retryAfterSecs", "rateLimitResetMs"] {
                if !input[key].is_null() {
                    meta[key] = input[key].clone();
                }
            }
            api::parse_response(
                input["operation"].as_str().unwrap(),
                &meta.to_string(),
                &body,
                ms(&input["now"]),
            )
        }
        "build" => api::build_request(
            input["operation"].as_str().unwrap(),
            &text(&input["params"]),
        ),
        other => panic!("unknown kind {other}"),
    };
    let answer: Value =
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{kind}: not JSON ({e}): {out}"));
    normalized(kind, answer)
}

/// What a golden holds of an answer. A built request is kept as its URL, its
/// variables and the first line of its query: the rest of the query text
/// changes whenever a fragment gains a field, and says nothing about whether
/// the wasm and the native core agree.
fn normalized(kind: &str, mut answer: Value) -> Value {
    if kind == "build" && answer["ok"] == true {
        let body = answer["data"]["body"].as_str().expect("body is text");
        let mut body: Value = serde_json::from_str(body).expect("body is JSON");
        let first_line = body["query"]
            .as_str()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        body["query"] = json!(first_line);
        answer["data"]["body"] = body;
    }
    answer
}

fn cases() -> Vec<(String, PathBuf, Value)> {
    let mut all = Vec::new();
    for kind in KINDS {
        let dir = golden_dir().join(kind);
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let case: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            all.push((kind.to_owned(), path, case));
        }
    }
    all
}

fn label(kind: &str, path: &Path) -> String {
    format!("{kind}/{}", path.file_stem().unwrap().to_string_lossy())
}

#[test]
fn the_native_core_gives_the_golden_answers() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    for (kind, path, mut case) in cases() {
        let got = answer(&kind, &case["input"]);
        if update {
            case["expected"] = got;
            let mut text = serde_json::to_string_pretty(&case).unwrap();
            text.push('\n');
            fs::write(&path, text).unwrap();
        } else if case["expected"] != got {
            failures.push(format!(
                "{}:\n  expected {}\n  got      {}",
                label(&kind, &path),
                case["expected"],
                got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} golden case(s) differ (UPDATE_GOLDEN=1 rewrites them):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_case_has_a_description_and_an_answer() {
    let all = cases();
    for kind in KINDS {
        assert!(
            all.iter().filter(|(k, _, _)| k == kind).count() >= 5,
            "{kind}: too few cases"
        );
    }
    for (kind, path, case) in &all {
        let name = label(kind, path);
        assert!(
            case["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name}: no description"
        );
        assert!(
            case["expected"]["ok"].is_boolean(),
            "{name}: no answer; run UPDATE_GOLDEN=1"
        );
    }
}

/// The audit goldens are what `audit_scoped` returns: the function the CLI's
/// `linear audit` calls and prints the findings of. Run it directly, without
/// the boundary's JSON handling, and compare.
#[test]
fn audit_goldens_are_what_the_core_returns_directly() {
    let mut checked = 0;
    for (kind, path, case) in cases() {
        if kind != "audit" || case["expected"]["ok"] != true {
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
        let now = DateTime::from_timestamp_millis(ms(&input["now"]) as i64).unwrap();
        let report = audit_scoped(&snapshot, &config, &options, now).unwrap();
        assert_eq!(
            case["expected"]["data"],
            serde_json::to_value(report).unwrap(),
            "{}",
            label(&kind, &path)
        );
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} audit goldens were checked");
}

/// The rules the audit goldens are meant to exercise really fire in them, so a
/// golden does not pass by being empty.
#[test]
fn the_audit_goldens_exercise_every_rule() {
    let mut seen = std::collections::BTreeSet::new();
    for (kind, _, case) in cases() {
        if kind != "audit" {
            continue;
        }
        for f in case["expected"]["data"]["findings"]
            .as_array()
            .into_iter()
            .flatten()
        {
            seen.insert(f["rule"].as_str().unwrap().to_owned());
        }
    }
    for rule in [
        "project-state-vs-issues",
        "overdue",
        "issue-without-milestone",
        "project-without-lead",
        "stale-in-progress",
        "status-update-outdated",
        "pr-merged-issue-open",
        "pr-open-too-long",
        "template-sections",
        "source-attachment",
        "label-groups-exclusive",
        "not-updated-since",
    ] {
        assert!(seen.contains(rule), "no audit golden has a {rule} finding");
    }
}
