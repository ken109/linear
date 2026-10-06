#!/usr/bin/env python3
"""Read-only comparison on a REAL workspace: the old tool's read commands against their
`linear` equivalents. Nothing is written to the workspace.

  scripts/parity/read_compare.py --workspace ken109
  scripts/parity/read_compare.py --workspace lt-three

How it gets credentials without printing them:
  * The old tool is run with its own stored OAuth token (`~/.config/ken109/linear-token.json`,
    `~/.config/lt-three/linear.json`); its `whoami` runs first so an expired token is refreshed
    the way the tool itself would.
  * The same access token is handed to `linear` for this process only, as the environment
    variable `LINEAR_API_KEY_REAL` (value `Bearer <token>`). The CLI sends an API key as the raw
    Authorization header, so a Bearer value works. Nothing is stored; outputs are checked not to
    contain the token.
Both sides are restricted to an allowlist of read commands; anything else is refused.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from parity import Result, Tool  # noqa: E402

HOME = Path.home()
WORKSPACES = {
    "ken109": {
        "tools": HOME / "ghq/github.com/ken109/monorepo/tools",
        "token": HOME / ".config/ken109/linear-token.json",
        "team": "KK",
    },
    "lt-three": {
        "tools": HOME / "ghq/github.com/lt-three/monorepo/tools",
        "token": HOME / ".config/lt-three/linear.json",
        "team": "LT3",
    },
}
OLD_READS = {"whoami", "projects", "initiatives", "templates", "skeletons", "template", "milestones",
             "issues", "status", "open-issues", "cycle"}
NEW_READS = {("workspace", "whoami"), ("project", "list"), ("project", "view"), ("issue", "list"),
             ("issue", "view"), ("milestone", "list"), ("milestone", "view"), ("initiative", "list"),
             ("initiative", "view"), ("template", "list"), ("template", "view"),
             ("template", "skeleton"), ("label", "list"), ("team", "list")}
CLEAN_ENV = ("LINEAR_", "KEN109_")


class Readers:
    def __init__(self, ws: str, work: Path):
        cfg = WORKSPACES[ws]
        self.tools = cfg["tools"]
        if not (self.tools / "linear.ts").exists():
            raise SystemExit(f"old tool not found in {self.tools}")
        self.base = {k: v for k, v in os.environ.items() if not k.startswith(CLEAN_ENV)}
        self.old_whoami = self._old(["whoami"])
        m = re.search(r"\(([^()\s]+)\)", self.old_whoami.out)
        if self.old_whoami.code != 0 or not m:
            raise SystemExit(f"the old tool is not logged in to {ws}: {self.old_whoami.err.strip()[:200]}")
        self.url_key = m.group(1)
        stored = json.loads(cfg["token"].read_text())  # refreshed by the whoami above if needed
        self._token = stored["access_token"]
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(HOME),
            "LINEAR_CONFIG_DIR": str(work / "cfg"),
            "LINEAR_CACHE_DIR": str(work / "cache"),
            "LINEAR_API_KEY_REAL": f"Bearer {self._token}",
            "LINEAR_WORKSPACE": "real",
        }
        self.new_env = env
        exe = os.environ.get("PARITY_LINEAR_BIN") or str(
            Path(__file__).resolve().parents[2] / "target/debug/linear")
        self.exe = exe
        r = Tool._exec([exe, "workspace", "add", "real", "--url-key", self.url_key, "--team",
                        cfg["team"], "--default"], env, None)
        if r.code != 0:
            raise SystemExit(f"cannot configure the CLI: {r.err}")

    def _scrub(self, r: Result) -> Result:
        if self._token in r.out or self._token in r.err:
            raise SystemExit("a token appeared in command output; aborting")
        return r

    def _old(self, args: list[str]) -> Result:
        if args[0] not in OLD_READS:
            raise SystemExit(f"refusing a non-read command of the old tool: {args[0]}")
        env = dict(self.base)
        return Tool._exec(["bun", str(self.tools / "linear.ts"), *args], env, None)

    def old(self, *args: str) -> Result:
        return self._scrub(self._old(list(args)))

    def new(self, *args: str, as_json: bool = True) -> Result:
        if tuple(args[:2]) not in NEW_READS:
            raise SystemExit(f"refusing a non-read command of linear: {args[:2]}")
        cmd = [self.exe, *args] + (["--json"] if as_json else [])
        return self._scrub(Tool._exec(cmd, self.new_env, None))


def jlist(r: Result) -> list:
    if r.code != 0 or not isinstance(r.json, list):
        raise RuntimeError(f"exit {r.code}: {r.err.strip()[:200]}")
    return r.json


def diff_sets(label: str, old: dict, new: dict, fields: list[str], out: list[str]) -> bool:
    """Compare two {key: {field: value}} maps; append findings to out. True when identical."""
    same = True
    only_old = sorted(set(old) - set(new))
    only_new = sorted(set(new) - set(old))
    if only_old:
        same = False
        out.append(f"- {label}: only in the old tool ({len(only_old)}): " + ", ".join(
            f"{k} ({old[k].get('name') or old[k].get('title') or ''})" for k in only_old[:15]))
    if only_new:
        same = False
        out.append(f"- {label}: only in linear ({len(only_new)}): " + ", ".join(
            f"{k} ({new[k].get('name') or new[k].get('title') or ''})" for k in only_new[:15]))
    for k in sorted(set(old) & set(new)):
        for f in fields:
            if old[k].get(f) != new[k].get(f):
                same = False
                out.append(f"- {label} {k} ({old[k].get('name') or old[k].get('title')}): field `{f}` "
                           f"old={old[k].get(f)!r} new={new[k].get(f)!r}")
    return same


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--workspace", required=True, choices=sorted(WORKSPACES))
    ap.add_argument("--report", default="", help="write the markdown report here")
    ap.add_argument("--max-projects", type=int, default=40,
                    help="per-project comparisons (milestones, issues) stop after this many projects")
    args = ap.parse_args()

    work = Path(tempfile.mkdtemp(prefix="parity-read-"))
    rd = Readers(args.workspace, work)
    ws = args.workspace
    lines = [f"# Read comparison: {ws} (`{rd.url_key}`)", ""]
    summary: list[tuple[str, str]] = []

    def section(name: str, fn) -> None:
        out: list[str] = []
        try:
            same = fn(out)
        except Exception as e:  # noqa: BLE001 - a failed comparison is a finding, not a crash
            same, out = False, [f"- could not compare: {e}"]
        summary.append((name, "same" if same else "DIFFERENT"))
        lines.append(f"## {name}: {'same' if same else 'DIFFERENT'}")
        lines.append("")
        lines.extend(out or ["(identical)"])
        lines.append("")
        print(f"  {name:<14} {'same' if same else 'DIFFERENT'}", flush=True)

    # ---- projects (open ones)
    old_projects = json.loads(rd.old("projects").out)
    new_projects = jlist(rd.new("project", "list", "--open", "--all"))

    def projects(out):
        # lt-three's `projects` has no `status` field (it filters on the deprecated `state`)
        fields = ["name", "targetDate", "url", "initiative"] + (
            ["status"] if old_projects and "status" in old_projects[0] else [])
        o = {p["id"]: {"name": p["name"], "status": p.get("status"), "targetDate": p["targetDate"],
                       "url": p["url"], "initiative": p["initiative"]} for p in old_projects}
        n = {p["id"]: {"name": p["name"], "status": p["status"]["name"], "targetDate": p["targetDate"],
                       "url": p["url"],
                       "initiative": (p["initiatives"]["nodes"] or [{}])[0].get("name")}
             for p in new_projects}
        out.append(f"{len(o)} open projects in the old tool, {len(n)} in linear.")
        return diff_sets("project", o, n, fields, out)

    section("projects", projects)

    # ---- initiatives
    def initiatives(out):
        o = {i["id"]: {"name": i["name"], "description": i["description"]}
             for i in json.loads(rd.old("initiatives").out)}
        rows = jlist(rd.new("initiative", "list", "--all"))
        if ws == "lt-three":  # that tool leaves out Completed initiatives
            rows = [r for r in rows if r["status"] != "Completed"]
        n = {i["id"]: {"name": i["name"]} for i in rows}
        out.append(f"{len(o)} initiatives in the old tool, {len(n)} in linear.")
        same = diff_sets("initiative", o, n, ["name"], out)
        if rows and "description" not in rows[0]:
            out.append("- `initiative list --json` carries no `description` (the old `initiatives` does)")
            same = False
        return same

    section("initiatives", initiatives)

    # ---- templates
    def templates(out):
        o = {t["id"]: {"name": t["name"], "type": t["type"], "description": t["description"]}
             for t in json.loads(rd.old("templates").out)}
        n = {t["id"]: {"name": t["name"], "type": t["type"], "description": t["description"]}
             for t in jlist(rd.new("template", "list"))}
        out.append(f"{len(o)} templates in the old tool, {len(n)} in linear.")
        return diff_sets("template", o, n, ["name", "type", "description"], out)

    section("templates", templates)

    # ---- skeletons
    def skeletons(out):
        o = rd.old("skeletons").out.strip()
        n = rd.new("template", "skeleton", as_json=False).out.strip()
        norm = lambda s: re.sub(r"\n{3,}", "\n\n", s).strip()  # noqa: E731
        if norm(o) == norm(n):
            out.append(f"{len(o.splitlines())} lines, identical (blank-line runs collapsed).")
            return True
        import difflib
        out.append("```diff")
        out.extend(list(difflib.unified_diff(norm(o).splitlines(), norm(n).splitlines(), "old", "new",
                                              lineterm="", n=1))[:40])
        out.append("```")
        return False

    section("skeletons", skeletons)

    def skeleton_one(out):
        names = [t["name"] for t in jlist(rd.new("template", "list", "--type", "issue"))][:12]
        same = True
        for name in names:
            o = rd.old("template", name).out.strip()
            n = rd.new("template", "skeleton", name, as_json=False).out.strip()
            if o != n:
                same = False
                out.append(f"- `{name}`: old={o[:120]!r} new={n[:120]!r}")
        out.append(f"{len(names)} issue templates compared one by one.")
        return same

    section("template <name>", skeleton_one)

    # ---- per-project: milestones and open issues
    ids = [p["id"] for p in old_projects][: args.max_projects]

    def milestones(out):
        same = True
        for pid in ids:
            o = {m["id"]: {"name": m["name"], "targetDate": m["targetDate"]}
                 for m in json.loads(rd.old("milestones", "--project", pid).out)}
            n = {m["id"]: {"name": m["name"], "targetDate": m["targetDate"]}
                 for m in jlist(rd.new("milestone", "list", "--project", pid))}
            same &= diff_sets(f"milestones of {pid[:8]}", o, n, ["name", "targetDate"], out)
        out.append(f"{len(ids)} open projects compared.")
        return same

    section("milestones", milestones)

    def issues(out):
        same = True
        total = 0
        for pid in ids:
            old_rows = json.loads(rd.old("issues", "--project", pid).out)
            new_rows = jlist(rd.new("issue", "list", "--project", pid, "--open", "--all"))
            total += len(old_rows)
            o = {r["identifier"]: {"title": r["title"], "state": r["state"], "milestone": r["milestone"]}
                 for r in old_rows}
            n = {r["identifier"]: {"title": r["title"], "state": r["state"]["name"],
                                   "milestone": (r["projectMilestone"] or {}).get("name")}
                 for r in new_rows}
            same &= diff_sets(f"issues of {pid[:8]}", o, n, ["title", "state", "milestone"], out)
            o_order = [r["identifier"] for r in old_rows]
            n_order = [r["identifier"] for r in new_rows]
            if set(o_order) == set(n_order) and o_order != n_order:
                same = False
                out.append(f"- issues of {pid[:8]}: same set, different order (old sortOrder asc vs "
                           f"linear's default); first old={o_order[:4]} new={n_order[:4]}")
        out.append(f"{total} open issues in {len(ids)} projects (old tool).")
        return same

    if ws == "ken109":  # lt-three's tool has no `issues` command
        section("issues", issues)

    # ---- status (current location)
    def status(out):
        text = rd.old("status").out
        shown = re.findall(r"`([0-9a-f]{12})`", text)
        by_slug = {p["slugId"]: p for p in new_projects}
        # the old tool's rule: started projects, plus any with a status update
        expected = {s for s, p in by_slug.items()
                    if p["lastUpdate"] is not None or p["status"]["type"] == "started"}
        same = True
        if set(shown) != expected:
            same = False
            out.append(f"- the old `status` shows {len(shown)} projects; the same rule over `project list` "
                       f"gives {len(expected)}: missing={sorted(set(shown) - expected)} "
                       f"extra={sorted(expected - set(shown))}")
        labels = {"onTrack": "順調", "atRisk": "注意", "offTrack": "遅れ"}
        for block in re.split(r"\n(?=- \*\*)", text):
            m = re.search(r"`([0-9a-f]{12})`", block)
            if not m or m.group(1) not in by_slug:
                continue
            p = by_slug[m.group(1)]
            lu = p["lastUpdate"]
            if lu is not None and labels[lu["health"]] not in block:
                same = False
                out.append(f"- {p['name']}: health {lu['health']} not shown the same way")
            if lu is None and "未記入" not in block:
                same = False
                out.append(f"- {p['name']}: old says it has an update; linear has none")
        out.append(f"{len(shown)} projects in the old `status`; text compared on slug, health and "
                   "'not written' marks only (it is prose, not JSON).")
        return same

    section("status", status)

    def status_one(out):
        same = True
        checked = 0
        for p in new_projects:
            if p["lastUpdate"] is None:
                continue
            old_text = rd.old("status", "--project", p["id"]).out
            v = rd.new("project", "view", p["id"])
            view = v.json
            checked += 1
            if view["lastUpdate"]["body"].strip() not in old_text:
                same = False
                out.append(f"- {p['name']}: latest update body differs")
            ms_old = re.findall(r"^- \[[ x]\] (.+)（[^（]*）$", old_text, re.M)
            ms_new = [m["name"] for m in view["projectMilestones"]["nodes"]]
            if sorted(ms_old) != sorted(ms_new):
                same = False
                out.append(f"- {p['name']}: milestones old={ms_old} new={ms_new}")
            older_old = len(re.findall(r"^- \d{4}-\d{2}-\d{2}（", old_text, re.M))
            older_new = max(len(view["projectUpdates"]["nodes"]) - 1, 0)
            if older_old != older_new:
                out.append(f"- {p['name']}: earlier updates shown old={older_old} new={older_new} "
                           "(old caps at 3, linear at 5)")
            if checked >= 12:
                break
        out.append(f"{checked} projects with a status update compared.")
        return same

    section("status --project", status_one)

    # ---- lt-three only
    if ws == "lt-three":
        def open_issues(out):
            o = {r["identifier"]: {"title": r["title"], "state": r["state"]["name"]
                                   if isinstance(r["state"], dict) else r["state"]}
                 for r in json.loads(rd.old("open-issues").out)}
            n_rows = jlist(rd.new("issue", "list", "--open", "--all"))
            n = {r["identifier"]: {"title": r["title"], "state": r["state"]["name"]} for r in n_rows}
            out.append(f"old `open-issues` (open + recently closed): {len(o)}; "
                       f"`issue list --open --all`: {len(n)}.")
            only_old = sorted(set(o) - set(n))
            only_new = sorted(set(n) - set(o))
            if only_old:
                out.append(f"- only in the old tool ({len(only_old)}; recently closed ones have no "
                           f"`linear` filter): {', '.join(only_old[:20])}")
            if only_new:
                out.append(f"- only in linear ({len(only_new)}): {', '.join(only_new[:20])}")
            return not only_old and not only_new

        section("open-issues", open_issues)

    who = rd.new("workspace", "whoami", as_json=False)
    lines.append("## whoami")
    lines.append("")
    lines.append("old: " + " / ".join(rd.old_whoami.out.strip().splitlines()))
    lines.append("")
    lines.append("linear: " + " / ".join(who.out.strip().splitlines()))
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    lines += [f"- {n}: {s}" for n, s in summary]
    text = "\n".join(lines) + "\n"
    if args.report:
        Path(args.report).write_text(text)
        print(f"report: {args.report}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
