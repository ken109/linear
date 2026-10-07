---
name: linear
description: Read and write Linear (issues, projects, milestones, initiatives, documents, comments) through the `linear` command-line client. Use when asked to look something up in Linear, file or update an issue, comment, change a project, or check what is in progress. Start with `linear usage`.
---

# Using the `linear` CLI

`linear` is a command-line client for Linear with safe writes: the CLI itself refuses a write that
breaks the ownership rules or the workspace's validators. Do not work around a refusal; report it.

## Start here

```sh
linear usage            # every command group, global flags, exit codes and the safety model (< 1000 tokens)
linear issue usage      # one group in detail: each command, its flags, whether it reads or writes
```

`linear usage` and `linear <group> usage` are generated from the command definition, so they are
always current. Read them before guessing a flag; use `linear <command> --help` for the full text of
one command. If `linear` is not installed or has no credentials (exit 3), say so and stop: the setup
is in the README (`linear workspace login`).

## Reading

- Add `--json` for data you will process; the keys are Linear's own, in camelCase.
- Cut the output down with `--fields id,identifier,title` (a list or a view, with `--json`) or
  `--id-only` (one uuid per line). A full `issue list --json` carries every description and is large.
- Lists stop at `--limit` (default 50) with a note on stderr; pass `--all` when you need every row.
- Pick the workspace with `-w NAME` when the machine has several.

## Writing safely

1. Read first. Look the issue or project up (`issue view KK-12`, `project view <name>`) and make
   sure it is the one meant. Names are resolved before anything is sent; an unknown or ambiguous
   name is exit 2 with the candidates, so pick one from the list rather than guessing again.
2. If the command lists `--dry-run` in `linear <group> usage`, run it with `--dry-run` first and read
   what it would do. Run the real write only when that is what was asked for.
3. Run the write once. Writes are never retried automatically, and several of them are safe to run
   again (`issue create` with the same `--source` URL returns the existing issue), but do not loop
   on a failure: read the error.
4. A command marked `needs --yes` cannot be undone. Add `--yes` only when the user asked for that
   deletion; without it the command sends nothing and exits 2.

## Exit codes

| Code | Meaning | What to do |
| ---- | ------- | ---------- |
| 0 | success | |
| 1 | general error (network, API) | read the message; retry a read, not a write |
| 2 | usage error | fix the flags or the name; the message lists the valid ones |
| 3 | credentials missing or rejected | stop and ask the user to log in |
| 4 | the ownership rules refused the write | the issue or project is not yours; do not retry or work around it |
| 5 | a validator refused the write | the message says what is missing (a template section, a source URL, a label); fix it |
| 6 | `audit --fail-on` found actionable items | expected from `linear audit --fail-on actionable` |

With `--json`, an error is `{"error":{"code","message"}}` on stderr.

## More

The README has the full reference: configuration (`workspaces.toml`), the ownership and validator
rules, the cache (`--cached`), `linear audit`, `linear brief` and the raw `linear api`.
