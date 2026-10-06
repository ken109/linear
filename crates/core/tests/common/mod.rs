//! Builders for audit tests: small hand-written issues and projects with every
//! unrelated field filled in, so a test states only what it is about.

#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use linear_core::audit::{Finding, Snapshot};
use linear_core::types::{Issue, Project};
use serde_json::{json, Value};

/// The fixed "now" of the audit tests: 2026-10-20 12:00 UTC.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}

pub const WS: &str = "ken109";

fn user(is_me: bool) -> Value {
    let (id, name) = if is_me {
        ("u-me", "Me")
    } else {
        ("u-other", "Other")
    };
    json!({
        "id": id, "name": name, "displayName": name.to_lowercase(),
        "email": format!("{id}@example.com"), "active": true, "isMe": is_me,
    })
}

fn state(kind: &str) -> Value {
    json!({ "id": format!("s-{kind}"), "name": format!("State {kind}"), "type": kind })
}

fn project_ref(slug: &str) -> Value {
    json!({
        "id": format!("p-{slug}"), "slugId": slug, "name": format!("Project {slug}"),
        "url": format!("https://linear.app/x/project/{slug}"),
    })
}

fn milestone(project_slug: &str, name: &str, target: Option<&str>, status: &str) -> Value {
    json!({
        "id": format!("m-{name}"), "name": name, "description": null,
        "targetDate": target, "status": status, "sortOrder": 0.0, "progress": 0.0,
        "project": project_ref(project_slug),
    })
}

pub struct IssueB(Value);

pub fn issue(identifier: &str) -> IssueB {
    IssueB(json!({
        "id": format!("i-{identifier}"),
        "identifier": identifier,
        "title": format!("Title of {identifier}"),
        "url": format!("https://linear.app/x/issue/{identifier}"),
        "team": { "id": "t-1", "key": "KK", "name": "Team" },
        "state": state("unstarted"),
        "assignee": null,
        "project": null,
        "projectMilestone": null,
        "labels": { "nodes": [] },
        "dueDate": null,
        "estimate": null,
        "createdAt": "2026-09-01T00:00:00Z",
        "updatedAt": "2026-10-19T00:00:00Z",
        "startedAt": null,
        "completedAt": null,
        "parent": null,
        "attachments": { "nodes": [] },
    }))
}

impl IssueB {
    fn set(mut self, key: &str, v: Value) -> Self {
        self.0[key] = v;
        self
    }
    pub fn state(self, kind: &str) -> Self {
        self.set("state", state(kind))
    }
    pub fn mine(self) -> Self {
        self.set("assignee", user(true))
    }
    pub fn theirs(self) -> Self {
        self.set("assignee", user(false))
    }
    pub fn updated(self, ts: &str) -> Self {
        self.set("updatedAt", json!(ts))
    }
    pub fn due(self, date: &str) -> Self {
        self.set("dueDate", json!(date))
    }
    pub fn started(self, ts: &str) -> Self {
        self.set("startedAt", json!(ts))
    }
    pub fn completed(self, ts: &str) -> Self {
        self.set("completedAt", json!(ts))
    }
    pub fn in_project(self, slug: &str) -> Self {
        self.set("project", project_ref(slug))
    }
    pub fn in_milestone(self, project_slug: &str, name: &str) -> Self {
        self.set(
            "projectMilestone",
            milestone(project_slug, name, None, "next"),
        )
    }
    pub fn build(self) -> Issue {
        serde_json::from_value(self.0).expect("issue fixture")
    }
}

pub struct ProjectB(Value);

pub fn project(slug: &str) -> ProjectB {
    ProjectB(json!({
        "id": format!("p-{slug}"),
        "slugId": slug,
        "name": format!("Project {slug}"),
        "url": format!("https://linear.app/x/project/{slug}"),
        "status": { "id": "ps-started", "name": "In Progress", "type": "started" },
        "lead": null,
        "startDate": null,
        "targetDate": null,
        "health": null,
        "projectMilestones": { "nodes": [] },
        "initiatives": { "nodes": [] },
        "lastUpdate": null,
        "issues": { "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } },
        "updatedAt": "2026-10-19T00:00:00Z",
    }))
}

impl ProjectB {
    fn set(mut self, key: &str, v: Value) -> Self {
        self.0[key] = v;
        self
    }
    /// `kind` is a project status type: backlog, planned, started, paused,
    /// completed or canceled.
    pub fn status(self, kind: &str) -> Self {
        self.set(
            "status",
            json!({ "id": format!("ps-{kind}"), "name": format!("Status {kind}"), "type": kind }),
        )
    }
    pub fn lead_me(self) -> Self {
        self.set("lead", user(true))
    }
    pub fn lead_other(self) -> Self {
        self.set("lead", user(false))
    }
    pub fn target(self, date: &str) -> Self {
        self.set("targetDate", json!(date))
    }
    pub fn milestone(mut self, name: &str, target: Option<&str>, status: &str) -> Self {
        let slug = self.0["slugId"].as_str().unwrap().to_owned();
        self.0["projectMilestones"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(milestone(&slug, name, target, status));
        self
    }
    /// The workflow state types of the project's issues, as the project
    /// fragment sees them.
    pub fn issue_states(self, kinds: &[&str]) -> Self {
        let nodes: Vec<Value> = kinds.iter().map(|k| json!({ "state": state(k) })).collect();
        let mut s = self;
        s.0["issues"]["nodes"] = json!(nodes);
        s
    }
    /// More issues exist than the selection window holds.
    pub fn issues_truncated(mut self) -> Self {
        self.0["issues"]["pageInfo"]["hasNextPage"] = json!(true);
        self
    }
    /// The latest status update, posted at `created`.
    pub fn update_at(self, created: &str) -> Self {
        let slug = self.0["slugId"].as_str().unwrap().to_owned();
        self.set(
            "lastUpdate",
            json!({
                "id": format!("su-{slug}"),
                "url": format!("https://linear.app/x/project/{slug}/activity"),
                "body": "Update.",
                "health": "onTrack",
                "createdAt": created,
                "updatedAt": created,
                "user": user(true),
                "project": project_ref(&slug),
            }),
        )
    }
    pub fn build(self) -> Project {
        serde_json::from_value(self.0).expect("project fixture")
    }
}

pub fn snapshot(issues: Vec<Issue>, projects: Vec<Project>) -> Snapshot {
    Snapshot {
        workspace: WS.to_owned(),
        issues,
        projects,
    }
}

/// The findings of one rule, as `(target identifier, actionable)` pairs.
pub fn of_rule(findings: &[Finding], rule: linear_core::audit::RuleId) -> Vec<(String, bool)> {
    findings
        .iter()
        .filter(|f| f.rule == rule)
        .map(|f| (f.target.identifier.clone(), f.actionable))
        .collect()
}
