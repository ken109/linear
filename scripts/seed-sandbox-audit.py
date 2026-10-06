#!/usr/bin/env python3
"""Seed the sandbox workspace with the discrepancies the live audit tests look for.

    set -a; . ~/.config/linear-dev/sandbox.env; set +a
    python3 scripts/seed-sandbox-audit.py

Everything it creates is named `audit-seed ...`. It is safe to run again: what
exists is left alone. It refuses to run against any workspace but the sandbox
(url key `ken109-sandbox`), and it never prints the API key.

What it plants (see crates/cli/tests/live_audit.rs for what is asserted):

  project  "audit-seed overdue project"      In Progress, no lead, target date in the past,
                                             one milestone with a past target date
    issue  "audit-seed late issue"           Todo, mine, past due date, in no milestone, no body
    issue  "audit-seed half-written issue"   In Progress, mine, in the milestone, a body that fills
                                             only part of "Sectioned Template", no source attachment
    issue  "audit-seed duplicate issue"      in the Duplicate state (canceledAt is set)
    issue  "audit-seed canceled issue"       in the Canceled state
  project  "audit-seed completed project"    Completed, with
    issue  "audit-seed open issue"           Todo

Not planted: two labels of one single-select group on an issue. Linear refuses that
(and drops a label that joins a group the issue already has a label of), so the
`label-groups-exclusive` rule can only be exercised by the unit tests.
"""
import json
import os
import sys
import urllib.request

URL = "https://api.linear.app/graphql"
URL_KEY = "ken109-sandbox"
KEY = os.environ.get("LINEAR_API_KEY_SANDBOX", "")
if not KEY:
    sys.exit("LINEAR_API_KEY_SANDBOX is not set")


def gql(query, variables=None):
    body = json.dumps({"query": query, "variables": variables or {}}).encode()
    req = urllib.request.Request(
        URL, body, {"content-type": "application/json", "authorization": KEY}
    )
    with urllib.request.urlopen(req) as r:
        out = json.load(r)
    if out.get("errors"):
        sys.exit("Linear said: " + json.dumps(out["errors"])[:500])
    return out["data"]


who = gql("{ viewer { id } organization { urlKey } }")
if who["organization"]["urlKey"] != URL_KEY:
    sys.exit(f"refusing to seed workspace {who['organization']['urlKey']!r}: only {URL_KEY!r}")
me = who["viewer"]["id"]

team = gql("{ teams { nodes { id key states { nodes { id type } } } } }")["teams"]["nodes"]
team = next(t for t in team if t["key"] == "SAND")
state = {s["type"]: s["id"] for s in team["states"]["nodes"]}
pstatus = {
    s["type"]: s["id"]
    for s in gql("{ projectStatuses { nodes { id type } } }")["projectStatuses"]["nodes"]
}


def find(kind, name, field="name"):
    data = gql(
        "query($n: String!) { %s(filter: { %s: { eq: $n } }) { nodes { id } } }" % (kind, field),
        {"n": name},
    )
    nodes = data[kind]["nodes"]
    return nodes[0]["id"] if nodes else None


# The field each create mutation returns its entity under.
CREATED = {
    "projectCreate": "project",
    "issueCreate": "issue",
    "issueLabelCreate": "issueLabel",
    "projectMilestoneCreate": "projectMilestone",
}


def ensure(kind, name, mutation, key, input_, field="name"):
    found = find(kind, name, field)
    if found:
        print(f"exists   {name}")
        return found
    made = gql(mutation, {"input": input_})[key]
    print(f"created  {name}")
    return made[CREATED[key]]["id"]


# ---- the overdue, leaderless project, its milestone and issues
late = ensure(
    "projects", "audit-seed overdue project",
    "mutation($input: ProjectCreateInput!) { projectCreate(input: $input) { project { id } } }",
    "projectCreate",
    {
        "name": "audit-seed overdue project",
        "teamIds": [team["id"]],
        "statusId": pstatus["started"],
        "targetDate": "2026-09-01",
    },
)
milestone = ensure(
    "projectMilestones", "audit-seed milestone",
    "mutation($input: ProjectMilestoneCreateInput!) { projectMilestoneCreate(input: $input) { projectMilestone { id } } }",
    "projectMilestoneCreate",
    {"projectId": late, "name": "audit-seed milestone", "targetDate": "2026-09-15"},
)

ISSUE = "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { issue { id } } }"


def issue(title, **fields):
    return ensure(
        "issues", title, ISSUE, "issueCreate",
        {"title": title, "teamId": team["id"], **fields}, field="title",
    )


issue(
    "audit-seed late issue",
    projectId=late, assigneeId=me, stateId=state["unstarted"], dueDate="2026-09-10",
)
issue(
    "audit-seed half-written issue",
    projectId=late, projectMilestoneId=milestone, assigneeId=me, stateId=state["started"],
    description="## Background\nWhy this matters.\n\n## Acceptance criteria\n",
)
late_issue = find("issues", "audit-seed late issue", "title")
duplicate_existed = find("issues", "audit-seed duplicate issue", "title") is not None
duplicate = issue("audit-seed duplicate issue", projectId=late, stateId=state["unstarted"])
if not duplicate_existed:
    # Linear puts an issue in the Duplicate state when it is marked as a duplicate of another.
    gql(
        "mutation($input: IssueRelationCreateInput!) { issueRelationCreate(input: $input) { success } }",
        {"input": {"issueId": duplicate, "relatedIssueId": late_issue, "type": "duplicate"}},
    )
    print("marked the duplicate issue as a duplicate")
issue("audit-seed canceled issue", projectId=late, stateId=state["canceled"])

# ---- the completed project with an open issue
done = ensure(
    "projects", "audit-seed completed project",
    "mutation($input: ProjectCreateInput!) { projectCreate(input: $input) { project { id } } }",
    "projectCreate",
    {
        "name": "audit-seed completed project",
        "teamIds": [team["id"]],
        "statusId": pstatus["completed"],
        "leadId": me,
    },
)
issue("audit-seed open issue", projectId=done, assigneeId=me, stateId=state["unstarted"])
print("done")
