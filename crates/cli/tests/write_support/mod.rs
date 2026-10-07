//! Helpers for the write-command tests: a mock Linear that answers by
//! operation name (writes make several requests, and their order is part of
//! what is under test), and builders for the responses a write reads.
#![allow(dead_code)]

use crate::common::*;
use crate::read_support::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

/// What the mock saw: the operation name and the variables.
#[derive(Debug, Clone)]
pub struct Call {
    pub op: String,
    pub variables: Value,
    /// The document is a mutation (judged from the document, not from its name).
    pub mutation: bool,
}

/// A mock Linear that answers each request from a queue keyed by its
/// `operationName`. When a queue runs down to its last reply, that reply
/// repeats. An operation with no route is answered with a GraphQL error
/// naming it, so a test that makes an unexpected request fails loudly.
pub struct Routed {
    pub url: String,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl Routed {
    pub fn start(routes: Vec<(&str, Vec<Reply>)>) -> Routed {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let url = format!(
            "http://{}/graphql",
            server.server_addr().to_ip().expect("ip")
        );
        let calls = Arc::new(Mutex::new(Vec::<Call>::new()));
        let log = Arc::clone(&calls);
        let mut routes: HashMap<String, Vec<Reply>> = routes
            .into_iter()
            .map(|(op, replies)| (op.to_owned(), replies))
            .collect();
        thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
                let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let op = parsed["operationName"].as_str().unwrap_or("").to_owned();
                log.lock().unwrap().push(Call {
                    op: op.clone(),
                    variables: parsed["variables"].clone(),
                    mutation: parsed["query"]
                        .as_str()
                        .is_some_and(|q| q.trim_start().starts_with("mutation")),
                });
                let reply = match routes.get_mut(&op) {
                    Some(queue) if queue.len() > 1 => queue.remove(0),
                    Some(queue) if queue.len() == 1 => Reply {
                        status: queue[0].status,
                        body: queue[0].body.clone(),
                    },
                    _ => Reply {
                        status: 200,
                        body: json!({"errors": [{"message": format!("mock: no route for {op}")}]})
                            .to_string(),
                    },
                };
                let response = tiny_http::Response::from_string(reply.body)
                    .with_status_code(reply.status)
                    .with_header(
                        tiny_http::Header::from_bytes("content-type", "application/json").unwrap(),
                    );
                let _ = req.respond(response);
            }
        });
        Routed { url, calls }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// The operation names seen, in order.
    pub fn ops(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.op).collect()
    }

    /// The variables of every call to `op`.
    pub fn of(&self, op: &str) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|c| c.op == op)
            .map(|c| c.variables)
            .collect()
    }

    /// Nothing was sent that changes anything: no request whose document is a mutation.
    /// (Judged from the document, so a mutation added later is caught without a list.)
    pub fn assert_read_only(&self) {
        let sent: Vec<String> = self
            .calls()
            .into_iter()
            .filter(|c| c.mutation)
            .map(|c| c.op)
            .collect();
        assert!(sent.is_empty(), "a mutation was sent: {sent:?}");
    }
}

/// What a `--dry-run --json` printed, after checking what every dry run promises: it
/// succeeded, no mutation reached the mock, and the plan has its stable shape.
pub fn plan(o: &std::process::Output, mock: &Routed) -> Value {
    assert_eq!(code(o), 0, "{}", stderr(o));
    mock.assert_read_only();
    let v = stdout_json(o);
    assert_eq!(v["dryRun"], true, "{v}");
    for key in [
        "workspace",
        "command",
        "target",
        "changed",
        "mutations",
        "rollback",
        "notes",
        "reason",
    ] {
        assert!(v.get(key).is_some(), "the plan has no {key}: {v}");
    }
    for key in ["kind", "name", "id", "new"] {
        assert!(
            v["target"].get(key).is_some(),
            "the target has no {key}: {v}"
        );
    }
    v
}

/// The operation names of a plan's mutations, in order.
pub fn planned(plan: &Value) -> Vec<String> {
    plan["mutations"]
        .as_array()
        .expect("mutations is a list")
        .iter()
        .map(|m| m["operation"].as_str().expect("operation").to_owned())
        .collect()
}

/// Run `linear <args>` against a routed mock with the test key.
pub fn run(sb: &Sandbox, mock: &Routed, args: &[&str]) -> std::process::Output {
    sb.run_url(args, &mock.url, &[("LINEAR_API_KEY_EXAMPLE", KEY)])
}

impl Sandbox {
    /// Like `run`, with the API pointed at `url`.
    pub fn run_url(
        &self,
        args: &[&str],
        url: &str,
        extra_env: &[(&str, &str)],
    ) -> std::process::Output {
        let mut env: Vec<(&str, &str)> = vec![("LINEAR_API_URL", url)];
        env.extend_from_slice(extra_env);
        self.run(args, None, &env)
    }
}

/// Like `workspace_with_rules`, with one more line (`key = value`) in the workspace's table.
pub fn workspace_with_setting(rules: &[&str], line: &str) -> Sandbox {
    let sb = workspace_with_rules(rules);
    let path = sb.config_dir().join("workspaces.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(line);
    text.push('\n');
    std::fs::write(&path, text).unwrap();
    sb
}

/// Like `workspace_with_rules`, with `ownership = "lenient"` (a team that works on each other's issues).
pub fn lenient_workspace_with_rules(rules: &[&str]) -> Sandbox {
    workspace_with_setting(rules, "ownership = \"lenient\"")
}

/// Like `workspace_with_rules`, with `source_kinds` set for the `source-attachment` rule.
pub fn workspace_with_source_kinds(kinds: &[&str]) -> Sandbox {
    let sb = workspace_with_rules(&["source-attachment"]);
    let path = sb.config_dir().join("workspaces.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    let list: Vec<String> = kinds.iter().map(|k| format!("\"{k}\"")).collect();
    text.push_str(&format!("source_kinds = [{}]\n", list.join(", ")));
    std::fs::write(&path, text).unwrap();
    sb
}

/// A sandbox with workspace `example` and the given validator rules enabled.
pub fn workspace_with_rules(rules: &[&str]) -> Sandbox {
    let sb = workspace();
    if !rules.is_empty() {
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        let list: Vec<String> = rules.iter().map(|r| format!("\"{r}\"")).collect();
        text.push_str(&format!("\nrules = [{}]\n", list.join(", ")));
        std::fs::write(&path, text).unwrap();
    }
    sb
}

// ---------------------------------------------------------------- responses

pub const ALICE: &str = "00000000-0000-4000-8000-000000000001";
pub const BOT: &str = "00000000-0000-4000-8000-000000000040";
pub const PROJECT: &str = "00000000-0000-4000-8000-000000000006";
pub const OTHER_PROJECT: &str = "00000000-0000-4000-8000-000000000060";

pub fn data(v: Value) -> Reply {
    ok(&json!({ "data": v }).to_string())
}

pub fn graphql_error(message: &str) -> Reply {
    ok(&json!({ "errors": [{ "message": message }] }).to_string())
}

fn fixture_data(name: &str) -> Value {
    serde_json::from_str::<Value>(&fixture(name)).unwrap()["data"].clone()
}

pub fn whoami() -> Reply {
    ok(WHOAMI_OK)
}

pub fn teams() -> Reply {
    data(fixture_data("teams"))
}

pub fn project_refs() -> Reply {
    ok(PROJECT_REFS)
}

/// Two projects, so an issue can be moved from one to the other.
pub fn two_project_refs() -> Reply {
    let mut v: Value = serde_json::from_str(PROJECT_REFS).unwrap();
    let nodes = v["data"]["projects"]["nodes"].as_array_mut().unwrap();
    nodes.push(json!({
        "id": OTHER_PROJECT, "slugId": "bbbbbbbbbbbb", "name": "Other Project",
        "url": "https://linear.app/example/project/other-project-bbbbbbbbbbbb"
    }));
    ok(&v.to_string())
}

pub fn templates() -> Reply {
    data(fixture_data("templates_sections"))
}

pub fn labels() -> Reply {
    data(fixture_data("labels"))
}

/// The `area` group with two children (a group Linear lets you pick one of).
pub fn conflicting_labels() -> Reply {
    let mut v = fixture_data("labels");
    let nodes = v["issueLabels"]["nodes"].as_array_mut().unwrap();
    let mut second = nodes[0].clone();
    second["id"] = json!("00000000-0000-4000-8000-000000000070");
    second["name"] = json!("ui");
    nodes.push(second);
    data(v)
}

pub fn users() -> Reply {
    data(fixture_data("users"))
}

pub fn no_issue_with_source() -> Reply {
    data(fixture_data("attachments_for_url_none"))
}

pub fn issue_with_source() -> Reply {
    data(fixture_data("attachments_for_url"))
}

/// The issue that carries the source, whose attachment stores this title,
/// subtitle and metadata.
pub fn issue_with_source_stored(title: &str, subtitle: Option<&str>, metadata: Value) -> Reply {
    let mut v = fixture_data("attachments_for_url");
    let node = &mut v["attachmentsForURL"]["nodes"][0];
    node["title"] = json!(title);
    node["subtitle"] = json!(subtitle);
    node["metadata"] = metadata;
    data(v)
}

/// A project with the given lead and one milestone.
pub fn ownership(project_id: &str, lead: Option<&str>) -> Reply {
    let view = fixture_data("issue_write_view");
    let mut project = view["write"]["project"].clone();
    project["id"] = json!(project_id);
    project["lead"] = match lead {
        Some(id) => {
            let mut u = project["lead"].clone();
            u["id"] = json!(id);
            u
        }
        None => Value::Null,
    };
    data(json!({ "project": project }))
}

/// What `issue_write_view` returns for `identifier`.
pub struct View(pub Value);

pub fn view(identifier: &str) -> View {
    let mut v = fixture_data("issue_write_view");
    v["issue"]["identifier"] = json!(identifier);
    v["issue"]["id"] = json!(format!("id-{identifier}"));
    View(v)
}

/// What `issue(id:)` answers for the issue `identifier` (the lookup behind `--parent`).
pub fn issue_by_id(identifier: &str) -> Reply {
    let mut v = fixture_data("issue");
    v["issue"]["identifier"] = json!(identifier);
    v["issue"]["id"] = json!(format!("id-{identifier}"));
    data(v)
}

/// What `issue(id:)` answers when Linear has no such issue.
pub fn no_such_issue() -> Reply {
    graphql_error("Entity not found: Issue")
}

/// The two cycles of the `cycles` fixture: #41 (2026-10-05T15:00Z ..) and #42.
pub const CYCLE_41: &str = "00000000-0000-4000-8000-000000000201";
pub const CYCLE_42: &str = "00000000-0000-4000-8000-000000000202";
/// A meeting day whose next day is in cycle #41.
pub const MEETING: &str = "2026-10-05";

/// What `cycles` returns: cycles #41 and #42 of the fixture team.
pub fn cycles() -> Reply {
    data(fixture_data("cycles"))
}

impl View {
    /// In the cycle with this id (one of the fixture's), or in none.
    pub fn in_cycle(mut self, id: Option<&str>) -> View {
        self.0["write"]["cycle"] = match id {
            Some(id) => {
                let all = fixture_data("cycles");
                all["cycles"]["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["id"] == id)
                    .unwrap_or_else(|| panic!("no fixture cycle {id}"))
                    .clone()
            }
            None => Value::Null,
        };
        self
    }

    /// Has this priority number (the fixture issue has 2, high).
    pub fn with_priority(mut self, priority: f64) -> View {
        self.0["issue"]["priority"] = json!(priority);
        self
    }

    /// Has this estimate, or none (the fixture issue has 3).
    pub fn with_estimate(mut self, estimate: Option<f64>) -> View {
        self.0["issue"]["estimate"] = json!(estimate);
        self
    }

    /// Is a sub-issue of `identifier` (whose id is `id-<identifier>`), or of nothing.
    pub fn with_parent(mut self, identifier: Option<&str>) -> View {
        self.0["issue"]["parent"] = match identifier {
            Some(i) => json!({
                "id": format!("id-{i}"), "identifier": i,
                "url": format!("https://linear.app/example/issue/{i}"),
            }),
            None => Value::Null,
        };
        self
    }

    pub fn order(mut self, sort: f64, priority: f64) -> View {
        self.0["issue"]["sortOrder"] = json!(sort);
        self.0["issue"]["prioritySortOrder"] = json!(priority);
        self
    }

    /// Assigned to `id`, or nobody.
    pub fn assigned_to(mut self, id: Option<&str>) -> View {
        self.0["issue"]["assignee"] = match id {
            Some(id) => {
                let mut u = self.0["issue"]["assignee"].clone();
                u["id"] = json!(id);
                u
            }
            None => Value::Null,
        };
        self
    }

    /// In the project with this id and lead (`None`: nobody leads it).
    pub fn in_project(mut self, project_id: &str, lead: Option<&str>) -> View {
        self.0["issue"]["project"]["id"] = json!(project_id);
        let owned = match ownership(project_id, lead).body.parse::<Value>() {
            Ok(v) => v["data"]["project"].clone(),
            Err(e) => panic!("{e}"),
        };
        self.0["write"]["project"] = owned;
        self
    }

    pub fn without_project(mut self) -> View {
        self.0["issue"]["project"] = Value::Null;
        self.0["write"]["project"] = Value::Null;
        self
    }

    pub fn reply(self) -> Reply {
        data(self.0)
    }
}

/// An `issueCreate` / `issueUpdate` payload carrying the fixture issue.
pub fn issue_payload(field: &str, identifier: &str) -> Reply {
    let mut issue = fixture_data("issue")["issue"].clone();
    issue["identifier"] = json!(identifier);
    issue["id"] = json!(format!("id-{identifier}"));
    data(json!({ field: { "success": true, "issue": issue } }))
}

pub fn attachment_ok() -> Reply {
    data(
        json!({ "attachmentCreate": { "success": true, "attachment": {
            "id": "00000000-0000-4000-8000-000000000080", "title": "Source", "subtitle": null,
            "url": "https://example.com/source/1", "sourceType": null, "metadata": {},
            "createdAt": "2026-10-06T13:34:35.885Z"
        }}}),
    )
}

pub fn delete_ok() -> Reply {
    data(json!({ "issueDelete": { "success": true } }))
}

/// The `n`th attachment of `fixtures/attachments_github.json` (see its README entry).
pub fn github_attachment(n: usize) -> Value {
    serde_json::from_str::<Value>(&fixture("attachments_github")).unwrap()["nodes"][n].clone()
}

/// What `attachmentLinkGitHubPR` answers with this attachment.
pub fn link_ok(attachment: Value) -> Reply {
    data(json!({ "attachmentLinkGitHubPR": { "success": true, "attachment": attachment } }))
}

/// What `attachmentsForURL` answers for the unlink lookup: attachments given as
/// `(attachment id, url, issue id)`.
pub fn attachment_targets(found: &[(&str, &str, &str)]) -> Reply {
    let nodes: Vec<Value> = found
        .iter()
        .map(|(id, url, issue)| {
            json!({
                "id": id, "title": "Link", "url": url,
                "issue": { "id": issue, "identifier": "EX-23", "url": "https://linear.app/example/issue/EX-23/x" }
            })
        })
        .collect();
    data(json!({ "attachmentsForURL": { "nodes": nodes } }))
}

/// What `integrations` answers for a workspace with these services.
pub fn integrations_of(services: &[&str]) -> Reply {
    let nodes: Vec<Value> = services
        .iter()
        .map(|s| json!({ "service": s, "archivedAt": null }))
        .collect();
    data(json!({ "integrations": { "nodes": nodes } }))
}

pub fn comment_ok() -> Reply {
    let comment = fixture_data("issue_comments")["issue"]["comments"]["nodes"][0].clone();
    data(json!({ "commentCreate": { "success": true, "comment": comment } }))
}

/// The routes a plain create needs, with `extra` added (and overriding).
pub fn create_routes(extra: Vec<(&str, Vec<Reply>)>) -> Vec<(&str, Vec<Reply>)> {
    let mut routes: Vec<(&str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("Teams", vec![teams()]),
        ("ProjectRefs", vec![project_refs()]),
        (
            "ProjectOwnershipQuery",
            vec![ownership(PROJECT, Some(ALICE))],
        ),
        ("Users", vec![users()]),
        ("Labels", vec![labels()]),
        ("Templates", vec![templates()]),
        ("AttachmentsForUrlQuery", vec![no_issue_with_source()]),
        ("IssueCreate", vec![issue_payload("issueCreate", "EX-30")]),
        ("AttachmentCreate", vec![attachment_ok()]),
        ("IssueDelete", vec![delete_ok()]),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

/// A description that fills every section of `Sectioned Template`.
pub const GOOD_BODY: &str =
    "## Background\n\nWhy.\n\n## Acceptance criteria\n\n- done\n\n## Out of scope\n\nNothing.\n";

pub fn write_file(sb: &Sandbox, name: &str, text: &str) -> String {
    let path = sb.cwd().join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}
