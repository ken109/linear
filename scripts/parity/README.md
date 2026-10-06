# Parity harness (temporary)

Checks that the Rust `linear` CLI does what the old TypeScript tool (`tools/linear.ts` in
ken109/monorepo) did, before that tool is retired. The result of the first run is written up in
[`docs/parity.md`](../../docs/parity.md).

**This directory is temporary.** It exists only for the switch from `tools/linear.ts` to `linear`.
Delete `scripts/parity/` and `docs/parity.md` when the old tool is retired (milestone 6, "retire
`tools/linear.ts`"). Nothing here is part of the product, and nothing in the repository depends on it.

The old tool is never copied into this repository. The harness only reads it from the monorepo
checkout (`--old-tools-dir`, default `~/ghq/github.com/ken109/monorepo/tools`).

## What is in here

| File              | What it does                                                                                      |
| ----------------- | ------------------------------------------------------------------------------------------------- |
| `run.sh`          | Runs the scenarios against the **sandbox** workspace (loads the sandbox key, builds if needed).  |
| `parity.py`       | The scenarios, the old-to-new command mapping, the state read-back and the report.                |
| `read_compare.py` | **Read-only** comparison of the old read commands with `linear`'s on a real workspace.            |
| `check_help.py`   | Checks that every `linear ...` command and `--flag` named in `docs/parity.md` really exists.      |

## Requirements

`python3` (standard library only), `bun` (the old tool), a built `linear` (`cargo build`), and the
sandbox key in `~/.config/linear-dev/sandbox.env` (`LINEAR_API_KEY_SANDBOX`; override the file with
`LINEAR_SANDBOX_ENV`). Never print or commit the key; the harness does neither.

## Scenarios: `scripts/parity/run.sh`

```sh
cargo build
scripts/parity/run.sh --report /tmp/parity-report.md
scripts/parity/run.sh --cleanup-only        # remove leftovers of an interrupted run
```

Takes about 25 minutes: each of the ~90 steps per tool runs a command, then reads the scratch
state back over GraphQL (two requests per step; templates and initiatives only in the steps that
touch them). **Linear allows 2500 requests an hour per key**, and the tools' own requests count, so
a run uses roughly a third of that; do not start it twice in an hour or while the sandbox live tests
run. On a rate limit the harness stops without retrying and says so; then wait, run `--cleanup-only`
and start again.

For each tool in turn it creates its own scratch projects (`parity-<timestamp>-old` / `-new`, plus
`-b` and `-probe` variants) and runs the same steps in them: create / update / status-update of a
project, milestone create / update / delete, issue create (template, source URL, idempotent re-run),
issue update (state, project, milestone, due date, assignee), reorder, comment, template create,
initiative create and linking a project to an initiative, and
for each of those the refusals (empty body, missing template section, bad date, unknown names, ...).
After every step the state is read back with the sandbox key (independent of both tools),
normalised (ids, URLs and the run tag replaced) and compared. The report lists, per step, the exit
codes, whether Linear changed, whether the resulting states match (with a diff when they do not),
the JSON keys each tool printed, the error messages of refused steps, and the read commands side by
side.

How each tool is pointed at the sandbox:

- **old tool**: it reads its key from `KEN109_LINEAR_API_KEY` (set to the sandbox key) and its OAuth
  token file from `KEN109_LINEAR_TOKEN_FILE` (set to a path that does not exist, so it can never
  fall back to the real ken109 workspace). Its team key `KK` is hard-coded and there is no
  environment override, so the harness runs a **temporary copy** of `linear.ts` in which only
  that one line (`const TEAM_KEY = "KK"`) is changed to `SAND`. The original is not touched, and the
  copy is deleted with the temp directory. The harness refuses to run if that line or the API
  endpoint is not exactly as expected.
- **linear**: `LINEAR_CONFIG_DIR` points at a temp directory with a sandbox workspace and the rules
  `template-sections`, `source-attachment`, `label-groups-exclusive`; `template-sections` is limited
  to `issue_create`, because the old tool only checks sections for issues (with the default the Rust
  rule also asks project bodies for a project template).

Safety: before any write the key's organization must be `ken109-sandbox` (and have team `SAND`), the
old tool's `whoami` and `linear workspace whoami` must both report the sandbox, and cleanup only
touches projects, their issues and templates whose name matches `parity-<14 digits>-(old|new)...`.
Seed data (`Fixture Project`, `Sectioned Template`, `audit-seed ...`) is only ever read. At the end
the harness cancels and archives its issues and projects and deletes its templates.

## Real workspaces: `read_compare.py`

```sh
python3 scripts/parity/read_compare.py --workspace ken109
python3 scripts/parity/read_compare.py --workspace lt-three     # needs the lt-three tool's stored login
```

Runs the old read commands (`projects`, `initiatives`, `templates`, `skeletons`, `template`,
`milestones`, `issues`, `status`, `open-issues`) and the `linear` equivalents against the real
workspace and compares the sets of items and the fields. Both sides are restricted to an allowlist of
read commands; the script refuses anything else, so nothing is written.

Credentials are not printed or stored: the old tool uses its own stored login (refreshed by its
`whoami` if needed), and the same access token is passed to `linear` for that process only, as
`LINEAR_API_KEY_REAL="Bearer <token>"` (the CLI sends an API key as the raw `Authorization`
header, so a bearer value works). Output is checked not to contain the token.

## Docs check: `check_help.py`

```sh
python3 scripts/parity/check_help.py        # reads docs/parity.md, runs `linear ... --help`
```
