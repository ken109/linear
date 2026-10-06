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
| `crates/wasm`   | `linear-wasm` | JSON-in/JSON-out wrapper over the core (skeleton).          |
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
- `crates/cli/tests/live.rs` talks to a real workspace and is ignored by default:

  ```sh
  LINEAR_API_KEY_SANDBOX=... cargo test -p linear --test live -- --ignored
  ```

## License

MIT
