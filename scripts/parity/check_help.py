#!/usr/bin/env python3
"""Check that every `linear ...` command and `--flag` named in docs/parity.md exists.

A code span that starts with `linear ` is read as: subcommand words, then arguments/flags. The
subcommand path is the leading run of lowercase words (`project status-update`); every
`--flag` that follows must appear in that subcommand's `--help` (or be a global option).
Exit 1 when something named in the document is not there.

    scripts/parity/check_help.py [docs/parity.md]
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

repo = Path(__file__).resolve().parents[2]
doc = Path(sys.argv[1]) if len(sys.argv) > 1 else repo / "docs/parity.md"
binary = os.environ.get("PARITY_LINEAR_BIN") or str(repo / "target/debug/linear")

cache: dict[tuple[str, ...], str | None] = {}


def help_of(path: tuple[str, ...]) -> str | None:
    if path not in cache:
        p = subprocess.run([binary, *path, "--help"], capture_output=True, text=True)
        cache[path] = p.stdout if p.returncode == 0 else None
    return cache[path]


checked = 0
bad: list[str] = []
seen: set[str] = set()
for span in re.findall(r"`(linear [^`]+)`", doc.read_text()):
    if span in seen:
        continue
    seen.add(span)
    words = span.split()[1:]
    path: list[str] = []
    for w in words:
        if re.fullmatch(r"[a-z][a-z-]*", w):
            path.append(w)
        else:
            break
    # a trailing word after the subcommands may be a positional value such as a name; try
    # the longest path whose --help exists
    while path and help_of(tuple(path)) is None:
        path.pop()
    if not path:
        top = help_of(())
        flags = re.findall(r"(?<![\w-])(--[a-z][a-z-]*)", span)
        if flags and top and all(re.search(rf"(?<![\w-]){re.escape(f)}(?![\w-])", top) for f in flags):
            checked += len(flags)  # a global option such as `linear --json`
        else:
            bad.append(f"{span}: no such command")
        continue
    h = help_of(tuple(path)) or ""
    for flag in re.findall(r"(?<![\w-])(--[a-z][a-z-]*)", span):
        checked += 1
        if flag in ("--json", "--quiet", "--workspace", "--help", "--version"):
            continue
        if not re.search(rf"(?<![\w-]){re.escape(flag)}(?![\w-])", h):
            bad.append(f"{span}: `linear {' '.join(path)}` has no {flag}")
    checked += 1

for b in bad:
    print("MISSING:", b)
print(f"{len(seen)} command spans, {checked} commands/flags checked, {len(bad)} missing")
sys.exit(1 if bad else 0)
