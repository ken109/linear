//! `linear brief` and `linear brief --session` against a mock Linear server.

mod common;
mod read_support;

use common::*;
use read_support::*;
use std::time::{Duration, Instant};

/// The clock of the golden tests in core: 2026-10-20 12:00 UTC, shown in JST. Independent of the
/// machine's clock and `TZ`.
const NOW: &str = "2026-10-20T21:00:00+09:00";

fn brief(sb: &Sandbox, mock: &Mock, args: &[&str]) -> std::process::Output {
    sb.run(
        args,
        Some(mock),
        &[("LINEAR_API_KEY_EXAMPLE", KEY), ("LINEAR_NOW", NOW)],
    )
}

#[test]
fn the_brief_lists_the_projects_in_progress_or_with_an_update() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    // 100000 days: nothing is stale.
    let o = brief(&sb, &mock, &["brief", "--stale-days", "100000"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    for want in [
        "## Linear project status (example",
        "**Ship the importer** (Platform) `aaaaaaaaaaaa`",
        "Stage: parser done, writer in progress",
        "Milestones 1/3 done · next: Writer (2026-10-20, 40%)",
        "**Tidy the backlog** `bbbbbbbbbbbb`",
        "No status update",
        "**Old research**",
        "**Planned but written about**",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    // Backlog without an update and a completed project are left out.
    assert!(!out.contains("Not started yet") && !out.contains("Already finished"));
    assert!(!out.contains("stale**"), "{out}");

    // One request, filtered to unfinished projects on Linear's side.
    assert_eq!(mock.requests().len(), 1);
    let req = request(&mock, 0);
    assert_eq!(req["operationName"], "ProjectList");
    assert_eq!(
        req["variables"]["filter"]["status"]["type"]["nin"],
        serde_json::json!(["completed", "canceled"])
    );
}

#[test]
fn a_threshold_of_zero_marks_every_update_stale() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = brief(&sb, &mock, &["brief", "--stale-days", "0"]);
    let out = stdout(&o);
    assert_eq!(out.matches("**stale**").count(), 3, "{out}");
    assert!(out.contains("0 days old or more is stale"), "{out}");
}

#[test]
fn json_has_the_workspace_the_threshold_and_the_projects() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = brief(&sb, &mock, &["brief", "--json", "--stale-days", "14"]);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["staleDays"], 14);
    let projects = v["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 4);
    let importer = projects
        .iter()
        .find(|p| p["slugId"] == "aaaaaaaaaaaa")
        .unwrap();
    assert_eq!(importer["health"], "onTrack");
    assert_eq!(importer["initiative"], "Platform");
    assert_eq!(importer["milestones"]["done"], 1);
    assert_eq!(importer["update"]["preview"].as_array().unwrap().len(), 3);
    let none = projects
        .iter()
        .find(|p| p["slugId"] == "bbbbbbbbbbbb")
        .unwrap();
    assert!(none["update"].is_null());
}

#[test]
fn quiet_prints_the_slug_ids() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = brief(&sb, &mock, &["brief", "--quiet"]);
    assert_eq!(
        stdout(&o),
        "eeeeeeeeeeee\naaaaaaaaaaaa\ncccccccccccc\nbbbbbbbbbbbb\n"
    );
}

#[test]
fn no_projects_says_so_and_a_failure_is_an_error() {
    let sb = workspace();
    let empty =
        r#"{"data":{"projects":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mock = Mock::start(vec![ok(empty)]);
    let o = brief(&sb, &mock, &["brief"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).contains("No project to show"), "{}", stdout(&o));

    let mock = Mock::start(vec![ok(&fixture("error_unauthenticated"))]);
    let o = brief(&sb, &mock, &["brief"]);
    assert_ne!(code(&o), 0);
    assert!(stdout(&o).is_empty());
    assert!(stderr(&o).contains("error:"), "{}", stderr(&o));
    assert_no_leak(&o);
}

#[test]
fn session_prints_the_same_markdown() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = brief(&sb, &mock, &["brief", "--session"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("**Ship the importer**"),
        "{}",
        stdout(&o)
    );
    assert_eq!(stderr(&o), "");
}

#[test]
fn the_default_threshold_is_14_days_in_the_clocks_calendar() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let out = stdout(&brief(&sb, &mock, &["brief"]));
    // Written the 6th (14 days) and 1 Aug (80 days) are stale; the 7th (13 days) is not.
    for want in [
        "2026-10-06 (14 days ago, **stale**) on track",
        "2026-10-07 (13 days ago) off track",
        "2026-08-01 (80 days ago, **stale**) at risk",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}

#[test]
fn a_clock_that_is_not_rfc_3339_is_a_usage_error() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = sb.run(
        &["brief"],
        Some(&mock),
        &[("LINEAR_API_KEY_EXAMPLE", KEY), ("LINEAR_NOW", "yesterday")],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("LINEAR_NOW"), "{}", stderr(&o));
}

#[test]
fn session_in_ci_prints_nothing_and_does_not_ask_linear() {
    let sb = workspace();
    let mock = Mock::start(vec![ok(&fixture("brief_projects"))]);
    let o = sb.run(
        &["brief", "--session"],
        Some(&mock),
        &[("LINEAR_API_KEY_EXAMPLE", KEY), ("CI", "true")],
    );
    assert_eq!(code(&o), 0);
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
    assert!(mock.requests().is_empty());
}

#[test]
fn session_is_silent_and_succeeds_when_anything_fails() {
    let sb = workspace();
    // A bad key, a Linear error, no reply queued (HTTP 500), a nonsense body.
    for reply in [
        Some(ok(&fixture("error_unauthenticated"))),
        Some(Reply {
            status: 500,
            body: "boom".to_owned(),
        }),
        Some(ok("not json")),
        None,
    ] {
        let mock = Mock::start(reply.into_iter().collect());
        let o = brief(&sb, &mock, &["brief", "--session"]);
        assert_eq!(code(&o), 0);
        assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
    }

    // No credentials at all, and no workspaces at all.
    let o = sb.run(&["brief", "--session"], None, &[]);
    assert_eq!(code(&o), 0);
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
    let bare = Sandbox::new();
    let o = bare.run(&["brief", "--session"], None, &[]);
    assert_eq!(code(&o), 0);
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
    let o = sb.run(&["--workspace", "nope", "brief", "--session"], None, &[]);
    assert_eq!(code(&o), 0);
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
}

#[test]
fn session_gives_up_after_four_seconds_when_linear_does_not_answer() {
    let sb = workspace();
    // A server that accepts the connection and never answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let held = std::thread::spawn(move || {
        let mut conns = Vec::new();
        while let Ok((c, _)) = listener.accept() {
            conns.push(c);
            if conns.len() >= 3 {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(8));
    });
    let started = Instant::now();
    let o = sb.run(
        &["brief", "--session"],
        None,
        &[("LINEAR_API_KEY_EXAMPLE", KEY), ("LINEAR_API_URL", &url)],
    );
    let took = started.elapsed();
    assert_eq!(code(&o), 0);
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
    assert!(
        took >= Duration::from_millis(3900) && took < Duration::from_secs(7),
        "took {took:?}"
    );
    drop(held);
}
