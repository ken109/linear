//! `cache` and `audit` against the real sandbox workspace. Ignored by default;
//! they need the sandbox credentials in the environment and the seeded data
//! (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_audit -- --ignored
//! ```
//!
//! Every check asserts a non-empty result, so an empty-but-successful
//! response cannot pass.

mod common;

use common::*;

/// A sandbox with the `sandbox` workspace configured, and the key to pass.
fn sandbox() -> (Sandbox, String) {
    let key = std::env::var("LINEAR_API_KEY_SANDBOX").expect("LINEAR_API_KEY_SANDBOX");
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "sandbox",
            "--url-key",
            "ken109-sandbox",
            "--team",
            "SAND",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    (sb, key)
}

/// Run `linear <args>` with the key and return the output.
fn run(sb: &Sandbox, key: &str, args: &[&str]) -> std::process::Output {
    let o = sb.run(args, None, &[("LINEAR_API_KEY_SANDBOX", key)]);
    assert!(
        !stdout(&o).contains(key) && !stderr(&o).contains(key),
        "the key must not be printed"
    );
    o
}

/// Run `linear <args> --json`, expect success and parse stdout.
fn json(sb: &Sandbox, key: &str, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let o = run(sb, key, &full);
    assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stdout(&o)))
}

fn entry(sb: &Sandbox) -> serde_json::Value {
    let path = sb.cache_dir().join("sandbox.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the cache entry")).unwrap()
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_refresh_caches_what_is_in_progress_and_survives_a_failed_one() {
    let (sb, key) = sandbox();
    let out = json(&sb, &key, &["cache", "refresh"]);
    assert_eq!(out["workspaces"][0]["status"], "ok", "{out}");
    assert_eq!(out["unreachable"], serde_json::json!([]));

    let e = entry(&sb);
    assert_eq!(e["data"]["viewer"]["isMe"], true);
    let issues = e["data"]["issues"].as_array().unwrap();
    assert!(
        !issues.is_empty(),
        "an issue assigned to the key's owner is In Progress"
    );
    for i in issues {
        assert_eq!(i["state"]["type"], "started");
        assert_eq!(i["assignee"]["isMe"], true);
    }
    assert!(!e["data"]["projects"].as_array().unwrap().is_empty());

    let rows = json(&sb, &key, &["cache", "show"]);
    assert_eq!(rows[0]["freshness"]["state"], "fresh");

    // A refresh with a key Linear rejects fails, and keeps what was there.
    let o = run(
        &sb,
        "lin_api_not_a_real_key",
        &["cache", "refresh", "--json"],
    );
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    let after = entry(&sb);
    assert_eq!(after["status"], "failed");
    assert_eq!(after["data"], e["data"]);
    assert!(after["failure"]["message"].is_string());
}

// ------------------------------------------------------------------- audit
//
// These rely on the data `scripts/seed-sandbox-audit.py` plants (names start
// with `audit-seed`), and enable every validator rule with `stale_days = 1`.

/// The sandbox with the rules of the audit tests switched on.
fn audited() -> (Sandbox, String) {
    let (sb, key) = sandbox();
    std::fs::write(
        sb.config_dir().join("workspaces.toml"),
        r#"default = "sandbox"

[workspaces.sandbox]
url_key = "ken109-sandbox"
default_team = "SAND"
rules = ["template-sections", "source-attachment", "label-groups-exclusive"]

[workspaces.sandbox.audit]
stale_days = 1
"#,
    )
    .unwrap();
    (sb, key)
}

/// `(rule, target title)` of every finding, with whether it is actionable.
fn found(report: &serde_json::Value) -> Vec<(String, String, bool)> {
    report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().unwrap().to_owned(),
                f["target"]["title"].as_str().unwrap().to_owned(),
                f["actionable"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn has(found: &[(String, String, bool)], rule: &str, title: &str) -> Option<bool> {
    found
        .iter()
        .find(|(r, t, _)| r == rule && t == title)
        .map(|(_, _, actionable)| *actionable)
}

fn finding<'a>(report: &'a serde_json::Value, rule: &str, title: &str) -> &'a serde_json::Value {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["rule"] == rule && f["target"]["title"] == title)
        .unwrap_or_else(|| panic!("no {rule} finding about {title:?}: {report}"))
}

fn identifier_of(report: &serde_json::Value, title: &str) -> String {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["target"]["title"] == title)
        .unwrap_or_else(|| panic!("no finding about {title:?}"))["target"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn the_planted_discrepancies_are_found() {
    let (sb, key) = audited();
    let report = json(&sb, &key, &["audit"]);
    assert_eq!(report["failedWorkspaces"], serde_json::json!([]));
    assert_eq!(report["unresolvedIssues"], serde_json::json!([]));
    let f = found(&report);
    assert!(!f.is_empty());

    // Linear's own consistency.
    let late = "audit-seed late issue";
    let project = "audit-seed overdue project";
    assert_eq!(
        has(
            &f,
            "project-state-vs-issues",
            "audit-seed completed project"
        ),
        Some(true),
        "a project the viewer leads: actionable"
    );
    assert_eq!(
        has(&f, "overdue", project),
        Some(false),
        "no lead: nobody owns it"
    );
    assert_eq!(has(&f, "overdue", "audit-seed milestone"), Some(false));
    assert_eq!(
        has(&f, "overdue", late),
        Some(true),
        "assigned to the viewer"
    );
    assert_eq!(has(&f, "issue-without-milestone", late), Some(true));
    assert_eq!(has(&f, "project-without-lead", project), Some(false));

    // Staleness: the project has no status update at all.
    let outdated = finding(&report, "status-update-outdated", project);
    assert!(outdated["message"]
        .as_str()
        .unwrap()
        .contains("no status update"));

    // The validators on existing issues. The half-written issue fills "Background"
    // and leaves "Acceptance criteria" empty and "Out of scope" out.
    let half = "audit-seed half-written issue";
    let sections = finding(&report, "template-sections", half)["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(sections.contains("Sectioned Template"), "{sections}");
    assert!(
        sections.contains("\"Acceptance criteria\" is empty"),
        "{sections}"
    );
    assert!(
        sections.contains("\"Out of scope\" is missing"),
        "{sections}"
    );
    assert!(!sections.contains("Background"), "{sections}");
    let none = finding(&report, "template-sections", late)["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        none.contains("follows none of the issue templates"),
        "{none}"
    );
    assert_eq!(has(&f, "source-attachment", half), Some(true));

    // Every finding names a fix for this workspace and links to its target.
    for finding in report["findings"].as_array().unwrap() {
        assert!(finding["fix"].as_str().unwrap().ends_with("-w sandbox"));
        assert!(finding["target"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://linear.app/ken109-sandbox/"));
        assert_eq!(finding["workspace"], "sandbox");
    }

    // Closed issues are not audited unless named; the canceled one has no finding.
    assert!(has(&f, "template-sections", "audit-seed canceled issue").is_none());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_stale_issue_in_progress_is_found_once_it_is_old_enough() {
    let (sb, key) = audited();
    let report = json(&sb, &key, &["audit"]);
    let half = "audit-seed half-written issue";
    let id = identifier_of(&report, half);
    let view = json(&sb, &key, &["issue", "view", &id]);
    let updated: chrono::DateTime<chrono::Utc> =
        view["updatedAt"].as_str().unwrap().parse().unwrap();
    let age = chrono::Utc::now() - updated;
    if age < chrono::Duration::days(1) {
        // Linear sets updatedAt itself, so a seeded issue cannot be made old:
        // this check starts to bite a day after the issue was last touched.
        eprintln!("skipped: {id} was updated {age} ago, less than the 1 day limit");
        return;
    }
    assert_eq!(has(&found(&report), "stale-in-progress", half), Some(true));
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_narrowed_audit_names_issues_and_reports_what_it_could_not_find() {
    let (sb, key) = audited();
    let full = json(&sb, &key, &["audit"]);
    let late_id = identifier_of(&full, "audit-seed late issue");

    // Only that issue and the project it is in.
    let report = json(&sb, &key, &["audit", "--issues", &late_id]);
    let f = found(&report);
    assert!(has(&f, "overdue", "audit-seed late issue").is_some());
    assert!(has(&f, "project-without-lead", "audit-seed overdue project").is_some());
    assert!(
        f.iter()
            .all(|(_, title, _)| title.starts_with("audit-seed")),
        "{f:?}"
    );
    assert!(has(&f, "template-sections", "audit-seed half-written issue").is_none());

    // A closed issue that is not in the open set is fetched by name, and an
    // unknown one is reported as unresolved, not as clean.
    let canceled = json(
        &sb,
        &key,
        &["issue", "list", "--state-type", "canceled", "--all"],
    );
    let canceled_id = canceled
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "audit-seed canceled issue")
        .expect("the seeded canceled issue")["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let report = json(
        &sb,
        &key,
        &["audit", "--issues", &format!("{canceled_id},NOPE-9999")],
    );
    assert_eq!(report["unresolvedIssues"], serde_json::json!(["NOPE-9999"]));
    assert!(
        has(
            &found(&report),
            "template-sections",
            "audit-seed canceled issue"
        )
        .is_some(),
        "an issue the caller named is checked even though it is closed"
    );
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn since_and_fail_on_decide_the_exit_code() {
    let (sb, key) = audited();
    let full = json(&sb, &key, &["audit"]);
    let late_id = identifier_of(&full, "audit-seed late issue");

    // Updated before the far future: reported, and always actionable.
    let args = [
        "audit",
        "--issues",
        late_id.as_str(),
        "--since",
        "2999-01-01T00:00:00Z",
    ];
    let report = json(&sb, &key, &args);
    let f = finding(&report, "not-updated-since", "audit-seed late issue");
    assert_eq!(f["actionable"], true);

    // Without --fail-on a finding is not a failure; with it, exit code 6.
    let o = run(&sb, &key, &args);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let mut failing = args.to_vec();
    failing.extend(["--fail-on", "actionable", "--json"]);
    let o = run(&sb, &key, &failing);
    assert_eq!(code(&o), 6, "{}", stdout(&o));
    assert!(stderr(&o).contains("audit_findings"), "{}", stderr(&o));
    // The findings are still printed.
    assert!(stdout(&o).contains("not-updated-since"));

    // --since without --issues is a usage error, before anything is fetched.
    let o = run(
        &sb,
        "not-even-used",
        &["audit", "--since", "2999-01-01T00:00:00Z"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_workspace_that_cannot_be_audited_is_a_failure_not_a_clean_result() {
    let (sb, _) = audited();
    let o = run(&sb, "lin_api_not_a_real_key", &["audit", "--json"]);
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    let out: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(out["findings"], serde_json::json!([]));
    assert_eq!(out["failedWorkspaces"][0]["workspace"], "sandbox");
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn the_cached_audit_is_the_audit_of_the_last_refresh() {
    let (sb, key) = audited();
    // Nothing cached yet: unknown, which must not look like "no findings".
    let o = run(&sb, &key, &["audit", "--cached", "--json"]);
    assert_eq!(code(&o), 1, "{}", stdout(&o));
    assert!(stderr(&o).contains("nothing cached"), "{}", stderr(&o));

    assert_eq!(
        json(&sb, &key, &["cache", "refresh"])["unreachable"],
        serde_json::json!([])
    );
    let live = json(&sb, &key, &["audit"]);
    let cached = json(&sb, &key, &["audit", "--cached"]);
    assert_eq!(cached["findings"], live["findings"]);
    assert_eq!(cached["cached"][0]["workspace"], "sandbox");
    assert!(!cached["findings"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn canceled_and_duplicate_issues_carry_their_canceled_at() {
    let (sb, key) = audited();
    let all = json(&sb, &key, &["issue", "list", "--all"]);
    let find = |title: &str| {
        all.as_array()
            .unwrap()
            .iter()
            .find(|i| i["title"] == title)
            .unwrap_or_else(|| panic!("no issue {title:?}"))
            .clone()
    };
    let canceled = find("audit-seed canceled issue");
    assert_eq!(canceled["state"]["type"], "canceled");
    assert!(canceled["canceledAt"].is_string(), "{canceled}");

    let duplicate = find("audit-seed duplicate issue");
    assert_eq!(duplicate["state"]["type"], "duplicate");
    assert!(duplicate["canceledAt"].is_string(), "{duplicate}");

    // And the description is there for the template rule to read.
    let half = find("audit-seed half-written issue");
    assert!(half["description"]
        .as_str()
        .unwrap()
        .contains("## Background"));
}
