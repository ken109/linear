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
