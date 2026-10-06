# linear

A fast, scriptable command-line client for [Linear](https://linear.app), written in Rust.

> Status: early development. Nothing here is stable yet.

## Goals

- One static binary; no runtime to install.
- Multiple workspaces, switched through a config file (`~/.config/linear/workspaces.toml`).
- Safe writes: ownership rules and declarative validators are enforced by the CLI itself.
- Machine-friendly: `--json` everywhere, stable exit codes.
- The logic lives in an I/O-free core crate, so the same code can run natively (CLI) and in WebAssembly.

## Layout

| Path            | Package       | Role                                                        |
| --------------- | ------------- | ----------------------------------------------------------- |
| `crates/core`   | `linear-core` | Pure logic: types, request building, response parsing.      |
| `crates/cli`    | `linear`      | The `linear` binary: HTTP, config, credentials, output.     |
| `crates/wasm`   | `linear-wasm` | JSON-in/JSON-out wrapper over the core: the six functions a Worker needs. |
| `packages/linear-wasm` | `@ken109/linear-wasm` | The wasm as an npm package, with generated TypeScript types and zod schemas. |
| `schema/`       | -             | Vendored Linear GraphQL SDL used to type-check queries.     |

## Build

```sh
cargo build --workspace
cargo test --workspace
```

## Configuration

```toml
# ~/.config/linear/workspaces.toml
default = "main"

[workspaces.main]
url_key = "my-company"   # linear.app/<url_key>
default_team = "ENG"
auth = "api-key"         # or "oauth" (not implemented yet)
```

The workspace is chosen by, in order: `--workspace`, `LINEAR_WORKSPACE`, a `.linear.toml`
(`workspace = "main"`) found in the current or a parent directory, then `default`.

```sh
linear workspace add main --url-key my-company --team ENG
printf %s "$LINEAR_API_KEY" | linear workspace login main --with-token   # or run it on a terminal to be prompted
linear workspace whoami
```

Credentials are stored in `~/.config/linear/credentials/<workspace>.json` (mode 0600).
`LINEAR_API_KEY_<NAME>` (for example `LINEAR_API_KEY_MAIN`) overrides the stored key.
Tokens are never printed.

## Reading

```sh
linear issue list --assignee me --state-type started        # my issues in progress
linear issue list --source-url https://example.com/a        # find an issue by where it came from
linear issue list --project "My Project" --milestone M1 --open --label bug
linear issue view KK-12

linear project list --open --lead me
linear project view my-project-1a2b3c4d5e6f                 # id, slug, URL or name
linear milestone list --project "My Project"
linear milestone view "M1" --project "My Project"

linear initiative list --status active
linear template list
linear template skeleton "Bug report"                       # the sections, read from Linear
linear label list                                           # groups first, each followed by its labels
linear team list
linear user view me
```

Listings return at most 50 results; `--limit <N>` changes that and `--all` follows every page.
When a listing was cut short, a note goes to stderr (also with `--json` and `--quiet`).
Nothing is filtered unless asked for: use `--open` to leave out completed and canceled work.
`--json` prints Linear's own shape plus a `workspace` field; `--quiet` prints one key per line
(issue identifier, project or initiative slug, milestone or template name, team key, user email).

## Cache

Commands always ask Linear. The cache exists for readers that must not wait for it (a hook, a
statusline) and is only used when asked for (`--cached`); nothing consults it silently.

```sh
linear cache refresh            # fetch and store; every configured workspace unless one is selected
linear cache show [--ttl 300]   # what is stored, how old it is, whether it can be trusted
linear cache clear              # remove every entry, or only the selected workspace's
```

One JSON file per workspace in `$LINEAR_CACHE_DIR`, else `$XDG_CACHE_HOME/linear`, else
`~/.cache/linear` (mode 0600), written to a temporary file and renamed into place. An entry has
`schema_version`, `status` (`ok` or `failed`), `attempted_at`, `fetched_at`, the last `failure`,
and `data`: the viewer ("me"), the viewer's issues In Progress and their projects, the `audit`
result, and the findings that are new since the previous snapshot.

- A failed refresh (no credentials, Linear unreachable, a bad key) keeps the previous snapshot
  and records the failure beside it; it never replaces it with an empty one. `refresh` then names
  the workspaces it could not reach (`unreachable` with `--json`) and exits 1.
- An entry older than the TTL (5 minutes) is unknown, not healthy: `show` reports `expired`, and a
  reader must not display it as current.
- A file written with another `schema_version` is treated as missing and replaced by the next
  refresh.
- Only one refresh per workspace runs at a time, so a statusline that starts one on every render
  does not start a dozen.

## Audit

`linear audit` finds Linear data that has drifted. It works on every configured workspace unless
`--workspace`, `LINEAR_WORKSPACE` or a `.linear.toml` selects one, and prints findings with the
command that fixes each (the fix names the workspace with `-w`).

```sh
linear audit                                    # every rule, every workspace
linear audit --json -w main
linear audit --issues KK-1,KK-2                 # only those issues and the projects they belong to
linear audit --issues KK-1,KK-2 --since 2026-10-06T12:00:00Z   # and report each not updated since then
linear audit --fail-on actionable               # exit code 6 when there is something for you to fix
linear audit --cached                           # the last `linear cache refresh`, not Linear
```

| Rule                      | Finds                                                                                          |
| ------------------------- | ---------------------------------------------------------------------------------------------- |
| `project-state-vs-issues` | a project's status that its issues contradict (completed with open issues, ...)                |
| `overdue`                 | a project, milestone or issue past its target date and still open                              |
| `issue-without-milestone` | an open issue in a project that has milestones, but in none of them                            |
| `project-without-lead`    | an open project nobody leads                                                                   |
| `stale-in-progress`       | an In Progress issue with no update for `stale_days` (7)                                       |
| `status-update-outdated`  | an In Progress project whose latest status update is older than its issues' last state change, or than `status_update_days` (14) |
| `template-sections`, `source-attachment`, `label-groups-exclusive` | the workspace's [validator rules](#validator-rules), applied to open issues. An issue does not record its template, so the template whose sections its body shares most is used; a body that shares none follows none |
| `not-updated-since`       | with `--issues` and `--since`: a named issue not updated since then                            |

A finding is `actionable` when you own its target (you lead the project, or the issue is
assigned to you) and informational otherwise. `--fail-on actionable` is the only thing that makes
findings change the exit code (6). An issue named with `--issues` that no audited workspace has is
listed as `unresolved_issues`, never silently dropped. A workspace that cannot be audited (no
credentials, Linear unreachable) is listed under `failed_workspaces` and the command exits 1, unless
`--fail-on` already exits with 6. `--cached` fails with exit 1 when an entry is missing or past its
TTL: unknown is not clean.

The thresholds are per workspace:

```toml
[workspaces.main.audit]
stale_days = 7
status_update_days = 14
```

Projects list only their first 100 issues, so the audit fetches issues from the issue side: every
open issue plus any updated within `status_update_days` + 1 days (a state change updates the issue),
and the issues named with `--issues` even if they are old and closed.

## Writing

```sh
linear issue create --title "Fix the thing" --project "My Project" \
  --template "Bug report" --body-file body.md --source https://example.com/a \
  [--milestone M1] [--assignee me] [--label bug] [--team ENG]
linear issue update KK-12 --state "In Progress" --due 2026-11-01 [--milestone M2] [--assignee me]
linear issue update KK-12 --project "Other Project" [--milestone M1]   # the old milestone is cleared
linear issue comment KK-12 --body-file comment.md
linear issue reorder KK-3 KK-1 KK-2                                    # same project; top first
```

`--body-file -` reads standard input. Names (states, projects, milestones, labels, users, teams)
are resolved before anything is sent; one that matches nothing, or more than one thing, is a
usage error (exit 2) that lists the candidates. Every write goes through the same steps, and
nothing is sent until the first two have passed:

1. **Ownership rules** (always on; exit 4). A project may be written only when you lead it. An
   issue may be changed (or commented on, or reordered) when it is assigned to you or its project
   is led by you. An issue may be created in a project you lead, or in one somebody else leads
   only if it is assigned to you and you pass `--allow-foreign`. "You" is the viewer of the
   selected workspace, and the credentials must belong to the workspace the configuration names.
2. **Validators** (the `rules` of the workspace; exit 5; all violations are reported together).
3. **The mutation.** A write made of several requests is all or nothing: `issue create` attaches
   the source URL after creating the issue, and if the attachment keeps failing (it is tried
   three times) the issue is deleted again. `issue reorder` puts the old values back if a
   later write fails.

`issue create` with a source URL that is already attached to an issue creates nothing and
returns that issue (`"existing": true` with `--json`), so running it again is safe.

`issue reorder` rewrites both `sortOrder` (manual order) and `prioritySortOrder` (Linear's
default view); both are shown by `issue list --json`. The issues you name trade the values they
already hold, so issues in between keep their place. Linear may adjust `prioritySortOrder` to
its own liking within a priority, so the exact numbers are not guaranteed, only the order.

### Validator rules

Choose the rules a workspace enforces in `workspaces.toml`:

```toml
[workspaces.main]
rules = ["template-sections", "source-attachment", "label-groups-exclusive"]
# rule_operations = { "template-sections" = ["issue_create"] }   # narrow a rule to some operations
```

| Rule                     | What it checks                                                                                     |
| ------------------------ | -------------------------------------------------------------------------------------------------- |
| `template-sections`      | `--template` names a Linear template and the body fills every section (headings read from Linear) |
| `source-attachment`      | `--source` is an http(s) URL; it is attached, and an issue that has it is returned instead         |
| `label-groups-exclusive` | at most one label from each single-select label group                                              |

Without a rule, its flag is optional (`--template` with no `template-sections` rule is ignored,
with a note). Whatever the rules, an unknown name, an empty update or comment, a `--source`
that is not an http(s) URL, a reorder across projects, and a workspace whose credentials belong to
another workspace are always refused.

## Output and exit codes

`--json` prints machine-readable output; errors then go to stderr as
`{"error":{"code","message"}}`. `--quiet` prints only the essential value(s).

| Code | Meaning                                  |
| ---- | ---------------------------------------- |
| 0    | success                                  |
| 1    | general error                            |
| 2    | usage error                              |
| 3    | authentication error                     |
| 4    | write refused by the ownership rules     |
| 5    | write refused by a validator             |
| 6    | `audit --fail-on` found actionable items |

## Raw GraphQL: `linear api`

For anything without a dedicated command, send a query directly. It uses the selected
workspace and its credentials, and prints the response's `data` as JSON.

```sh
linear api '{ viewer { name } }'
linear api 'query($id: String!) { issue(id: $id) { title } }' --var id=ENG-1
linear api --query-file q.graphql --variables-file vars.json -w other
echo '{ teams { nodes { key } } }' | linear api -
```

Variables: `--var KEY=VALUE` (always a string), `--var-json KEY=JSON` (typed),
`--variables-file FILE` (a JSON object; the flags override it).

`linear api` is read-only. A document that defines a mutation is refused locally with exit
code 4 before anything is sent, because the CLI's ownership rules and validators cannot be
applied to free-form documents. `--mutation` is reserved for a future opt-in
(`allow_raw_mutation = true` in the workspace config) and does nothing yet.

## Schema coverage

`schema/linear.graphql` is Linear's published SDL. `crates/core/coverage.toml` classifies every
field of the Query, Mutation and Subscription roots, and of the core entities (Issue, Project,
ProjectMilestone, ProjectUpdate, Initiative, User, Template, Team, IssueLabel, Comment,
Attachment), as `implemented` or `unsupported` (with a reason). For the entities, `implemented`
means the field is part of the `--json` output. `crates/core/tests/coverage.rs` fails when a
field is unclassified, when a classified field no longer exists, and when the manifest
disagrees with what the code selects. So adding a query or a fragment field means updating the
manifest in the same change.

`scripts/update-schema.sh` refreshes the vendored schema. The `update-schema` workflow runs it
weekly and opens a PR when the schema changed; the PR body says whether coverage still passes
and lists the fields to classify.

## Testing

```sh
cargo test --workspace
```

- `crates/core/tests`: fragment parsing and response handling against anonymized real
  responses (`tests/fixtures`), with a fixed `now`.
- `crates/cli/tests`: the binary against a mock HTTP server (`LINEAR_API_URL`) in an isolated
  config directory (`LINEAR_CONFIG_DIR`).
  `tests/issue_write.rs` answers by operation name (`write_support`), to check the order of
  requests, rollback, idempotence and exit codes 4 and 5.
- `crates/cli/tests/live*.rs` talk to a real (sandbox) workspace and are ignored by default;
  `live_write.rs` creates issues there and cancels them when it is done:

  ```sh
  LINEAR_API_KEY_SANDBOX=... cargo test -p linear --test live_write -- --ignored
  ```

  `live_audit.rs` needs the discrepancies `scripts/seed-sandbox-audit.py` plants (run it once;
  it is idempotent and refuses any workspace but the sandbox).

## License

MIT
