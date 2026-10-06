//! Read commands against the real sandbox workspace. Ignored by default; they
//! need the sandbox credentials in the environment and the seeded data
//! (see `tests/fixtures/README.md`):
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_read -- --ignored
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

/// Run `linear <args> --json` and parse stdout.
fn json(sb: &Sandbox, key: &str, args: &[&str]) -> serde_json::Value {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--json");
    let o = sb.run(&full, None, &[("LINEAR_API_KEY_SANDBOX", key)]);
    assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
    assert!(!stdout(&o).contains(key) && !stderr(&o).contains(key));
    serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stdout(&o)))
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn my_started_issues_are_listed() {
    let (sb, key) = sandbox();
    let v = json(
        &sb,
        &key,
        &[
            "issue",
            "list",
            "--assignee",
            "me",
            "--state-type",
            "started",
        ],
    );
    let rows = v.as_array().unwrap();
    assert!(
        !rows.is_empty(),
        "no started issue is assigned to the key's owner"
    );
    for r in rows {
        assert_eq!(r["state"]["type"], "started");
        assert_eq!(r["assignee"]["isMe"], true);
        assert_eq!(r["workspace"], "sandbox");
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn an_issue_is_found_by_its_origin_url_and_viewed() {
    let (sb, key) = sandbox();
    let v = json(
        &sb,
        &key,
        &[
            "issue",
            "list",
            "--source-url",
            "https://example.com/source/1",
        ],
    );
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{v}");
    let identifier = rows[0]["identifier"].as_str().unwrap().to_owned();
    assert_eq!(rows[0]["sourceUrl"], "https://example.com/source/1");

    let view = json(&sb, &key, &["issue", "view", &identifier]);
    assert_eq!(view["identifier"], identifier.as_str());
    assert!(!view["description"].as_str().unwrap_or("").is_empty());
    assert!(!view["comments"]["nodes"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn paging_with_a_small_limit_and_with_all_agree() {
    let (sb, key) = sandbox();
    let all = json(&sb, &key, &["issue", "list", "--all"]);
    let total = all.as_array().unwrap().len();
    assert!(total >= 3, "the sandbox is seeded with at least 3 issues");

    let o = sb.run(
        &["issue", "list", "--limit", "2", "--quiet"],
        None,
        &[("LINEAR_API_KEY_SANDBOX", &key)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).lines().count(), 2);
    assert!(stderr(&o).contains("more exist"), "{}", stderr(&o));
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn projects_show_the_lead_and_open_excludes_finished_ones() {
    let (sb, key) = sandbox();
    let all = json(&sb, &key, &["project", "list"]);
    let open = json(&sb, &key, &["project", "list", "--open"]);
    let all = all.as_array().unwrap();
    let open = open.as_array().unwrap();
    assert!(!open.is_empty());
    assert!(open.len() < all.len(), "a completed project is seeded");
    for p in open {
        let t = p["status"]["type"].as_str().unwrap();
        assert!(t != "completed" && t != "canceled", "{t}");
    }
    // `ken109-linear projects` has no lead; here it is present and `me` finds it.
    let mine = json(&sb, &key, &["project", "list", "--lead", "me"]);
    assert!(!mine.as_array().unwrap().is_empty());
    assert_eq!(mine[0]["lead"]["isMe"], true);
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn the_latest_status_update_is_the_newest_one() {
    let (sb, key) = sandbox();
    let v = json(&sb, &key, &["project", "view", "Fixture Project"]);
    let updates = v["projectUpdates"]["nodes"].as_array().unwrap();
    assert!(
        updates.len() >= 2,
        "the sandbox project has at least two status updates"
    );
    // `lastUpdate` (used by `project list`) is the newest of all updates, and
    // the update list is newest first.
    let newest = updates
        .iter()
        .map(|u| u["createdAt"].as_str().unwrap())
        .max()
        .unwrap();
    assert_eq!(v["lastUpdate"]["createdAt"], newest);
    assert_eq!(updates[0]["createdAt"], newest);
    for pair in updates.windows(2) {
        assert!(pair[0]["createdAt"].as_str() >= pair[1]["createdAt"].as_str());
    }
    assert!(!v["description"].as_str().unwrap_or("").is_empty());
    assert!(v["issueCounts"]["completed"].as_u64().unwrap() >= 1);
    assert!(!v["projectMilestones"]["nodes"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn milestones_and_their_issues_are_read() {
    let (sb, key) = sandbox();
    let list = json(
        &sb,
        &key,
        &["milestone", "list", "--project", "Fixture Project"],
    );
    let rows = list.as_array().unwrap();
    assert!(!rows.is_empty());
    let name = rows[0]["name"].as_str().unwrap().to_owned();
    assert!(rows[0]["targetDate"].is_string());

    let view = json(
        &sb,
        &key,
        &["milestone", "view", &name, "--project", "Fixture Project"],
    );
    assert_eq!(view["name"], name.as_str());
    assert!(!view["issues"]["nodes"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn templates_labels_teams_and_users_are_read() {
    let (sb, key) = sandbox();

    // The section definitions come from Linear: the seeded template has real headings.
    let sections = json(&sb, &key, &["template", "view", "Sectioned Template"]);
    assert_eq!(
        sections["sections"],
        serde_json::json!(["Background", "Acceptance criteria", "Out of scope"])
    );
    let o = sb.run(
        &["template", "skeleton", "Sectioned Template"],
        None,
        &[("LINEAR_API_KEY_SANDBOX", &key)],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "## Background\n\n## Acceptance criteria\n\n## Out of scope\n"
    );

    let labels = json(&sb, &key, &["label", "list"]);
    let labels = labels.as_array().unwrap();
    let child = labels.iter().find(|l| l["name"] == "api").unwrap();
    assert_eq!(child["parent"]["name"], "area");
    assert!(labels
        .iter()
        .any(|l| l["name"] == "area" && l["isGroup"] == true));

    let teams = json(&sb, &key, &["team", "list"]);
    assert!(teams.as_array().unwrap().iter().any(|t| t["key"] == "SAND"));

    let me = json(&sb, &key, &["user", "view", "me"]);
    assert_eq!(me["isMe"], true);
    assert!(!me["email"].as_str().unwrap_or("").is_empty());
    assert!(json(&sb, &key, &["user", "list"])
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["isMe"] == true));
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn initiatives_list_without_error_even_where_the_plan_has_none() {
    // The free plan disables initiatives, so the sandbox returns an empty
    // list; this only checks that the query is accepted.
    let (sb, key) = sandbox();
    assert!(json(&sb, &key, &["initiative", "list"]).is_array());
}
