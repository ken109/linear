#!/usr/bin/env python3
"""Run the same scenarios through the old TypeScript tool and the Rust CLI against the
sandbox workspace, read the resulting Linear state back over GraphQL, and compare.

Temporary harness: it exists only while the old tool (`tools/linear.ts` in ken109/monorepo)
is still around. See README.md. Python 3 standard library only.

Safety:
  * Both tools are given the SANDBOX key and nothing else. The old tool is run with an
    otherwise empty credential environment (its OAuth token file is pointed at nothing), so it
    cannot fall back to the real workspace.
  * Before any write, the key is checked: the organization must be `ken109-sandbox`.
  * Everything written lives in projects named `parity-<timestamp>-old` / `-new` (and `-b`,
    `-probe` variants). Cleanup touches only projects, their issues and templates whose name
    matches that pattern. Seed data is never written.
"""

from __future__ import annotations

import argparse
import copy
import difflib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

SANDBOX_URL_KEY = "ken109-sandbox"
SANDBOX_TEAM = "SAND"
ENDPOINT = "https://api.linear.app/graphql"
TAG_RE = re.compile(r"parity-\d{14}-(?:old|new)")
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
SLUG_URL_RE = re.compile(r"https://linear\.app/[^\s\"')]*")
# Names of what this harness may clean up. Nothing else is ever touched.
OWNED_RE = re.compile(r"^parity-\d{14}-(?:old|new)(?:-[a-z0-9]+)*$")


# ------------------------------------------------------------------------------ GraphQL


class Linear:
    """Direct GraphQL reads/cleanup with the sandbox key. Independent of both tools."""

    def __init__(self, key: str):
        self._key = key

    requests = 0
    remaining: int | None = None  # from the X-RateLimit-Requests-Remaining header

    def q(self, query: str, variables: dict | None = None) -> dict:
        body = json.dumps({"query": query, "variables": variables or {}}).encode()
        last = None
        Linear.requests += 1
        for wait in (0, 2, 5, 10):
            time.sleep(wait)
            req = urllib.request.Request(
                ENDPOINT,
                data=body,
                headers={"content-type": "application/json", "authorization": self._key},
            )
            try:
                with urllib.request.urlopen(req, timeout=60) as r:
                    left = r.headers.get("X-RateLimit-Requests-Remaining")
                    if left is not None and left.isdigit():
                        Linear.remaining = int(left)
                    out = json.load(r)
            except urllib.error.HTTPError as e:
                text = e.read().decode(errors="replace")
                last = f"HTTP {e.code}: {text[:300]}"
                if "RATELIMITED" in text:
                    # Do not retry: it only prolongs the limit (2500 requests an hour per key).
                    raise SystemExit("rate limited by Linear; wait (up to an hour), then run "
                                     "scripts/parity/run.sh --cleanup-only and run again")
                if e.code in (429, 500, 502, 503, 504):
                    continue
                raise RuntimeError(last) from None
            except urllib.error.URLError as e:
                last = str(e)
                continue
            if out.get("errors"):
                raise RuntimeError(json.dumps(out["errors"])[:600])
            return out["data"]
        raise RuntimeError(f"Linear unreachable: {last}")


# ------------------------------------------------------------------------------ tools


@dataclass
class Result:
    code: int
    out: str
    err: str
    json: Any = None


class Tool:
    kind = ""

    def run(self, args: list[str], stdin: str | None = None) -> Result:  # pragma: no cover
        raise NotImplementedError

    @staticmethod
    def _exec(cmd: list[str], env: dict, stdin: str | None, cwd: str | None = None) -> Result:
        p = subprocess.run(
            cmd, input=stdin, capture_output=True, text=True, env=env, cwd=cwd, timeout=300
        )
        parsed = None
        text = p.stdout.strip()
        if text[:1] in "[{":
            try:
                parsed = json.loads(text)
            except ValueError:
                parsed = None
        return Result(p.returncode, p.stdout, p.stderr, parsed)


class OldTool(Tool):
    """`bun tools/linear.ts` from a temporary copy whose only change is the hard-coded team key
    (`KK`), so it can address the sandbox team. The original file is only read."""

    kind = "old"

    def __init__(self, tools_dir: Path, key: str, workdir: Path):
        src = (tools_dir / "linear.ts").read_text()
        auth = (tools_dir / "linear-auth.ts").read_text()
        needle = 'const TEAM_KEY = "KK";'
        if src.count(needle) != 1:
            raise SystemExit("old tool: cannot find the TEAM_KEY line to redirect; refusing to run")
        if 'const ENDPOINT = "https://api.linear.app/graphql";' not in src:
            raise SystemExit("old tool: unexpected endpoint; refusing to run")
        self.dir = workdir / "old-tool"
        self.dir.mkdir(parents=True)
        (self.dir / "linear.ts").write_text(
            src.replace(needle, f'const TEAM_KEY = "{SANDBOX_TEAM}";')
        )
        (self.dir / "linear-auth.ts").write_text(auth)
        self.env = {
            "PATH": os.environ["PATH"],
            "HOME": os.environ.get("HOME", ""),
            "KEN109_LINEAR_API_KEY": key,
            # Without this the old tool could fall back to the stored OAuth token (ken109).
            "KEN109_LINEAR_TOKEN_FILE": "/nonexistent/parity-no-token.json",
        }

    def run(self, args: list[str], stdin: str | None = None) -> Result:
        return self._exec(["bun", str(self.dir / "linear.ts"), *args], self.env, stdin)


class NewTool(Tool):
    kind = "new"

    def __init__(self, binary: str, key: str, workdir: Path):
        cfg = workdir / "cfg"
        self.env = {
            "PATH": os.environ["PATH"],
            "HOME": os.environ.get("HOME", ""),
            "LINEAR_CONFIG_DIR": str(cfg),
            "LINEAR_CACHE_DIR": str(workdir / "cache"),
            "LINEAR_API_KEY_SANDBOX": key,
            "LINEAR_WORKSPACE": "sandbox",
        }
        self.bin = binary
        r = self._exec(
            [binary, "workspace", "add", "sandbox", "--url-key", SANDBOX_URL_KEY, "--team",
             SANDBOX_TEAM, "--default"],
            self.env, None,
        )
        if r.code != 0:
            raise SystemExit(f"cannot configure the new CLI: {r.err}")
        path = cfg / "workspaces.toml"
        path.write_text(
            path.read_text()
            + '\nrules = ["template-sections", "source-attachment", "label-groups-exclusive"]\n'
            # The old tool checks template sections for issues only. By default the Rust rule also
            # asks project bodies for a project template (exit 5), so narrow it to match.
            + 'rule_operations = { "template-sections" = ["issue_create"] }\n'
        )

    def run(self, args: list[str], stdin: str | None = None, as_json: bool = True) -> Result:
        cmd = [self.bin, *args] + (["--json"] if as_json else [])
        r = self._exec(cmd, self.env, stdin)
        if r.code == 1 and "timeout" in r.err:
            # Linear is occasionally slower than the CLI's 30 s limit; one retry, noted in stderr.
            r = self._exec(cmd, self.env, stdin)
            r.err = "(retried once after a timeout) " + r.err
        return r


# ------------------------------------------------------------------------------ snapshots

PROJECT_Q = """
query($tag: String!) { projects(first: 10, filter: { name: { startsWith: $tag } }) { nodes {
  id name description content targetDate status { name } lead { email }
  initiatives(first: 5) { nodes { name } }
  projectMilestones(first: 20) { nodes { name description targetDate sortOrder } }
  projectUpdates(first: 20) { nodes { health body createdAt user { email } } }
} } }
"""
# One request for the issues of every scratch project (the API allows 2500 requests an hour).
ISSUES_Q = """
query($tag: String!) { issues(first: 40, filter: { project: { name: { startsWith: $tag } } }) { nodes {
  project { name }
  identifier title description createdAt sortOrder prioritySortOrder dueDate priority
  state { name } assignee { email } projectMilestone { name } labels(first: 5) { nodes { name } }
  attachments(first: 5) { nodes { title url } }
  comments(first: 10) { nodes { body createdAt user { email } } }
} } }
"""
TEMPLATES_Q = "query { templates { id name type description templateData } }"
INITIATIVES_Q = """
query($tag: String!) { initiatives(first: 10, filter: { name: { startsWith: $tag } }) { nodes {
  id name description status projects(first: 10) { nodes { name } }
} } }
"""


def headings_of(doc: Any) -> list[str]:
    out: list[str] = []

    def text(n: Any) -> str:
        if isinstance(n.get("text"), str):
            return n["text"]
        return "".join(text(c) for c in n.get("content", []))

    def walk(n: Any) -> None:
        if n.get("type") == "heading":
            out.append(text(n))
        for c in n.get("content", []):
            walk(c)

    walk(doc)
    return out


class Observer:
    def __init__(self, gql: Linear, tag: str):
        self.gql = gql
        self.tag = tag

    def snapshot(self, slow: bool = False) -> dict:
        """Projects, milestones, updates and issues every time (2 requests). Templates and
        initiatives only when `slow` (they only change in the steps that touch them)."""
        if slow or not hasattr(self, "_cache"):
            self._cache = {"templates": self._templates(), "initiatives": self._initiatives()}
        projects = self.gql.q(PROJECT_Q, {"tag": self.tag})["projects"]["nodes"]
        all_issues = self.gql.q(ISSUES_Q, {"tag": self.tag})["issues"]["nodes"]
        snap: dict[str, Any] = {"projects": {}, **copy.deepcopy(self._cache)}
        for p in projects:
            issues = [i for i in all_issues if i["project"]["name"] == p["name"]]
            issues.sort(key=lambda i: i["createdAt"])
            by_sort = [i["title"] for i in sorted(issues, key=lambda i: i["sortOrder"])]
            by_prio = [i["title"] for i in sorted(issues, key=lambda i: i["prioritySortOrder"])]
            updates = sorted(p["projectUpdates"]["nodes"], key=lambda u: u["createdAt"])
            snap["projects"][p["name"]] = {
                "summary": p["description"],
                "content": p["content"],
                "targetDate": p["targetDate"],
                "status": p["status"]["name"],
                "lead": (p["lead"] or {}).get("email"),
                "initiatives": sorted(i["name"] for i in p["initiatives"]["nodes"]),
                "milestones": [
                    {k: m[k] for k in ("name", "description", "targetDate")}
                    for m in sorted(p["projectMilestones"]["nodes"], key=lambda m: m["sortOrder"])
                ],
                "statusUpdates": [
                    {"health": u["health"], "body": u["body"], "user": (u["user"] or {}).get("email")}
                    for u in updates
                ],
                "issues": [
                    {
                        "title": i["title"],
                        "description": i["description"],
                        "state": i["state"]["name"],
                        "assignee": (i["assignee"] or {}).get("email"),
                        "dueDate": i["dueDate"],
                        "priority": i["priority"],
                        "milestone": (i["projectMilestone"] or {}).get("name"),
                        "labels": sorted(l["name"] for l in i["labels"]["nodes"]),
                        "attachments": sorted(
                            ({"title": a["title"], "url": a["url"]} for a in i["attachments"]["nodes"]),
                            key=lambda a: a["url"],
                        ),
                        "comments": [
                            {"body": c["body"], "user": (c["user"] or {}).get("email")}
                            for c in sorted(i["comments"]["nodes"], key=lambda c: c["createdAt"])
                        ],
                    }
                    for i in issues
                ],
                "orderBySortOrder": by_sort,
                "orderByPrioritySortOrder": by_prio,
            }
        return snap

    def _templates(self) -> dict:
        out = {}
        for t in self.gql.q(TEMPLATES_Q)["templates"]:
            if not t["name"].startswith(self.tag):
                continue
            data = t["templateData"]
            data = json.loads(data) if isinstance(data, str) else data
            desc = data.get("descriptionData")
            desc = json.loads(desc) if isinstance(desc, str) else desc
            out[t["name"]] = {
                "type": t["type"],
                "description": t["description"],
                "headings": headings_of(desc) if desc else [],
            }
        return out

    def _initiatives(self) -> dict:
        nodes = self.gql.q(INITIATIVES_Q, {"tag": self.tag})["initiatives"]["nodes"]
        return {
            i["name"]: {"description": i["description"], "status": i["status"],
                        "projects": sorted(p["name"] for p in i["projects"]["nodes"])}
            for i in nodes
        }

    def normalize(self, snap: dict, drop_probes: bool = True) -> dict:
        """Replace the tool tag, ids and URLs so the two tools' states can be compared."""
        text = json.dumps(snap, sort_keys=True)
        text = text.replace(self.tag, "<TAG>")
        text = TAG_RE.sub("<TAG>", text)
        text = UUID_RE.sub("<uuid>", text)
        text = SLUG_URL_RE.sub(lambda m: m.group(0).split("?")[0], text)
        # The default title of the source attachment differs: the old tool says "出どころ",
        # linear says "Source". Reported in docs/parity.md, so it is not repeated as a state diff.
        text = text.replace(json.dumps("出どころ"), '"<default source title>"').replace(
            '"title": "Source"', '"title": "<default source title>"')
        out = json.loads(text)
        if drop_probes:
            out["projects"] = {k: v for k, v in out["projects"].items() if "-probe" not in k}
            out["templates"] = {k: v for k, v in out["templates"].items() if "-probe" not in k}
        return out


# ------------------------------------------------------------------------------ driver


@dataclass
class Step:
    label: str
    desc: str
    cmd: list[str]
    code: int
    err: str
    out_keys: list[str]
    changed: bool
    state: dict
    extra: dict = field(default_factory=dict)


class Driver:
    """The same operations expressed for each tool. This class is the old -> new mapping."""

    def __init__(self, tool: Tool, obs: Observer, gql: Linear, files: dict[str, Path], tag: str):
        self.t = tool
        self.old = tool.kind == "old"
        self.obs = obs
        self.gql = gql
        self.f = files
        self.tag = tag
        self.steps: list[Step] = []
        self.state = obs.snapshot(slow=True)
        self.ids: dict[str, str] = {}
        self.viewer_email = gql.q("query { viewer { email } }")["viewer"]["email"]

    # -- plumbing ---------------------------------------------------------------------

    def step(self, label: str, desc: str, args: list[str], stdin: str | None = None) -> Result:
        r = self.t.run(args, stdin)
        # templates (t..) and initiatives (n..) are only read back in the steps that touch them,
        # and in every step that follows the rate limit of the API would be spent on nothing
        after = self.obs.snapshot(slow=label[:1] in ("t", "n"))
        keys: list[str] = []
        if isinstance(r.json, dict):
            keys = sorted(r.json.keys())
        elif isinstance(r.json, list) and r.json and isinstance(r.json[0], dict):
            keys = sorted(r.json[0].keys())
        self.steps.append(
            Step(label, desc, args, r.code, r.err.strip(), keys, after != self.state, after,
                 {"out": r.out.strip()[:400]})
        )
        self.state = after
        mark = "ok " if r.code == 0 else f"x{r.code} "
        print(f"  [{self.t.kind}] {label:<28} {mark}{desc}", flush=True)
        return r

    def repair(self, query: str, variables: dict) -> None:
        """Undo, over GraphQL, what the last step wrote when it should have been refused (the old
        tool accepts some invalid dates). Keeps one accepted mistake from showing up as a state
        difference in every later step. Only runs when the last step succeeded."""
        if self.steps and self.steps[-1].code == 0:
            self.gql.q(query, variables)
            self.steps[-1].extra["repaired"] = True
            self.state = self.obs.snapshot()

    def project_id(self, name: str) -> str:
        data = self.gql.q(
            "query($n: String!) { projects(first: 5, filter: { name: { eq: $n } }) { nodes { id } } }",
            {"n": name},
        )
        nodes = data["projects"]["nodes"]
        if not nodes:
            raise RuntimeError(f"project {name} was not created by {self.t.kind}")
        return nodes[0]["id"]

    def issue_ident(self, source: str) -> str:
        data = self.gql.q(
            "query($u: String!) { attachmentsForURL(url: $u, first: 1) { nodes { issue { identifier } } } }",
            {"u": source},
        )
        return data["attachmentsForURL"]["nodes"][0]["issue"]["identifier"]

    def name(self, suffix: str = "") -> str:
        return f"{self.tag}{suffix}"

    # -- projects ---------------------------------------------------------------------

    def project_create(self, label, desc, name, summary=None, body=None, target=None, lead=None,
                       initiative=None):
        if self.old:
            a = ["create-project", "--name", name]
            if summary is not None:
                a += ["--summary", summary]
            if body is not None:
                a += ["--content-file", str(self.f[body])]
            if target:
                a += ["--target-date", target]
            if lead:
                a += ["--lead", lead]
        else:
            a = ["project", "create", "--name", name]
            if summary is not None:
                a += ["--summary", summary]
            if body is not None:
                a += ["--body-file", str(self.f[body])]
            if target:
                a += ["--target-date", target]
            if lead:
                a += ["--lead", lead]
        if initiative:
            a += ["--initiative", initiative]
        return self.step(label, desc, a)

    def initiative_create(self, label, desc, name, description=None):
        flags = ["--name", name]
        if description is not None:
            flags += ["--description-file", str(self.f[description])]
        a = ["create-initiative", *flags] if self.old else ["initiative", "create", *flags]
        return self.step(label, desc, a)

    def project_update(self, label, desc, ref, name=None, summary=None, body=None, status=None,
                       target=None, lead=None, initiative=None):
        flags: list[str] = []
        if initiative is not None:
            flags += ["--initiative", initiative]
        if name is not None:
            flags += ["--name", name]
        if summary is not None:
            flags += ["--summary", summary]
        if body is not None:
            flags += ["--content-file" if self.old else "--body-file", str(self.f[body])]
        if status is not None:
            flags += ["--state" if self.old else "--status", status]
        if target is not None:
            flags += ["--target-date", target]
        if lead is not None:
            flags += ["--lead", lead]
        a = ["update-project", "--id", ref, *flags] if self.old else ["project", "update", ref, *flags]
        return self.step(label, desc, a)

    def status_update(self, label, desc, ref, health, body):
        if self.old:
            a = ["status-update", "--project", ref, "--health", health, "--body-file", str(self.f[body])]
        else:
            a = ["project", "status-update", ref, "--health", health, "--body-file", str(self.f[body])]
        return self.step(label, desc, a)

    # -- milestones -------------------------------------------------------------------

    def milestone_create(self, label, desc, project, name, target=None, description=None):
        flags = ["--name", name]
        if target is not None:
            flags += ["--target-date", target]
        if description is not None:
            flags += ["--description-file", str(self.f[description])]
        a = ["create-milestone", "--project", project, *flags] if self.old else [
            "milestone", "create", "--project", project, *flags]
        return self.step(label, desc, a)

    def milestone_update(self, label, desc, project, name, new_name=None, target=None, description=None):
        flags: list[str] = []
        if new_name is not None:
            flags += ["--new-name", new_name]
        if target is not None:
            flags += ["--target-date", target]
        if description is not None:
            flags += ["--description-file", str(self.f[description])]
        a = ["update-milestone", "--project", project, "--name", name, *flags] if self.old else [
            "milestone", "update", name, "--project", project, *flags]
        return self.step(label, desc, a)

    def milestone_delete(self, label, desc, project, name):
        a = ["delete-milestone", "--project", project, "--name", name] if self.old else [
            "milestone", "delete", name, "--project", project]
        return self.step(label, desc, a)

    # -- issues -----------------------------------------------------------------------

    def issue_create(self, label, desc, project, title, template, body, source, source_title=None,
                     milestone=None, assignee=None, labels=()):
        flags = ["--project", project, "--title", title, "--template", template,
                 "--body-file", str(self.f[body]), "--source", source]
        if source_title:
            flags += ["--source-title", source_title]
        if milestone:
            flags += ["--milestone", milestone]
        if assignee:
            flags += ["--assignee", assignee]
        for l in labels:
            flags += ["--label", l]
        a = ["create-issue", *flags] if self.old else ["issue", "create", *flags]
        return self.step(label, desc, a)

    def issue_update(self, label, desc, ident, state=None, project=None, milestone=None, due=None,
                     assignee=None):
        flags: list[str] = []
        for flag, v in (("--state", state), ("--project", project), ("--milestone", milestone),
                        ("--due", due), ("--assignee", assignee)):
            if v is not None:
                flags += [flag, v]
        a = ["update-issue", "--id", ident, *flags] if self.old else ["issue", "update", ident, *flags]
        return self.step(label, desc, a)

    def issue_reorder(self, label, desc, idents):
        a = ["reorder-issues", "--ids", ",".join(idents)] if self.old else ["issue", "reorder", *idents]
        return self.step(label, desc, a)

    def issue_comment(self, label, desc, ident, body):
        if self.old:
            a = ["comment", "--id", ident, "--body-file", str(self.f[body])]
        else:
            a = ["issue", "comment", ident, "--body-file", str(self.f[body])]
        return self.step(label, desc, a)

    # -- templates --------------------------------------------------------------------

    def template_create(self, label, desc, name, body, description=None):
        flags = ["--name", name, "--body-file", str(self.f[body])]
        if description:
            flags += ["--description", description]
        a = ["create-template", *flags] if self.old else ["template", "create", *flags]
        return self.step(label, desc, a)


# ------------------------------------------------------------------------------ scenario

ISSUE_BODY = "## Background\n\nWhy.\n\n## Acceptance criteria\n\n- it works\n\n## Out of scope\n\nNothing.\n"


def write_files(d: Path) -> dict[str, Path]:
    contents = {
        "proj1": "# Plan\n\nFirst body.\n",
        "proj2": "# Plan\n\nSecond body.\n\n- one\n- two\n",
        "empty": "",
        "blank": "  \n\n",
        "status": "- Now: scenario run\n- Next: compare\n- Waiting: nothing\n",
        "mdesc": "Milestone description.\n",
        "mdesc2": "Changed milestone description.\n",
        "issue": ISSUE_BODY,
        "issue_missing": "## Background\n\nWhy.\n\n## Acceptance criteria\n\n- it works\n",
        "issue_empty_section": "## Background\n\nWhy.\n\n## Acceptance criteria\n\n## Out of scope\n\nNothing.\n",
        "comment": "A comment.\n",
        "tpl": "## Why\n\n**Bold** note.\n\n## Plan\n\n- step one\n- step two\n\n## Done when\n\n1. first\n2. second\n",
        "tpl_nohead": "Just a paragraph, no sections.\n",
        "issue_tpl": "## Why\n\nBecause.\n\n## Plan\n\n- do it\n\n## Done when\n\nIt is done.\n",
        "issue_tpl_missing": "## Why\n\nBecause.\n",
    }
    files = {}
    for k, v in contents.items():
        p = d / f"{k}.md"
        p.write_text(v)
        files[k] = p
    return files


def scenario(d: Driver) -> None:
    p1, p2 = d.name(), d.name("-b")
    day = lambda s: s  # readability
    src = lambda n: f"https://example.com/parity/{d.tag}/{n}"

    # ---- projects
    d.project_create("p01.create", "create project", p1, "Parity scratch", "proj1", "2026-12-31")
    pid1 = d.project_id(p1)
    d.project_create("p02.create-again", "same name again (idempotent)", p1, "Parity scratch",
                     "proj1", "2026-12-31")
    d.project_create("p03.create-b", "second project (for moves)", p2, "Second scratch", "proj1")
    pid2 = d.project_id(p2)
    d.project_create("p04.create-no-summary", "create without --summary/--body", d.name("-probe"))
    d.project_create("p05.create-bad-date", "create with a bad target date", d.name("-probe2"),
                     "x", "proj1", "2026-13-45")
    d.project_create("p05b.create-unknown-initiative", "create under an initiative that does not exist",
                     d.name("-probe3"), "x", "proj1", initiative="Nope")
    d.project_update("p06.update-text", "summary + body + target date", pid1, summary="Changed",
                     body="proj2", target="2027-01-31")
    d.project_update("p07.update-status", "status In Progress", pid1, status="In Progress")
    d.project_update("p08.update-status-again", "same status again", pid1, status="In Progress")
    d.project_update("p09.update-status-case", "status name in other case", pid1, status="planned")
    d.project_update("p10.update-lead", "lead me", pid1, lead="me")
    d.project_update("p11.rename", "rename (keeps the tag prefix)", pid1, name=p1 + "-r")
    d.project_update("p12.update-nothing", "no change requested", pid1)
    d.project_update("p13.update-bad-date", "bad target date (Feb 30)", pid1, target="2027-02-30")
    d.repair("mutation($id: String!) { projectUpdate(id: $id, input: { targetDate: \"2027-01-31\" }) { success } }",
             {"id": pid1})
    d.project_update("p14.update-bad-status", "unknown status", pid1, status="Nope")
    d.project_update("p15.update-empty-body", "empty body file", pid1, body="empty")
    d.project_update("p16.update-name-clash", "rename to the other project's name", pid1, name=p2)
    d.project_update("p17.update-unknown", "unknown project", "no-such-project-0000", summary="x")
    d.project_update("p18.update-bad-lead", "unknown lead", pid1, lead="nobody@example.invalid")
    d.status_update("p19.status-update", "onTrack with body", pid1, "onTrack", "status")
    d.status_update("p20.status-update-again", "same update again", pid1, "onTrack", "status")
    d.status_update("p21.status-update-risk", "atRisk", pid1, "atRisk", "status")
    d.status_update("p22.status-empty", "empty body", pid1, "onTrack", "empty")
    d.status_update("p23.status-blank", "whitespace-only body", pid1, "onTrack", "blank")
    d.status_update("p24.status-bad-health", "unknown health", pid1, "fine", "status")

    # ---- initiatives (enabled in the sandbox since 2026-10-07)
    ini = d.name("-init")
    d.initiative_create("n01.create", "create an initiative with a description", ini, "mdesc")
    d.initiative_create("n02.create-again", "same name again (idempotent)", ini, "mdesc")
    d.initiative_create("n03.create-no-name", "initiative without a description", d.name("-init2"))
    d.project_create("n04.project-in-initiative", "create a project under the initiative", d.name("-i"),
                     "x", "proj1", initiative=ini)
    d.project_update("n05.link-initiative", "put the first project under the initiative", pid1,
                     initiative=ini)
    d.project_update("n06.link-again", "link it again (idempotent)", pid1, initiative=ini)
    d.project_update("n07.link-unknown", "link to an initiative that does not exist", pid1,
                     initiative="Nope")

    # ---- milestones
    d.milestone_create("m01.create", "M1 with description", pid1, "M1", "2026-11-01", "mdesc")
    d.milestone_create("m02.create-again", "M1 again (idempotent)", pid1, "M1", "2026-11-01", "mdesc")
    d.milestone_create("m03.create-second", "M2", pid1, "M2", "2026-12-01")
    d.milestone_create("m04.create-no-date", "no target date", pid1, "M3")
    d.milestone_create("m05.create-bad-date", "bad target date (Feb 31)", pid1, "M3", "2026-02-31")
    if d.steps[-1].code == 0:  # the old tool accepted it; remove the milestone it made
        for m in d.gql.q("query($id: String!) { project(id: $id) { projectMilestones(first: 20) { nodes { id name } } } }",
                         {"id": pid1})["project"]["projectMilestones"]["nodes"]:
            if m["name"] == "M3":
                d.repair("mutation($id: String!) { projectMilestoneDelete(id: $id) { success } }", {"id": m["id"]})
    d.milestone_update("m06.update", "rename + date + description", pid1, "M1", "M1r", "2026-11-15", "mdesc2")
    d.milestone_update("m07.update-again", "same update again", pid1, "M1r", "M1r", "2026-11-15", "mdesc2")
    d.milestone_update("m08.update-clash", "rename onto M2", pid1, "M1r", "M2")
    d.milestone_update("m09.update-nothing", "no change requested", pid1, "M1r")
    d.milestone_update("m10.update-unknown", "unknown milestone", pid1, "Nope", "X")
    d.milestone_update("m11.update-bad-date", "bad target date", pid1, "M1r", target="nope")
    d.milestone_update("m12.update-case", "current name in other case", pid1, "m1R", target="2026-11-16")
    d.milestone_delete("m13.delete-empty", "delete M2 (empty)", pid1, "M2")
    d.milestone_delete("m14.delete-unknown", "delete a milestone that is not there", pid1, "M2")

    # ---- issues
    base = dict(template="Sectioned Template", body="issue")
    d.issue_create("i01.create", "create A (template, source, milestone, label)", pid1,
                   f"{d.tag} A", source=src("a"), source_title="Origin A", milestone="M1r",
                   labels=["api"], **base)
    a_id = d.issue_ident(src("a"))
    d.issue_create("i02.create-again", "same source again (idempotent)", pid1, f"{d.tag} A again",
                   source=src("a"), **base)
    d.issue_create("i03.missing-section", "body lacks a template section", pid1, f"{d.tag} X",
                   template="Sectioned Template", body="issue_missing", source=src("x1"))
    d.issue_create("i04.empty-section", "a template section is left empty", pid1, f"{d.tag} X",
                   template="Sectioned Template", body="issue_empty_section", source=src("x2"))
    d.issue_create("i05.empty-body", "empty body file", pid1, f"{d.tag} X", template="Sectioned Template",
                   body="empty", source=src("x3"))
    d.issue_create("i06.unknown-template", "unknown template", pid1, f"{d.tag} X", template="Nope",
                   body="issue", source=src("x4"))
    d.issue_create("i07.unknown-milestone", "unknown milestone", pid1, f"{d.tag} X", source=src("x5"),
                   milestone="Nope", **base)
    d.issue_create("i08.unknown-label", "unknown label", pid1, f"{d.tag} X", source=src("x6"),
                   labels=["nope"], **base)
    d.issue_create("i09.bad-source", "source that is not a URL (probe)", d.project_id(p2), f"{d.tag} X",
                   source="not a url", **base)
    d.issue_create("i10.unknown-assignee", "unknown assignee", pid1, f"{d.tag} X", source=src("x7"),
                   assignee="nobody@example.invalid", **base)
    d.issue_create("i11.create-b", "create B (assignee by email)", pid1, f"{d.tag} B", source=src("b"),
                   assignee=d.viewer_email, **base)
    d.issue_create("i12.create-c", "create C", pid1, f"{d.tag} C", source=src("c"), **base)
    d.issue_create("i13.create-d", "create D in the other project", pid2, f"{d.tag} D", source=src("d"),
                   **base)
    b_id, c_id, d_id = (d.issue_ident(src(n)) for n in ("b", "c", "d"))

    d.milestone_delete("m15.delete-with-issue", "delete a milestone that holds issue A", pid1, "M1r")

    d.issue_update("u01.state", "state In Progress", a_id, state="In Progress")
    d.issue_update("u02.state-case", "state name in other case", a_id, state="done")
    d.issue_update("u03.due", "due date", a_id, due="2026-11-20")
    d.issue_update("u04.assignee", "assignee by email", a_id, assignee=d.viewer_email)
    d.issue_update("u05.milestone-same", "milestone it already has", a_id, milestone="M1r")
    d.issue_update("u06.move-project", "move to the other project (milestone cleared)", a_id, project=pid2)
    d.issue_update("u07.move-back", "move back with a milestone", a_id, project=pid1, milestone="M1r")
    d.issue_update("u08.nothing", "no change requested", a_id)
    d.issue_update("u09.bad-state", "unknown state", a_id, state="Nope")
    d.issue_update("u10.bad-due", "bad due date (Nov 31)", a_id, due="2026-11-31")
    d.repair("mutation($id: String!) { issueUpdate(id: $id, input: { dueDate: \"2026-11-20\" }) { success } }",
             {"id": a_id})
    d.issue_update("u11.bad-assignee", "unknown assignee", a_id, assignee="nobody@example.invalid")
    d.issue_update("u12.bad-milestone", "unknown milestone", a_id, milestone="Nope")
    d.issue_update("u13.unknown-issue", "unknown issue", "SAND-99999", state="Done")
    d.issue_update("u14.milestone-in-other", "milestone while moving to a project without it", a_id,
                   project=pid2, milestone="M1r")

    d.issue_reorder("r01.reorder", "A, B, C top first", [a_id, b_id, c_id])
    d.issue_reorder("r02.reorder-same", "same order again", [a_id, b_id, c_id])
    d.issue_reorder("r03.reorder-subset", "B above A (C untouched)", [b_id, a_id])
    d.issue_reorder("r04.reorder-one", "a single issue", [a_id])
    d.issue_reorder("r05.reorder-dup", "the same issue twice", [a_id, a_id])
    d.issue_reorder("r06.reorder-unknown", "an issue that does not exist", [a_id, "SAND-99999"])
    d.issue_reorder("r07.reorder-cross", "issues of two projects", [a_id, d_id])

    d.issue_comment("c01.comment", "comment", a_id, "comment")
    d.issue_comment("c02.comment-empty", "empty comment", a_id, "empty")
    d.issue_comment("c03.comment-blank", "whitespace-only comment", a_id, "blank")
    d.issue_comment("c04.comment-unknown", "unknown issue", "SAND-99999", "comment")

    d.issue_update("u15.move-out", "move A out of M1r's project", a_id, project=pid2)
    d.milestone_delete("m16.delete-emptied", "delete M1r once it is empty", pid1, "M1r")

    # ---- templates
    t1 = d.name("-tpl")
    d.template_create("t01.create", "template with three sections", t1, "tpl", "What it is for")
    d.template_create("t02.create-again", "same name again (idempotent)", t1, "tpl")
    d.template_create("t03.no-heading", "body without a heading", d.name("-probe-tpl"), "tpl_nohead")
    d.issue_create("t04.issue-from-new-template", "issue from the new template, one section missing",
                   pid2, f"{d.tag} E", template=t1, body="issue_tpl_missing", source=src("e1"))
    d.issue_create("t05.issue-from-new-template-ok", "issue from the new template, all sections", pid2,
                   f"{d.tag} E", template=t1, body="issue_tpl", source=src("e2"))


# ------------------------------------------------------------------------------ reads


def read_phase(drivers: dict[str, Driver], gql: Linear) -> list[dict]:
    """Old read commands vs their Rust equivalents on the same sandbox state."""
    rows = []
    old, new = drivers["old"], drivers["new"]

    def pair(label, old_args, new_args, extract_old, extract_new, note="", plain=False):
        ro = old.t.run(old_args)
        rn = new.t.run(new_args, as_json=not plain)
        row = {"label": label, "old_cmd": old_args, "new_cmd": new_args, "old_code": ro.code,
               "new_code": rn.code, "note": note}
        try:
            row["old"] = extract_old(ro)
            row["new"] = extract_new(rn)
            if isinstance(row["old"], str):
                row["old"] = TAG_RE.sub("<TAG>", row["old"])
            if isinstance(row["new"], str):
                row["new"] = TAG_RE.sub("<TAG>", row["new"])
        except Exception as e:  # noqa: BLE001 - report the shape problem instead of dying
            row["old"], row["new"] = f"extract failed: {e}", ""
            row["error"] = f"{ro.err[:200]} | {rn.err[:200]}"
        row["match"] = row["old"] == row["new"]
        rows.append(row)
        print(f"  [read] {label:<24} {'same' if row['match'] else 'DIFFERENT'}", flush=True)

    def n(s: Any) -> Any:
        return json.loads(TAG_RE.sub("<TAG>", json.dumps(s)))

    p_old, p_new = old.project_id(old.name("-r")), new.project_id(new.name("-r"))
    # issues: open issues of the project, in the order the old tool prints them (sortOrder asc)
    pair("issues (set)", ["issues", "--project", p_old], ["issue", "list", "--project", p_new, "--open"],
         lambda r: sorted(n([i["title"] for i in r.json])),
         lambda r: sorted(n([i["title"] for i in (r.json["issues"] if isinstance(r.json, dict) else r.json)])),
         "which open issues a project has")
    pair("issues (order)", ["issues", "--project", p_old], ["issue", "list", "--project", p_new, "--open"],
         lambda r: n([i["title"] for i in r.json]),
         lambda r: n([i["title"] for i in (r.json["issues"] if isinstance(r.json, dict) else r.json)]),
         "order: old = sortOrder ascending (as on screen), new = Linear's default order")
    pair("milestones", ["milestones", "--project", p_old], ["milestone", "list", "--project", p_new],
         lambda r: n([[m["name"], m["targetDate"]] for m in r.json]),
         lambda r: n([[m["name"], m["targetDate"]] for m in (r.json["milestones"] if isinstance(r.json, dict) else r.json)]))
    pair("projects", ["projects"], ["project", "list", "--open", "--all"],
         lambda r: n(sorted([p["name"], p["status"], p["targetDate"]] for p in r.json)),
         lambda r: n(sorted([p["name"], p["status"]["name"], p["targetDate"]]
                            for p in (r.json["projects"] if isinstance(r.json, dict) else r.json))),
         "open projects: name, status, target date")
    pair("templates", ["templates"], ["template", "list"],
         lambda r: n(sorted([t["name"], t["type"]] for t in r.json)),
         lambda r: n(sorted([t["name"], t["type"]]
                            for t in (r.json["templates"] if isinstance(r.json, dict) else r.json))))
    pair("skeletons", ["skeletons"], ["template", "skeleton"],
         lambda r: re.sub(r"\n{3,}", "\n\n", r.out).strip(),
         lambda r: re.sub(r"\n{3,}", "\n\n", r.out).strip(),
         "stdout, every issue template as markdown (blank-line runs collapsed)", plain=True)
    pair("template-one", ["template", "Sectioned Template"], ["template", "skeleton", "Sectioned Template"],
         lambda r: r.out.strip(), lambda r: r.out.strip(), "stdout", plain=True)
    pair("initiatives", ["initiatives"], ["initiative", "list"],
         lambda r: n(sorted(i["name"] for i in r.json)),
         lambda r: n(sorted(i["name"] for i in (r.json["initiatives"] if isinstance(r.json, dict) else r.json))))
    return rows


# ------------------------------------------------------------------------------ cleanup


def cleanup(gql: Linear, prefix_re=OWNED_RE, tags: list[str] | None = None) -> dict:
    """Cancel + archive what the harness created. Only names matching OWNED_RE are touched."""
    done = {"issues": 0, "projects": 0, "templates": 0, "initiatives": 0}
    states = gql.q(
        'query($k: String!) { workflowStates(first: 10, filter: { type: { eq: "canceled" }, '
        "team: { key: { eq: $k } } }) { nodes { id } } }",
        {"k": SANDBOX_TEAM},
    )
    cancel_state = states["workflowStates"]["nodes"][0]["id"]
    pstatus = gql.q("query { projectStatuses { nodes { id type } } }")["projectStatuses"]["nodes"]
    pcancel = next(s["id"] for s in pstatus if s["type"] == "canceled")
    projects = gql.q(
        'query { projects(first: 100, filter: { name: { startsWith: "parity-" } }) { nodes { id name } } }'
    )["projects"]["nodes"]
    for p in projects:
        if not prefix_re.match(p["name"]):
            continue
        if tags is not None and not any(p["name"].startswith(t) for t in tags):
            continue
        issues = gql.q(
            "query($id: String!) { project(id: $id) { issues(first: 100) { nodes { id state { type } } } } }",
            {"id": p["id"]},
        )["project"]["issues"]["nodes"]
        for i in issues:
            if i["state"]["type"] not in ("canceled", "completed"):
                gql.q("mutation($id: String!, $s: String!) { issueUpdate(id: $id, input: { stateId: $s }) { success } }",
                      {"id": i["id"], "s": cancel_state})
            gql.q("mutation($id: String!) { issueArchive(id: $id) { success } }", {"id": i["id"]})
            done["issues"] += 1
        gql.q("mutation($id: String!, $s: String!) { projectUpdate(id: $id, input: { statusId: $s }) { success } }",
              {"id": p["id"], "s": pcancel})
        gql.q("mutation($id: String!) { projectArchive(id: $id) { success } }", {"id": p["id"]})
        done["projects"] += 1
    for t in gql.q(TEMPLATES_Q)["templates"]:
        if t["name"].startswith("parity-") and OWNED_RE.match(t["name"]):
            if tags is None or any(t["name"].startswith(x) for x in tags):
                gql.q("mutation($id: String!) { templateDelete(id: $id) { success } }", {"id": t["id"]})
                done["templates"] += 1
    for i in gql.q('query { initiatives(first: 50, filter: { name: { startsWith: "parity-" } }) '
                   "{ nodes { id name } } }")["initiatives"]["nodes"]:
        if OWNED_RE.match(i["name"]) and (tags is None or any(i["name"].startswith(x) for x in tags)):
            gql.q("mutation($id: String!) { initiativeArchive(id: $id) { success } }", {"id": i["id"]})
            done["initiatives"] += 1
    return done


# ------------------------------------------------------------------------------ report


def classify(a: Step, b: Step) -> tuple[str, str]:
    """(verdict, why) for one step run by the old (a) and the new (b) tool."""
    ok_a, ok_b = a.code == 0, b.code == 0
    if ok_a != ok_b:
        who = "old accepts, new refuses" if ok_a else "old refuses, new accepts"
        return "DIFF", who
    if a.changed != b.changed:
        return "DIFF", f"changed Linear: old={a.changed} new={b.changed}"
    return "same", ""


def report(old_steps: list[Step], new_steps: list[Step], obs: dict[str, Observer], reads: list[dict],
           out: Path) -> str:
    lines = ["# Parity run", ""]
    lines += ["| step | what | old exit | new exit | Linear changed (old/new) | verdict |",
              "| --- | --- | --- | --- | --- | --- |"]
    for a, b in zip(old_steps, new_steps):
        v, why = classify(a, b)
        lines.append(f"| {a.label} | {a.desc} | {a.code} | {b.code} | "
                     f"{'yes' if a.changed else 'no'}/{'yes' if b.changed else 'no'} | {v}{(' (' + why + ')') if why else ''} |")
    # state comparison after every step
    lines += ["", "## Resulting state after each step (normalised: ids, URLs, tag removed)", ""]
    lines.append("A difference is printed at the first step where it appears and again whenever the set "
                 "of differing lines changes; steps in between carry the same difference.\n")
    diffs = 0
    prev: list[str] = []
    for a, b in zip(old_steps, new_steps):
        na = obs["old"].normalize(a.state)
        nb = obs["new"].normalize(b.state)
        cur: list[str] = []
        if na != nb:
            diffs += 1
            sa = json.dumps(na, indent=1, sort_keys=True).splitlines()
            sb = json.dumps(nb, indent=1, sort_keys=True).splitlines()
            cur = [l for l in difflib.unified_diff(sa, sb, "old", "new", lineterm="", n=0)
                   if l[:1] in "+-" and not l.startswith(("+++", "---"))]
        if cur != prev:
            if cur:
                lines += [f"### {a.label}: states differ", "", "```diff"] + cur[:50] + ["```", ""]
            else:
                lines += [f"### {a.label}: states identical again", ""]
            prev = cur
    lines.append(f"{len(old_steps) - diffs} of {len(old_steps)} steps end in an identical state.")
    lines += ["", "## Output shape of successful commands (top-level JSON keys)", "",
              "| step | old keys | new keys |", "| --- | --- | --- |"]
    for a, b in zip(old_steps, new_steps):
        if a.code == 0 and b.code == 0 and (a.out_keys or b.out_keys) and a.out_keys != b.out_keys:
            lines.append(f"| {a.label} | {', '.join(a.out_keys)} | {', '.join(b.out_keys)} |")
    lines += ["", "## Error messages of refused steps", "",
              "| step | old (exit) | new (exit) |", "| --- | --- | --- |"]
    for a, b in zip(old_steps, new_steps):
        if a.code != 0 or b.code != 0:
            ea = a.err.splitlines()[0][:110] if a.err else ""
            eb = b.err.splitlines()[0][:110] if b.err else ""
            lines.append(f"| {a.label} | `{ea}` ({a.code}) | `{eb}` ({b.code}) |")
    lines += ["", "## Reads (old read command vs Rust equivalent, same sandbox state)", "",
              "| read | old exit | new exit | result |", "| --- | --- | --- | --- |"]
    for r in reads:
        lines.append(f"| {r['label']} | {r['old_code']} | {r['new_code']} | {'same' if r['match'] else 'DIFFERENT'} |")
    for r in reads:
        if not r["match"]:
            lines += ["", f"### read {r['label']}: differs ({r['note']})", "", "```",
                      "old: " + json.dumps(r["old"], ensure_ascii=False)[:1500],
                      "new: " + json.dumps(r["new"], ensure_ascii=False)[:1500], "```"]
    text = "\n".join(lines) + "\n"
    out.write_text(text)
    return text


# ------------------------------------------------------------------------------ main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--old-tools-dir", default=os.environ.get(
        "PARITY_OLD_TOOLS", str(Path.home() / "ghq/github.com/ken109/monorepo/tools")),
        help="directory holding the old linear.ts and linear-auth.ts (read only)")
    ap.add_argument("--linear-bin", default=os.environ.get("PARITY_LINEAR_BIN", ""),
                    help="the Rust binary (default: target/debug/linear of this checkout)")
    ap.add_argument("--report", default="", help="write the markdown report here (default: temp dir)")
    ap.add_argument("--min-remaining", type=int, default=2100,
                    help="do not start with fewer API requests left than this (a run uses ~1900)")
    ap.add_argument("--keep", action="store_true", help="do not clean up the sandbox at the end")
    ap.add_argument("--cleanup-only", action="store_true",
                    help="cancel and archive every leftover parity-* project/template and exit")
    args = ap.parse_args()

    key = os.environ.get("LINEAR_API_KEY_SANDBOX", "")
    if not key:
        print("LINEAR_API_KEY_SANDBOX is not set (set -a; . ~/.config/linear-dev/sandbox.env; set +a)",
              file=sys.stderr)
        return 2
    gql = Linear(key)
    org = gql.q("query { organization { urlKey } teams { nodes { key } } }")
    if org["organization"]["urlKey"] != SANDBOX_URL_KEY or SANDBOX_TEAM not in [
            t["key"] for t in org["teams"]["nodes"]]:
        print("refusing to run: the key does not belong to the sandbox workspace", file=sys.stderr)
        return 2

    if args.cleanup_only:
        print("cleanup:", cleanup(gql))
        return 0
    print(f"requests left in this hour for the sandbox key: {Linear.remaining}")
    if Linear.remaining is not None and Linear.remaining < args.min_remaining:
        print(f"refusing to start: a run needs about {args.min_remaining} requests (--min-remaining); "
              "wait for the hourly limit to refill", file=sys.stderr)
        return 2

    repo = Path(__file__).resolve().parents[2]
    binary = args.linear_bin or str(repo / "target/debug/linear")
    if not Path(binary).exists():
        print(f"{binary} not found: run `cargo build` first (or pass --linear-bin)", file=sys.stderr)
        return 2
    if shutil.which("bun") is None:
        print("bun is needed to run the old tool", file=sys.stderr)
        return 2

    stamp = time.strftime("%Y%m%d%H%M%S")
    work = Path(tempfile.mkdtemp(prefix="parity-"))
    files = write_files(work)
    tools = {"old": OldTool(Path(args.old_tools_dir), key, work), "new": NewTool(binary, key, work)}

    # The old tool must see the sandbox too; checked with its own read-only command.
    who = tools["old"].run(["whoami"])
    if f"({SANDBOX_URL_KEY})" not in who.out:
        print(f"refusing to run: the old tool does not report the sandbox: {who.out!r}", file=sys.stderr)
        return 2
    who = tools["new"].run(["workspace", "whoami"])
    if who.code != 0 or SANDBOX_URL_KEY not in who.out:
        print(f"refusing to run: the Rust CLI does not report the sandbox: {who.err!r}", file=sys.stderr)
        return 2

    tags = {k: f"parity-{stamp}-{k}" for k in tools}
    obs = {k: Observer(gql, tags[k]) for k in tools}
    drivers: dict[str, Driver] = {}
    reads: list[dict] = []
    try:
        for kind in ("old", "new"):
            print(f"\n== {kind}: scenario in {tags[kind]}*", flush=True)
            drivers[kind] = Driver(tools[kind], obs[kind], gql, files, tags[kind])
            scenario(drivers[kind])
        print("\n== reads", flush=True)
        reads = read_phase(drivers, gql)
    finally:
        if not args.keep:
            try:
                print("\n== cleanup:", cleanup(gql, tags=list(tags.values())), flush=True)
            except Exception as e:  # noqa: BLE001 - keep the scenario's own error visible
                print(f"\n== cleanup FAILED ({e}); run scripts/parity/run.sh --cleanup-only", flush=True)
        else:
            print("\n== left in place (--keep); run with --cleanup-only to remove")

    out = Path(args.report) if args.report else work / "report.md"
    text = report(drivers["old"].steps, drivers["new"].steps, obs, reads, out)
    print(f"\nreport: {out}")
    print(f"(the harness itself sent {Linear.requests} GraphQL requests; the tools sent more)")
    n_diff = sum(1 for a, b in zip(drivers["old"].steps, drivers["new"].steps) if classify(a, b)[0] == "DIFF")
    print(f"{len(drivers['old'].steps)} steps, {n_diff} differ in outcome; "
          f"{sum(1 for r in reads if not r['match'])} of {len(reads)} reads differ")
    return 0


if __name__ == "__main__":
    sys.exit(main())
