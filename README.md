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

## Install

```sh
brew install ken109/tap/linear
# or, from source (needs a Rust toolchain, 1.85 or newer):
cargo install --git https://github.com/ken109/linear linear
```

Release archives for macOS (arm64, x86_64) and Linux (x86_64, static musl build) are attached
to each [GitHub Release](https://github.com/ken109/linear/releases) as
`linear-<version>-<target>.tar.gz` with a `.sha256` file beside each.

While this repository is private, release assets cannot be downloaded without a token, and the
Homebrew formula does not send one, so `brew install` works once the repository is public.
Until then use `cargo install --git` (with git credentials that can read the repository) or
`gh release download`.

`linear --version` prints the workspace version in `Cargo.toml` (`[workspace.package] version`,
inherited by `crates/cli`), so it equals the release tag without the leading `v`.

### Releasing

1. Set the same new version in `Cargo.toml` (`[workspace.package] version`) and in
   `packages/linear-wasm/package.json`, refresh `Cargo.lock` (`cargo check`), and merge to `main`.
2. Tag that commit and push the tag:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

3. The `Release` workflow (`.github/workflows/release.yml`) fails unless the tag equals both
   versions; it never bumps anything. It then builds the three binaries, builds the wasm npm
   tarball (`ken109-linear-wasm-<version>.tgz`), attaches everything with checksums to a GitHub
   Release, and commits a regenerated `Formula/linear.rb` to `ken109/homebrew-tap`. A tag with a
   pre-release suffix (`v1.0.0-rc.1`) makes a pre-release and leaves the tap alone.

The tap step needs the repository secret `TAP_GITHUB_TOKEN`: a token that can push to
`ken109/homebrew-tap` (for a fine-grained token, `Contents: read and write` on that repository).

To rehearse, run the workflow by hand with `dry_run` on (the default) from the Actions tab or
`gh workflow run release.yml -f dry_run=true`. It builds and packages everything and uploads the
archives, the wasm tarball and the generated formula to the run, without creating a Release or
touching the tap. The formula is rendered from `packaging/linear.rb.in` by
`scripts/release/formula.sh`.

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

### Reading from the cache: `--cached`

```sh
linear issue list --cached [--ttl 300]       # my issues In Progress, from the last refresh
linear issue view KK-12 --cached
linear project list --cached                 # the projects of those issues
linear project view my-project --cached      # id, slug, URL or name, among those projects
linear audit --cached
```

`--cached` reads the file and nothing else: no credentials, no request. It never falls back to
Linear, and it never serves a snapshot it cannot vouch for. When the workspace has no snapshot,
the snapshot is older than `--ttl` (default 300 seconds), or the file is unusable (another
`schema_version`, another workspace's entry, not valid JSON), the command prints nothing on stdout,
says why on stderr and exits 1; run `linear cache refresh`. A snapshot that a failed refresh could
not renew is served only while it is within the TTL. A successful read says on stderr how old the
snapshot is (not with `--quiet`); stdout has the same shape as a live read.

Only what the snapshot holds can be read, so these commands qualify and no others:

- `issue list`: the issues assigned to you that are In Progress (state type `started`). Filters
  that would select something else are a usage error (exit 2): `--assignee` other than `me`,
  `--state-type` other than `started`, `--state`, `--open`, `--team`, `--project`, `--milestone`,
  `--label`, `--source-url`. `--limit` and `--all` work.
- `issue view`: one of those issues, by identifier or id. The priority label and the comments are
  not in the snapshot, so they are absent (not empty). An issue outside the snapshot exits 1.
- `project list`: the projects those issues belong to (not every project you lead). No filters:
  `--lead`, `--status-type`, `--open` and `--initiative` are a usage error.
- `project view`: one of those projects, matched like a live reference. The description, status
  update history and `--content` are not in the snapshot (`--content` is a usage error).
- `audit --cached`: the findings of the last refresh (see [Audit](#audit)).

The other reads (`milestone`, `initiative`, `template`, `label`, `team`, `user`) are not in the
snapshot and have no `--cached`.

### `linear status`

One line for a statusline or a SessionStart hook. It reads the cache and nothing else, so it
returns at once and never touches the network:

```console
$ linear status
main: 3 in progress, 1 actionable
$ linear status                  # the snapshot is older than the TTL
main: unknown (snapshot 2h old)
$ linear status                  # two workspaces, one never refreshed
main: 3 in progress, 1 actionable | work: unknown (nothing cached)
```

`actionable` is the number of audit findings about things you own. Past `--ttl` (default 300
seconds), with no snapshot, or with an unusable file, the workspace reads `unknown (...)` and no
numbers are shown: old figures are never printed as current. A refresh that failed while the
snapshot is still within the TTL adds ` (last refresh failed)`. Workspaces are joined with ` | `;
`--workspace` (or `LINEAR_WORKSPACE`, or a `.linear.toml`) narrows it to one.

The exit code is 0 whatever the state, because the line itself says it. `--json` prints one object
per workspace for scripts:

```json
[{"workspace": "main", "state": "fresh", "ttl_secs": 300, "fetched_at": "2026-10-07T01:02:03Z",
  "age_secs": 42, "in_progress": 3, "findings": 4, "actionable": 1, "new_findings": 0,
  "refresh_failed": false, "reason": null, "line": "main: 3 in progress, 1 actionable"}]
```

`state` is `fresh`, `expired`, `missing` or `unusable`; the counts and `fetched_at` are `null`
unless it is `fresh`, and `reason` says why when it is not (or what the failed refresh said).
`--quiet` prints `<workspace> <state>` per line. Starting a refresh is the caller's job, for
example `linear cache refresh >/dev/null 2>&1 &` when the line says `unknown`.

## Audit

`linear audit` finds Linear data that has drifted. It works on every configured workspace unless
`--workspace`, `LINEAR_WORKSPACE` or a `.linear.toml` selects one, and prints findings with the
command that fixes each (the fix names the workspace with `-w`). A fix is a command this CLI
really has: a test parses every fix the audit can print with the CLI's own argument definition
(`<state>` and the like stand for a value you choose; `--status <started-status>` is a project
status of that type, by the name your workspace gives it).

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
  [--source-title "Where it came from"] [--meta kind=slack --meta ticket=42] \
  [--milestone M1] [--assignee me] [--label bug] [--team ENG]
linear issue update KK-12 --state "In Progress" --due 2026-11-01 [--milestone M2] [--assignee me]
linear issue update KK-12 --project "Other Project" [--milestone M1]   # the old milestone is cleared
linear issue update KK-12 --body-file body.md [--template "Bug report"]   # replace the description
linear issue update KK-12 --source https://example.com/a [--source-title ..] [--meta kind=slack]
linear issue update KK-12 --labels bug,api                             # exactly these labels
linear issue update KK-12 --add-labels bug --remove-labels triage      # or edit the set
linear issue comment KK-12 --body-file comment.md
linear issue reorder KK-3 KK-1 KK-2                                    # same project; top first
linear issue reorder KK-3,KK-1,KK-2                                    # the same, comma-separated
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
   three times) the issue is deleted again. `issue update` writes the fields first and attaches
   the source last, and puts the fields back if the attachment keeps failing. `issue reorder`
   puts the old values back if a later write fails.

`issue create` with a source URL that is already attached to an issue creates nothing and
returns that issue (`"existing": true` with `--json`), so running it again is safe. (This needs
the `source-attachment` rule; without it the lookup is not made.)

`issue reorder` takes at least two issues, as separate arguments, one comma-separated list, or
a mix; naming one is a usage error (exit 2).

#### `issue update`

`issue update` sends only the fields that differ from what the issue has now, and never `null`
for a field it was not asked to clear. A second identical run sends nothing and says so
(`"changed": []` with `--json`; otherwise `changed` lists what was written: `description`,
`state`, `project`, `milestone`, `dueDate`, `assignee`, `labels`, `source`).

- `--body-file FILE|-` replaces the description (an empty file is a usage error). Trailing
  whitespace does not count as a difference, nor does the bullet Linear rewrites (`- item` is
  stored as `* item`); another rewrite the CLI does not know about costs one redundant write. `--template NAME` (only with `--body-file`) holds the
  new body to that Linear template through the `template-sections` rule; without `--template` no
  template is checked, because an issue does not record which one it came from.
- `--source URL` attaches the URL as the issue's source, with `--source-title` and `--meta
  KEY=VALUE` read exactly as in `issue create` (a number is sent as a number, `str:` forces text).
  An attachment the issue already has with that URL is updated, never duplicated: Linear upserts
  on the URL, so the stored title and subtitle are sent back unless `--source-title` replaces the
  title, and the metadata is replaced as a whole (not merged) when `--meta` is given. A title and
  metadata that are already what is stored send nothing. With the `source-attachment` rule the URL
  must be http(s), `source_kinds` is checked on what is written (a new attachment needs
  `--meta kind=...`), and a URL that **another** issue carries is refused (exit 5). The rule
  applies to `issue update` as well as `issue create` (`rule_operations` can narrow it to
  `issue_create`); an update that names no `--source` is never asked for one.
- `--labels NAME,NAME` (repeatable; `--label` is the same flag, as in `issue create`) **replaces**
  the issue's labels with exactly these. `--add-labels` and `--remove-labels` edit the set
  instead (a label to remove that the issue does not have is ignored); they cannot be combined
  with `--labels`. With `label-groups-exclusive`, the labels the issue would end up with are
  checked, so adding a second label of a single-select group is refused (exit 5) unless the first
  one is removed in the same command.

#### Source metadata

`--meta KEY=VALUE` (repeatable, only with `--source`) puts metadata on the source attachment,
the flat object Linear lets an attachment carry. Values are strings or numbers, nothing nested:

- A value that reads as a number (`42`, `-1`, `3.5`, `1e3`) is sent as a **number**. Write
  `--meta build=str:123` to send the text `"123"` (`str:` is removed; `str:str:1` is the text
  `str:1`). Things that only look like numbers stay text: `007`, `+1`, `.5`, `NaN`, and
  integers too large to hold exactly.
- A key must not be empty or repeated; a pair without `=` is a usage error. The metadata is
  checked before anything is sent.
- `issue view --json` (and `issue list --json`) include each attachment's `metadata`, `subtitle`
  and `sourceType`. Attachments made by integrations may nest their metadata; it is shown as is.

Running `issue create` again with the same `--source` and `--meta` that differs from what the
attachment stores does not create a second issue: it returns the existing one and **replaces**
the attachment's metadata (Linear upserts attachments on their URL, and the stored object is
replaced, not merged), keeping its title unless `--source-title` is given. The output says
`"metadataUpdated": true`. Metadata that is already what is stored (numbers compare by value)
sends nothing, and without `--meta` nothing is read or written.

`issue reorder` rewrites both `sortOrder` (manual order) and `prioritySortOrder` (Linear's
default view); both are shown by `issue list --json`. The issues you name trade the values they
already hold, so issues in between keep their place. Linear may adjust `prioritySortOrder` to
its own liking within a priority, so the exact numbers are not guaranteed, only the order.

### Projects

```sh
linear project create --name "Ship it" [--summary "One line"] [--body-file body.md] \
  [--template "Project"] [--target-date 2026-12-31] [--initiative "Roadmap"] [--lead me] [--team ENG]
linear project update "Ship it" [--name ..] [--summary ..] [--body-file body.md] [--template ..] \
  [--status "In Progress"] [--target-date 2027-01-31] [--initiative "Roadmap"] [--lead me]
linear project status-update "Ship it" --health onTrack|atRisk|offTrack --body-file update.md
linear project reorder "Ship it" "Other" "Third"                       # top first
```

A project may be created or written only when you lead it: `create` with another `--lead`,
and `update`, `status-update` and `reorder` on a project somebody else leads (or nobody does),
are refused with exit 4 before anything is sent. `update --lead` hands a project you lead to
someone else; it cannot take one over.

- `create` returns an unfinished project (not completed or canceled) with the same name instead of
  creating another (`"existing": true`; its lead is left alone). It leads the new project with you
  unless `--lead` says otherwise. `--initiative` puts it under an initiative after creating it; if
  that keeps failing (three tries) the new project is deleted again.
- `update` sends only the fields that differ from what the project has now, and never `null`. A
  second identical run changes nothing (`"changed": []`). `--initiative` only adds a link (it
  never removes the project from another initiative). `--name` is refused when another
  unfinished project has that name. If a later step fails, the fields already written are put
  back. Status names come from the workspace (`In Progress`, `Completed`, ...), ignoring case.
- `status-update` writes a status update (Linear's `ProjectUpdate`): a health and a body. The body
  must not be empty.
- `reorder` works like `issue reorder` on the projects' `sortOrder` and `prioritySortOrder`.

To require a section such as `## Definition of done` in every project body, give a Linear
*project* template that heading, enable the `template-sections` rule and pass `--template`
(see below); no heading is hard-coded in the CLI.

### Validator rules

Choose the rules a workspace enforces in `workspaces.toml`:

```toml
[workspaces.main]
rules = ["template-sections", "source-attachment", "label-groups-exclusive"]
# rule_operations = { "template-sections" = ["issue_create"] }   # narrow a rule to some operations
# source_kinds = ["life-decision", "life-note", "slack", "github"] # see below
```

| Rule                     | What it checks                                                                                     |
| ------------------------ | -------------------------------------------------------------------------------------------------- |
| `template-sections`      | `--template` names a Linear template and the body fills every section (headings read from Linear) |
| `source-attachment`      | `--source` is an http(s) URL; it is attached, and an issue that has it is returned instead (`issue update --source`: upserted on the issue, refused if another issue has it) |
| `label-groups-exclusive` | at most one label from each single-select label group                                              |

`source_kinds` (a workspace key, optional, needs `source-attachment`) makes the rule also check
the kind of the source: `issue create` must pass `--meta kind=<one of the list>`, otherwise it
is refused (exit 5), and `audit` reports an issue that has no http(s) attachment whose
`metadata.kind` is in the list, naming the issue. Unset, metadata is not looked at. An empty
list or an empty kind is a configuration error.

Without a rule, its flag is optional (`--template` with no `template-sections` rule is ignored,
with a note). Whatever the rules, an unknown name, an empty update or comment, a `--source`
that is not an http(s) URL, a reorder across projects, and a workspace whose credentials belong to
another workspace are always refused.

### Milestones, initiatives and templates

```sh
linear milestone create --project "My Project" --name "Design review" --target-date 2026-11-01 \
  [--description-file desc.md]
linear milestone update "Design review" --project "My Project" \
  [--new-name "Review"] [--target-date 2026-11-15] [--description-file desc.md]
linear milestone delete "Review" --project "My Project"

linear initiative create --name "Long effort" [--description-file desc.md]
linear template create --name "Bug report" --body-file body.md [--description "..."] [--team ENG]
```

- A **milestone** belongs to a project, so the ownership rule for projects applies: only the
  project's lead may create, change or delete one (exit 4). A target date is required (without
  one a milestone does not show on the timeline). `create` with a name the project already has
  returns that milestone (`"existing": true` with `--json`); `update` refuses a new name that
  another milestone of the project has (exit 2) and sends only what differs from now; `delete`
  refuses a milestone that still has issues in it (exit 1, naming them), because deleting it
  would silently unfile them.
- An **initiative** with the same name as an existing one is returned instead of creating
  another. (Linear refuses to create initiatives on its free plan; the message is passed on.)
- `template create` makes an **issue template** from a markdown body. Headings are the sections,
  and a body without one is refused. Understood markdown: headings, paragraphs, bullet and
  numbered lists and `**bold**`. A template with the same name returns the existing one. Edit a
  template's sections in Linear's own UI; `linear template skeleton` reads them back.

No validator rule applies to these three (the rules cover issues and projects), and initiatives
and templates are not owned by a project, so only the checks above run.

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

`linear api` is read-only by default. A document that defines a mutation is refused locally
with exit code 4 before anything is sent. A subscription is always a usage error (exit 2).

### Raw mutations

```toml
# ~/.config/linear/workspaces.toml
[workspaces.main]
allow_raw_mutation = true   # default: false
```

```sh
linear api --mutation 'mutation($id: String!) { issueDelete(id: $id) { success } }' --var id=ENG-1
```

A mutation is sent only with **both** `--mutation` and `allow_raw_mutation = true` for the
selected workspace; with either missing it is refused (exit 4, and the message names the
setting). **The ownership rules and the validators do not apply to it.** A free-form document
cannot be read by a machine to find out whose project or issue it changes, so nothing checks
that you own it, that an issue follows its template, or that a source is attached: it can change
anything the API key can. Use the dedicated commands when one exists.

Before sending, the CLI checks that the credentials belong to the workspace the configuration
names (as for every write) and prints a warning to stderr saying that no rule applies. The
warning is left out with `--json` and `--quiet`, so a script that asked for machine output gets
exactly the response. `--mutation` on a document that only has queries runs the query as usual.

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
  requests, rollback, idempotence and exit codes 4 and 5; `structure_write.rs` does the same
  for milestones, initiatives and templates, and `api_mutation.rs` for `linear api --mutation`.
- `crates/cli/tests/live*.rs` talk to a real (sandbox) workspace and are ignored by default;
  `live_write.rs` and `live_attachment_meta.rs` create issues there and cancel them when they are done:

  ```sh
  LINEAR_API_KEY_SANDBOX=... cargo test -p linear --test live_write -- --ignored
  ```

  `live_audit.rs` needs the discrepancies `scripts/seed-sandbox-audit.py` plants (run it once;
  it is idempotent and refuses any workspace but the sandbox).

## License

MIT
