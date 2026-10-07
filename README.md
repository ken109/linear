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

Release archives for macOS (arm64, x86_64), Linux (x86_64, static musl build) and Windows
(x86_64) are attached to each [GitHub Release](https://github.com/ken109/linear/releases) as
`linear-<version>-<target>.tar.gz` (Windows: `.zip`) with a `.sha256` file beside each.

The Homebrew tap ([ken109/homebrew-tap](https://github.com/ken109/homebrew-tap)) picks up new
releases automatically: its bump workflow reads the latest GitHub Release of this repository,
downloads the archives above and commits the formula. It runs every 6 hours; to pick up a release
right away, run `gh workflow run bump.yml --repo ken109/homebrew-tap`.

`linear --version` prints the workspace version in `Cargo.toml` (`[workspace.package] version`,
inherited by `crates/cli`), so it equals the release tag without the leading `v`.

### Shell completions

```sh
linear completions <bash|zsh|fish|elvish|powershell>
```

prints a completion script for the shell. It is generated from the command definition, so it
always matches the commands of the binary you run, and it needs no configuration or
credentials. It completes commands, subcommands and flags; issue ids and project names are not
completed.

```sh
# bash: load it from ~/.bashrc
source <(linear completions bash)
# zsh: a file called _linear in a directory of $fpath (then restart the shell)
linear completions zsh > "${fpath[1]}/_linear"
# fish
linear completions fish > ~/.config/fish/completions/linear.fish
# powershell: add this line to $PROFILE
linear completions powershell | Out-String | Invoke-Expression
```

With Homebrew the formula installs the bash, zsh and fish scripts itself (it runs
`linear completions <shell>` when the formula is installed).

### Windows

Download `linear-<version>-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/ken109/linear/releases/latest), check it against the
`.sha256` file beside it, and put `linear.exe` in a directory on your `PATH`:

```powershell
$v = "<version>"
$zip = "linear-$v-x86_64-pc-windows-msvc.zip"
Invoke-WebRequest "https://github.com/ken109/linear/releases/download/v$v/$zip" -OutFile $zip
(Get-FileHash $zip -Algorithm SHA256).Hash   # compare with the .sha256 file
Expand-Archive $zip -DestinationPath .
```

The binary has no runtime to install (the C runtime is linked in). On Windows the configuration
is `%APPDATA%\linear` (`workspaces.toml`, `credentials\<workspace>.json`) and the cache is
`%LOCALAPPDATA%\linear`, unless `LINEAR_CONFIG_DIR` / `LINEAR_CACHE_DIR` (or `XDG_CONFIG_HOME` /
`XDG_CACHE_HOME`) say otherwise. There is no file mode to set there: the credentials file is
protected by the permissions of your profile folder, which only you and administrators can read
by default, and `linear` does not check it the way it checks the mode (0600) on macOS and Linux.
CI builds, lints and tests the workspace on `windows-latest` as well as on Linux.

### Releasing

Releases are cut by [release-please](https://github.com/googleapis/release-please). Nothing is
tagged or bumped by hand.

1. Commit to `main` with [Conventional Commits](https://www.conventionalcommits.org/)
   (`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, and `!` or a `BREAKING CHANGE:` footer for
   breaking changes). release-please derives the next version and `CHANGELOG.md` from these
   messages, so a message that does not follow the format is left out of the release. While the
   version is below 1.0.0 a `feat:` bumps the minor version and a `fix:` the patch version;
   `chore:`, `test:`, `ci:` and `build:` commits do not appear in the changelog.
2. The `Release Please` workflow (`.github/workflows/release-please.yml`) keeps one release PR
   open against `main`. It sets the new version in `Cargo.toml` (`[workspace.package] version`),
   `Cargo.lock`, `packages/linear-wasm/package.json` and its `package-lock.json`, and adds the
   changelog entry. Review it and merge it.
3. Merging the PR makes release-please tag the commit `vX.Y.Z` and create the GitHub Release with
   the changelog as its notes. The same workflow then calls the `Release` workflow
   (`.github/workflows/release.yml`) with that tag. It fails unless the tag equals both versions;
   it never bumps anything. It builds the three binaries, builds the wasm npm tarball
   (`linear-wasm-<version>.tgz`) and attaches everything with checksums to the Release. A
   version with a pre-release suffix (`1.0.0-rc.1`) is marked as a pre-release.
4. Nothing is pushed to the Homebrew tap from this repository. The tap's bump workflow finds the
   new Release by itself (every 6 hours, or on demand with
   `gh workflow run bump.yml --repo ken109/homebrew-tap`) and commits the formula.

The `Release` workflow is called from `release-please.yml` rather than started by the tag,
because a tag or Release created with the default `GITHUB_TOKEN` does not start other workflows.
A failed release can be re-run from the Actions tab (re-run failed jobs); the upload replaces the
assets.

One-time setup:

- Repository setting "Allow GitHub Actions to create and approve pull requests"
  (Settings > Actions > General). Without it release-please cannot open the release PR.

A PR opened by `GITHUB_TOKEN` does not start other workflows, so `ci.yml` does not run on the
release PR. The `verify-release-pr` job in `release-please.yml` checks that the versions agree
and that `Cargo.lock` is current.

To rehearse the build, run the `Release` workflow by hand with `dry_run` on (the default) from the
Actions tab or `gh workflow run release.yml -f dry_run=true`. It builds and packages everything
from the branch you pick and uploads the archives and the wasm tarball to the run, without
creating a Release.

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
auth = "api-key"         # or "client_credentials" (CI, see below); "oauth" is not implemented yet
ownership = "strict"     # or "lenient": see "Ownership rules" below
# source_title = "出どころ"  # title of a new source attachment without --source-title (default "Source")
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

### OS keyring

By default the credential is a file. A workspace can keep it in the OS keyring instead (the
macOS Keychain, or libsecret on Linux):

```sh
printf %s "$LINEAR_API_KEY" | linear workspace login main --with-token --keyring
linear workspace migrate main            # move a stored credential from the file into the keyring
linear workspace migrate main --to file  # and back
```

`--keyring` (and `migrate`) set `credential_store = "keyring"` for the workspace in
`workspaces.toml`; later logins and every command then use the keyring. The entry is service
`linear-cli`, account `<workspace>`, and holds the same JSON as the file; the file is removed once
the entry is written (`migrate` reads the entry back before it removes anything).
`workspace list` shows `keyring` in the `CREDENTIALS` column.

- **Where there is no keyring, the file is used.** On WSL, a CI runner or a server without a
  Secret Service, `login --keyring` stores the file and says so (and leaves `workspaces.toml`
  alone), and a workspace set to `keyring` reads its `credentials/<workspace>.json` when the
  keyring is not there or has no entry. `migrate` stops and changes nothing.
- On Linux the keyring is reached through libsecret's `secret-tool` (package `libsecret-tools`),
  which has to be installed and needs a running Secret Service (GNOME Keyring, KWallet). The value
  is passed on standard input, never on the command line. `linear` has no extra dependency for it.
- `LINEAR_CREDENTIAL_STORE=file|keyring` overrides `credential_store` for a run, and
  `LINEAR_KEYRING=off` makes the keyring count as missing (a shared `workspaces.toml` on a machine
  without one).
- macOS may ask for permission when a new build of `linear` first reads an entry another build
  wrote; "Always Allow" remembers it.
- A binary built with `--no-default-features` has no keyring support and always falls back to the
  file. The environment override `LINEAR_API_KEY_<NAME>` still comes first in every case.

### Client credentials (CI)

A workspace can act as an **app** instead of a person, through Linear's OAuth client credentials
grant. What an app creates shows the app as its creator, and nobody's personal key sits in CI.

```toml
[workspaces.team]
url_key = "my-team"
auth = "client_credentials"
client_id = "<the OAuth app's client id>"        # public; or set LINEAR_CLIENT_ID
ownership = "lenient"                             # see below
```

```sh
export LINEAR_CLIENT_SECRET=...        # the app's secret: from CI secrets, never a file
linear -w team workspace whoami        # or `workspace login`: checks the grant, stores nothing
```

- **The secret is only read from the environment**: `LINEAR_CLIENT_SECRET_<NAME>` (for example
  `LINEAR_CLIENT_SECRET_TEAM`) for one workspace, else `LINEAR_CLIENT_SECRET`. The client id is
  `LINEAR_CLIENT_ID_<NAME>`, else `LINEAR_CLIENT_ID`, else `client_id` in `workspaces.toml`. Nothing
  is written to `credentials/`.
- On first use a run asks `https://api.linear.app/oauth/token` for a token (grant
  `client_credentials`, scope `read,write,initiative:write`; Linear makes the token an app
  token by itself). It is kept in memory for the run, replaced a minute before it expires, and
  replaced once, with the request sent again, when Linear refuses it. Linear's tokens last 30
  days and cannot be refreshed, so each run simply asks for a new one.
- **Use the same scopes as everybody else using the app.** Linear invalidates an app's tokens
  when a token is requested with different scopes; that is why the scopes are fixed here to the
  ones the app's other users request.
- The secret and the token are never printed: not in errors (a server that echoes the secret
  is masked), not in `workspace list` (it only says `env` or `no client secret`), not in logs.
- "You" in the ownership rules is the user Linear reports for the token, which is the app.
  With `ownership = "strict"` an app can only write what is assigned to it or led by it, which
  is rarely what CI wants; a workspace that files issues for people sets `ownership = "lenient"`.
- `LINEAR_OAUTH_TOKEN_URL` overrides the token endpoint (for tests and proxies), like
  `LINEAR_API_URL` does for the API.

### Time limit and retries

A request to Linear gives up after 30 seconds. Change it with `--timeout <secs>` (any command)
or `LINEAR_TIMEOUT=<secs>`; the flag wins.

A **read** that fails because of a timeout, a broken connection, a 5xx, or a rate limit that ends
within 10 seconds is sent again, at most twice, after 1 s and then 2 s. `LINEAR_RETRIES=<n>` changes
the number (`0` turns retrying off). **A write is never retried**: it may have reached Linear even
though its response was lost, and sending it again could create a second issue. A rate limit that
asks for a longer wait, such as the hourly request limit, is not waited out; the command stops
with the error (exit 1, with the wait Linear asked for). Errors that a second try cannot fix
(authentication, a bad request, a GraphQL error) are never retried.

## Reading

```sh
linear issue list --assignee me --state-type started        # my issues in progress
linear issue list --source-url https://example.com/a        # find an issue by where it came from
linear issue list --project "My Project" --milestone M1 --open --label bug
linear issue list --open --completed-since 14d --all        # open issues, plus the ones closed in the last 14 days
linear issue list --project "My Project" --open --order manual --all   # the screen order of the project
linear issue view KK-12

linear project list --open --lead me
linear project view my-project-1a2b3c4d5e6f                 # id, slug, URL or name
linear milestone list --project "My Project"
linear milestone view "M1" --project "My Project"

linear initiative list --status active                       # --json includes each description
linear initiative status-updates "Long effort"               # its status updates, newest first
linear template list
linear template skeleton "Bug report"                       # the sections, read from Linear
linear template skeleton "Project" --type project           # the same for a project template
linear label list                                           # groups first, each followed by its labels
linear document list --project "My Project"                 # or --initiative, --title
linear document view design-notes-1a2b3c4d5e6f              # id, slug id or URL; prints the body
linear team list
linear user view me
linear brief                                                # where each unfinished project stands
linear cycle 2026-10-05                                     # the cycle that holds the day after that meeting
linear cycle list --state active                            # a team's cycles, newest first
linear cycle view 41                                        # one cycle by number, with its issues
```

Listings return at most 50 results; `--limit <N>` changes that and `--all` follows every page.
When a listing was cut short, a note goes to stderr (also with `--json` and `--quiet`).
Nothing is filtered unless asked for: use `--open` to leave out completed and canceled work.
`--completed-since <Nd|YYYY-MM-DD>` keeps the issues that were completed or canceled at or after
that time: `14d` is 14 days (of 24 hours) back from now, a date means 00:00 UTC that day. Like every
filter it narrows the others, with one exception that is the point of it: next to `--open` the two
are alternatives, so `--open --completed-since 14d` is the open issues **and** the ones closed in
the last 14 days (what a duplicate check needs); on its own it lists just the closed ones;
`--state-type completed --completed-since 14d` is the completed ones only. A bad value is a usage
error (exit 2) before anything is sent.
`--order manual` sorts by `sortOrder`, the order `issue reorder` writes and a project's screen shows
(top first); the default is the order Linear returns. Linear cannot sort by it, so every matching
page is fetched first and `--limit` then keeps the first of the sorted list. The numbers only
compare within one project, so use it with `--project`.
`--json` prints Linear's own shape plus a `workspace` field; `--quiet` prints one key per line
(issue identifier, project or initiative slug, milestone or template name, team key, user email).

## Brief

`linear brief` prints where each unfinished project stands, as markdown: the brief to read when a
session starts. Unlike `linear status` (below) it asks Linear, not the cache.

```console
$ linear brief
## Linear project status (main: projects in progress)

- **Ship the importer** (Platform) `1a2b3c4d5e6f`
  - 2026-10-06 (1 day ago) on track · Alice
    > Stage: parser done, writer in progress
    > Next: wire the retry queue
    > Waiting: nothing
  - Milestones 1/3 done · next: Writer (2026-10-20, 40%)
- **Old research** `9f8e7d6c5b4a`
  - 2026-08-01 (80 days ago, **stale**) at risk · Bob
    > Stage: waiting for the vendor
- **Tidy the backlog** `0a1b2c3d4e5f`
  - No status update

Details: `linear project view <slug>`. **A status update 14 days old or more is stale: check it before relying on it.**
```

A project is shown when it is not completed or canceled and it is In Progress or has a status
update (one still in the backlog that has an update is being worked on; an In Progress project
without one is shown so that a forgotten update is noticed). Newest update first, projects without
one last. For each: the initiative, the health and author of the latest update, its first three
lines (list markers removed, long lines cut), the milestones done and the next one, and `**stale**`
once the update is `--stale-days` old (default: `status_update_days` of the workspace's `[audit]`,
14, the threshold `linear audit` uses). Dates are shown, and ages counted in calendar days, in the
machine's time zone (an update written at 23:00 is "1 day ago" the next morning). `--json` prints the same facts as data (`workspace`, `staleDays`, and per project `health`,
`update` with `ageDays`, `stale` and `preview`, and `milestones`); `--quiet` prints the slug ids.
There is no `--cached`: the cache holds only the projects of your issues In Progress, which is
another set.

`linear brief --session` is for a SessionStart hook, where the output becomes part of the session's
context and a failure must never get in the way of the session starting. It prints nothing in CI
(when the `CI` environment variable is set, checked before anything else), prints nothing and
exits 0 when anything fails (no credentials, offline, a bad key, a Linear error), and gives up after
4 seconds in all, whatever is still running.

```sh
linear brief --session     # a SessionStart hook command
```

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
`schemaVersion`, `status` (`ok` or `failed`), `attemptedAt`, `fetchedAt`, the last `failure`,
and `data`: the viewer ("me"), the viewer's issues In Progress and their projects, the `audit`
result, and the findings that are new since the previous snapshot.

- A failed refresh (no credentials, Linear unreachable, a bad key) keeps the previous snapshot
  and records the failure beside it; it never replaces it with an empty one. `refresh` then names
  the workspaces it could not reach (`unreachable` with `--json`) and exits 1.
- An entry older than the TTL (5 minutes) is unknown, not healthy: `show` reports `expired`, and a
  reader must not display it as current.
- A file written with another `schemaVersion` (or by an older build, which spelled its keys in
  snake_case) is treated as missing and replaced by the next refresh.
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
`schemaVersion`, another workspace's entry, not valid JSON), the command prints nothing on stdout,
says why on stderr and exits 1; run `linear cache refresh`. A snapshot that a failed refresh could
not renew is served only while it is within the TTL. A successful read says on stderr how old the
snapshot is (not with `--quiet`); stdout has the same shape as a live read.

Only what the snapshot holds can be read, so these commands qualify and no others:

- `issue list`: the issues assigned to you that are In Progress (state type `started`). Filters
  that would select something else are a usage error (exit 2): `--assignee` other than `me`,
  `--state-type` other than `started`, `--state`, `--open`, `--team`, `--project`, `--milestone`,
  `--label`, `--source-url`, `--completed-since`. `--limit`, `--all` and `--order` work.
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
[{"workspace": "main", "state": "fresh", "ttlSecs": 300, "fetchedAt": "2026-10-07T01:02:03Z",
  "ageSecs": 42, "inProgress": 3, "findings": 4, "actionable": 1, "newFindings": 0,
  "refreshFailed": false, "reason": null, "line": "main: 3 in progress, 1 actionable"}]
```

`state` is `fresh`, `expired`, `missing` or `unusable`; the counts and `fetchedAt` are `null`
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
| `pr-merged-issue-open`    | an In Progress issue whose GitHub pull request is merged and that has no other one still waiting. Warns only: the pull request never decides the state (see [GitHub](#github)) |
| `pr-open-too-long`        | an issue whose GitHub pull request (not a draft) has been open for `pr_open_days` (14)         |
| `template-sections`, `source-attachment`, `label-groups-exclusive` | the workspace's [validator rules](#validator-rules), applied to open issues. An issue does not record its template, so the template whose sections its body shares most is used; a body that shares none follows none |
| `not-updated-since`       | with `--issues` and `--since`: a named issue not updated since then                            |

A finding is `actionable` when you own its target (you lead the project, or the issue is
assigned to you) and informational otherwise. `--fail-on actionable` is the only thing that makes
findings change the exit code (6). An issue named with `--issues` that no audited workspace has is
listed as `unresolvedIssues`, never silently dropped. A workspace that cannot be audited (no
credentials, Linear unreachable) is listed under `failedWorkspaces` and the command exits 1, unless
`--fail-on` already exits with 6. `--cached` fails with exit 1 when an entry is missing or past its
TTL: unknown is not clean.

The thresholds are per workspace:

```toml
[workspaces.main.audit]
stale_days = 7
status_update_days = 14
pr_open_days = 14
```

Projects list only their first 100 issues, so the audit fetches issues from the issue side: every
open issue plus any updated within `status_update_days` + 1 days (a state change updates the issue),
and the issues named with `--issues` even if they are old and closed.

## Writing

```sh
linear issue create --title "Fix the thing" --project "My Project" \
  --template "Bug report" --body-file body.md --source https://example.com/a \
  [--source-title "Where it came from"] [--meta kind=slack --meta ticket=42] \
  [--milestone M1] [--assignee me] [--label bug] [--team ENG] [--held-on 2026-10-05]
linear issue update KK-12 --state "In Progress" --due 2026-11-01 [--milestone M2] [--assignee me]
linear issue update KK-12 --project "Other Project" [--milestone M1]   # the old milestone is cleared
linear issue update KK-12 --body-file body.md [--template "Bug report"]   # replace the description
linear issue update KK-12 --source https://example.com/a [--source-title ..] [--meta kind=slack]
linear issue update KK-12 --labels bug,api                             # exactly these labels
linear issue update KK-12 --add-labels bug --remove-labels triage      # or edit the set
linear issue comment KK-12 --body-file comment.md
linear issue link-pr KK-12 https://github.com/owner/repo/pull/34      # needs the GitHub integration
linear issue attach-file KK-12 ./shot.png [--title "Login bug"]       # upload a file and attach it
linear issue unlink KK-12 https://example.com/a --yes                  # delete that attachment
linear issue delete KK-12                                              # trash; also: archive, unarchive
linear issue relate KK-12 --blocks KK-13                               # or --related, or --duplicate (of)
linear issue unrelate KK-12 --blocks KK-13                             # the same arguments remove it
linear comment update <COMMENT-ID> --body-file comment.md              # a comment you wrote
linear comment delete <COMMENT-ID> --yes                               # cannot be undone
linear issue reorder KK-3 KK-1 KK-2                                    # same project; top first
linear issue reorder KK-3,KK-1,KK-2                                    # the same, comma-separated
```

A date that is not on the calendar (`2027-02-30`) is refused as a usage error (exit 2) in `--due`
and `--target-date`; it is not rolled over to the next valid day.

`--body-file -` reads standard input. Names (states, projects, milestones, labels, users, teams)
are resolved before anything is sent; one that matches nothing, or more than one thing, is a
usage error (exit 2) that lists the candidates. Every write goes through the same steps, and
nothing is sent until the first two have passed:

1. **Ownership rules** (always on; exit 4; see "Ownership rules" below for the `lenient`
   setting). A project may be written only when you lead it. An
   issue may be changed (or commented on, reordered, deleted, archived, restored, unlinked from or attached a file to) when it is assigned to you or its project
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

#### Cycles: `linear cycle` and `issue create --held-on`

A recurring meeting declares what gets done in the week after it, so its issues belong in the
cycle that is running the next day. `linear cycle 2026-10-05` prints **the cycle that contains the
day after the meeting** (read-only; `--team` or the workspace's `default_team` picks the team).
The rule does not look at weekdays: with cycles that start on Tuesday, a Monday meeting lands in
the cycle that starts the next morning, a Tuesday one in the cycle that started that day. The day
is judged at noon Japan Standard Time, so a midnight boundary never decides it. When no cycle
contains that day the command fails (exit 2) and lists the cycles that exist, because a missing
cycle means the team's cycles are not set up that far ahead; it is never taken for "no cycle".
`--json` prints `id`, `number`, `name`, `startsAt`, `endsAt`, plus `workspace`, `team` and `heldOn`;
`--quiet` prints the cycle id.

`issue create --held-on 2026-10-05` puts the new issue in that cycle, and fails before creating
anything when there is none. If an issue with the same `--source` already exists (this needs the
`source-attachment` rule, as above), its cycle is set only when it has none; its description, labels
and everything else are not touched, and a cycle somebody chose is not taken back (a note says
so). That alignment is a write to an existing issue, so the ownership rule for changing an issue
applies to it (exit 4). With `--json` the output has `cycle` (the cycle the issue is in) and
`changed` (`["cycle"]` when this run set it, otherwise empty). Creating or changing cycles
themselves is not supported.

`linear cycle list` and `linear cycle view <NUMBER>` read the same team's cycles for planning
(read-only; `--team` or `default_team` picks the team). They are subcommands of `cycle`, and
`linear cycle <DATE>` is unchanged: a date is the command's own argument, so `list` and `view`
never clash with it (`cycle --team EX list` is refused; give `--team` to the subcommand).

- `cycle list` prints `NUMBER NAME STATE STARTS ENDS PROGRESS`. `STATE` is `active`, `next`
  (the next one to start), `upcoming` (later) or `past`; `--state active|upcoming|past` narrows
  it. Linear cannot sort cycles, so every page is fetched and the list is sorted newest first
  before `--limit` (default 50) or `--all` applies. `--json` prints an array of cycles with
  `id`, `number`, `name`, `description`, `startsAt`, `endsAt`, `completedAt`, `isActive`,
  `isFuture`, `isNext`, `isPast`, `progress` (0 to 1) and `team`, each with `workspace`;
  `--quiet` prints the numbers.
- `cycle view 41` prints that cycle and the issues in it (`--limit` and `--all` page through
  the issues, as for the other listings). `--json` is the cycle's fields above plus `issues`
  (identifier, title, url, state, assignee); `--quiet` prints the cycle number. A number the
  team does not have is a usage error (exit 2).

#### `issue update`

`issue update` sends only the fields that differ from what the issue has now, and never `null`
for a field it was not asked to clear. A second identical run sends nothing and says so
(`"changed": []` with `--json`; otherwise `changed` lists what was written: `description`,
`state`, `project`, `milestone`, `dueDate`, `assignee`, `labels`, `source`).

- `--body-file FILE|-` replaces the description (an empty file is a usage error). Trailing
  whitespace does not count as a difference, nor does the bullet Linear rewrites (`- item` is
  stored as `* item`); another rewrite the CLI does not know about costs one redundant write. `--template NAME` (only with `--body-file`) holds the
  new body to that Linear template through the `template-sections` rule. An issue does not
  record which template it came from, so with the rule on, `--body-file` without `--template`
  is refused (exit 5) rather than left unchecked; there is no flag to skip the check. To leave
  updates unchecked, narrow the rule with `rule_operations` (see [Validator rules](#validator-rules)).
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
  and `sourceType`. Attachments made by integrations may nest their metadata; it is shown as is
  (the [GitHub](#github) section says how pull requests are read from it).

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

### GitHub

Linear's GitHub integration links a pull request to an issue and keeps its state in sync. Three
things of it reach `linear`:

- **`issue view --json` shows the branch name**: `branchName` is Linear's suggested git branch for
  the issue (Linear's own field name). A branch named like that, or one that contains the issue
  identifier, has its pull request linked to the issue by the integration with no command at all.
- **`issue link-pr <issue> <url>`** links a pull request (`https://github.com/<owner>/<repo>/pull/<n>`;
  anything after the number, such as `/files`, is dropped) with `attachmentLinkGitHubPR`, which
  makes the attachment the integration keeps in sync, unlike a plain `--source` link. It follows
  the same [ownership rules](#ownership-rules) as any change to the issue. It asks Linear first
  whether the workspace has the GitHub integration installed and refuses with an error, sending
  nothing, if not; a pull request the issue already has is not linked again (`alreadyLinked`).
- **`issue view --json` reads the pull requests**: the raw attachments (`sourceType: "github"`
  and the integration's `metadata`) are under `attachments`, and `pullRequests` is the reading
  of them: `url`, `number`, `title`, `status` (`open`, `draft`, `merged`, `closed`, or `unknown`
  when the metadata does not say), `openedAt`, `mergedAt` and `closedAt`. A `github` attachment
  whose URL is not `.../pull/<n>` (the integration also makes one for a linked GitHub issue) is
  not a pull request. Only the first 10 attachments of an issue are read.

The audit rules `pr-merged-issue-open` and `pr-open-too-long` only warn, and never change a state:
a pull request is evidence, not the authority on whether the work is done. A workspace without
the integration, and an issue without a pull request attachment, give them nothing to look at.

The shape of the integration's `metadata` is the integration's own, not a documented contract,
and none of the workspaces it was developed against held a pull-request attachment to copy, so
the reading is deliberately lenient: it uses `status` (or `mergedAt` / `closedAt` / `draft`) when
they are there, takes the time a pull request was opened from `createdAt` (else from when the
attachment was made), and reports anything it cannot read as `unknown`, which no rule acts on.
`issue link-pr` needs a workspace with the integration and so is not covered by the live (sandbox)
tests; the mock tests cover it, including the refusal when the integration is missing.

### Comments and relations

```sh
linear comment update <COMMENT-ID> --body-file comment.md
linear comment delete <COMMENT-ID> --yes
linear issue relate KK-12 --blocks KK-13      # KK-12 blocks KK-13
linear issue relate KK-12 --duplicate KK-13   # KK-12 is a duplicate of KK-13 (the issue you keep)
linear issue relate KK-12 --related KK-13     # no direction
linear issue unrelate KK-12 --blocks KK-13
```

- **A comment is its author's.** `comment update` and `comment delete` name the comment by its id
  (the `id` that `issue comment --json` and `issue view --json` print; only a comment on an issue
  qualifies). The ownership rule looks at who wrote the comment, not at who owns the issue it is on:
  in a `strict` workspace somebody else's comment, or one with no author (an integration's), is
  refused (exit 4). A `lenient` workspace lets anyone edit a comment, like any change to an issue,
  but `comment delete` stays the author's even there (see the table under "Ownership rules").
  `comment update` replaces the text (`--body-file`, `-` for standard input); text equal to what is
  there sends nothing (`changed: false`).
- **`comment delete` needs `--yes`, because a deleted comment cannot be restored.** Measured on the
  sandbox (2026-10-07): afterwards `comment(id:)` answers "Entity not found" and the issue's
  `comments(includeArchived: true)` no longer lists it, and Linear's schema has no mutation that
  brings a comment back (no `commentUnarchive`). Without `--yes` the command fails with exit 2
  before any request.
- **A relation is one fact, stored once** as `issue` `type` `relatedIssue`: `KK-12 blocks KK-13` is
  shown on KK-12 under `relations` and on KK-13 under `inverseRelations`. `issue relate` writes it
  from the issue named first, so that is the issue the ownership rules ask about (the same as for
  changing it: assigned to you, or in a project you lead); the other issue only has to exist, and
  may be somebody else's. A relation that is already there is not made again (`alreadyRelated`);
  `blocks` and `duplicate` have a direction (`KK-13 blocks KK-12` is another relation), `related`
  has none and is found from either end. An issue cannot be related to itself (exit 2).
- **A duplicate relation closes the issue.** Linear moves the issue to its `Duplicate` state when the
  relation is made (measured on the sandbox), which is canceling it, so `issue relate --duplicate`
  also asks the cancel rule: it stays refused on somebody else's issue in a `lenient` workspace.
  Removing the relation moves the issue back out of `Duplicate` (observed; Linear may take a few
  seconds), which is not a cancel, so `unrelate` asks only for the ordinary change.
- **`issue unrelate` takes the same arguments as `relate` and needs no `--yes`.** A relation holds
  only the two issues and its kind, so `relate` makes it again exactly (only its id changes). One
  that is not there sends nothing (`removed: []`, exit 0), so running it twice is safe.
- **`issue view --json` shows the relations**, in Linear's own shape: `relations` and
  `inverseRelations`, each `{ "nodes": [{ id, type, issue, relatedIssue }] }` where an end is
  `{ id, identifier, title, url, state }` (at most 50 of each). `type` is `blocks`, `duplicate`,
  `related` or `similar` (Linear's own suggestion; the CLI makes the first three). The text view
  lists them from the issue's point of view (`blocks`, `blocked by`, `duplicate of`,
  `duplicated by`, `related to`). `--cached` has no relations: the cache holds only the list fields.

### Projects

```sh
linear project create --name "Ship it" [--summary "One line"] [--body-file body.md] \
  [--template "Project"] [--target-date 2026-12-31] [--initiative "Roadmap"] [--lead me] [--team ENG]
linear project update "Ship it" [--name ..] [--summary ..] [--body-file body.md] [--template ..] \
  [--status "In Progress"] [--target-date 2027-01-31] [--initiative "Roadmap"] [--lead me]
linear project status-update "Ship it" --health onTrack|atRisk|offTrack --body-file update.md
linear project reorder "Ship it" "Other" "Third"                       # top first
linear project delete "Ship it"                                        # trash; unarchive brings it back
linear project unarchive "Ship it"
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
*project* template that heading (`linear template create --type project`), enable the `template-sections` rule and pass `--template`
(see below); no heading is hard-coded in the CLI.

### Ownership rules

The ownership rules (exit 4) are always on. How strict they are is a per-workspace key:

```toml
[workspaces.team]
ownership = "lenient"   # default: "strict"
```

| | `strict` (default) | `lenient` |
| --- | --- | --- |
| create an issue in a project somebody else leads | only assigned to you, with `--allow-foreign` | allowed, for anyone, no flag |
| create an issue without a project | only assigned to you | allowed, for anyone |
| change, comment on, reorder, delete, archive or restore an issue owned by someone else, or unlink its attachments or attach a file to it (this includes moving it to another project) | refused | allowed |
| cancel an issue (a `canceled` or `duplicate` state) that is not yours | refused | **refused** |
| edit a comment somebody else wrote (`comment update`) | refused | allowed |
| delete a comment somebody else wrote (`comment delete`) | refused | **refused** |
| create, change, delete or restore a project (and its milestones and status updates) you do not lead | refused | **refused** |

`linear workspace list` shows the value (`ownership` in `--json`). The setting is checked when
the configuration is read; anything but `strict` or `lenient` is an error. Use `lenient` for a
workspace where people file and edit issues for each other (a team workspace); keep `strict`
for a personal one. Validators and the workspace check still apply in both.

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

By default `template-sections` covers every write it supports, `issue_create`, `issue_update`,
`project_create`, `project_update`, `document_create` and `document_update`: while the rule is on, `issue create`, `project create`, `issue update --body-file`
and `project update --body-file` are refused (exit 5) unless `--template` names the Linear
template to hold the body to. An update that leaves the body alone is not checked. Replacing a body without a template is refused, not skipped, because an
issue or a project does not record which template it came from. There is no flag to opt out of one
write; to stop checking some operations, name the ones to keep in `rule_operations`, for example
`rule_operations = { "template-sections" = ["issue_create"] }` to check issue creation only (updates
and projects are then unchecked).

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
linear initiative update "Long effort" [--name "New name"] [--description-file desc.md] \
  [--status active] [--target-date 2027-03-31] [--owner me]
linear initiative add-project "Long effort" "My Project"               # also: remove-project
linear initiative status-update "Long effort" --health onTrack --body-file update.md
linear initiative archive "Long effort"                                # also: unarchive, delete
linear template create --name "Bug report" --body-file body.md [--description "..."] [--team ENG]
linear template create --type project --name "Project" --body-file body.md [--description "..."]
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
  `initiative update` changes the name, the description (from a file), the status
  (`active`, `planned`, `proposed`, `completed`, `canceled`), the target date and the owner, and
  sends only what differs from now (`changed` lists it with `--json`; a run that changes nothing
  sends nothing). `initiative add-project` and `remove-project` put a project under an initiative
  or take it out; both are idempotent (a project already under it, or not under it, is left
  alone and nothing is sent, `unchanged` with `--json`). Only the link goes: the project and the
  initiative stay, and the other command puts it back. `initiative status-update` is
  `project status-update` for an initiative: `--health onTrack|atRisk|offTrack` and the text from
  `--body-file`; `initiative status-updates` lists them. An initiative belongs to the workspace
  and has no ownership rule (as for `create`, `archive` and `delete`), so anyone may update it
  and write its status updates. A project's initiatives change when it is added or removed, so
  `add-project` and `remove-project` also follow the ownership rule of the project (you must lead
  it, exit 4), as `project update --initiative` does. Initiative relations and labels are not
  supported.
- `template create` makes an **issue template** (the default) or, with `--type project`, a
  **project template** from a markdown body. Headings are the sections, and a body without one is
  refused. Understood markdown: headings, paragraphs, bullet and numbered lists and `**bold**`.
  An issue template belongs to a team (`--team`, default the workspace's `default_team`); a
  project template has none, and `--team` with `--type project` is refused (exit 2). A template
  with the same name **and type** returns the existing one (an issue template and a project
  template may share a name). Edit a template's sections in Linear's own UI;
  `linear template skeleton [NAME] [--type project]` reads them back (`--type` defaults to
  `issue`), and `linear template list --type project` / `linear template view NAME` show project
  templates.

No validator rule applies to these three (the rules cover issues and projects), and initiatives
and templates are not owned by a project, so only the checks above run.

### Files

```sh
linear file upload ./shot.png [--name "Login screen"] [--content-type image/png]
linear issue attach-file KK-12 ./shot.png [--title "Login bug"]
linear file download https://uploads.linear.app/<org>/<id>/<id> [--output shot.png] [--force]
```

A file reaches Linear in two steps: `fileUpload` returns a signed URL and the headers to send,
then the bytes are `PUT` there (without your credential, which the signed URL does not need).
The result is an asset URL that is private to the workspace.

- `file upload` stops there and prints the URL and the markdown that embeds it: `![name](url)`
  for an image, `[name](url)` for anything else (`--json`: `assetUrl` and `markdown`; `--quiet`:
  the URL). Paste the markdown into an issue description or a comment (`issue comment`,
  `issue update --body-file`) to show the image inline.
- `issue attach-file` also attaches the URL to the issue, titled with the file's name unless
  `--title` says otherwise, and prints the same URL and markdown. It follows the ownership rules
  of changing the issue and, like the other issue writes, the issue's validators run first.
- **Limit:** a file may be at most **25 MiB** (a screenshot, a log, a document; not a video).
  A larger file, an empty one, a directory and an unreadable path are refused (exit 2) before any
  request. The whole file is read into memory and sent in one request, and `--timeout` applies to
  it, so a large file on a slow line needs `--timeout` raised. Linear applies limits of its own on
  top. Videos and large files are not handled any further than this.
- **Failure:** if the `PUT` fails, nothing was stored and nothing was attached (exit 1). If the
  file was stored but attaching it fails (after two more tries: `attachmentCreate` is an upsert on
  the URL, so repeating it is safe), the CLI looks whether an attachment with that URL exists. If
  none does, it deletes the stored file again (Linear's `fileUploadDangerouslyDelete`, used for
  nothing else), and the error says so; if one does, or that cannot be checked, the file is kept
  and its URL is in the error; if the delete fails, the URL is in the error as well (a file with
  nothing attached to it is harmless).
- `file download` saves the file at a Linear URL. The workspace's credential is sent only to
  `uploads.linear.app` over https (or to the origin of the API endpoint, which is how a proxy
  stands in for Linear), and a redirect to another host drops it, so a URL of any other site is
  fetched without it. It saves to `--output` (`-` for standard output), by default to the last
  part of the URL in the current directory, refuses to replace an existing file without
  `--force`, writes through a temporary file so a failed download leaves nothing, and refuses to
  save more than 100 MiB. A deleted file may stay downloadable for a short while from Linear's
  cache.

### Delete and archive

```sh
linear issue delete KK-12        # trash it
linear issue archive KK-12
linear issue unarchive KK-12     # brings back an archived or deleted issue
linear project delete "Ship it"  # trash it
linear project unarchive "Ship it"
linear initiative archive "Long effort"
linear initiative unarchive "Long effort"
linear initiative delete "Long effort"   # trash it
linear issue unlink KK-12 https://example.com/a --yes
```

| Command | Reversible | How it is undone |
| --- | --- | --- |
| `issue delete`, `project delete`, `initiative delete` | yes: Linear moves it to the trash and keeps it for a while | `issue unarchive`, `project unarchive`; for an initiative, `initiative unarchive` |
| `issue archive`, `initiative archive` | yes | `issue unarchive`, `initiative unarchive` |
| `issue unlink` | **unknown**: Linear documents no way to bring a deleted attachment back | none; the command needs `--yes` |

None of these needs `--yes` except `issue unlink`. Without it, `issue unlink` looks the
attachment up, prints what it would delete and exits with code 2, sending nothing. It finds the
attachment of that issue whose URL is exactly the one given; an issue without one is left alone
(the command says so and exits 0, `notLinked` with `--json`), like `issue link-pr` with a pull
request already linked. Metadata of an attachment cannot be changed with these commands.

The ownership rules are those of changing the thing: an issue must be yours (assigned to you, or
in a project you lead; any issue in a `lenient` workspace), and a project must be one you lead,
as for `project update`. An initiative belongs to the workspace and has no ownership rule, as for
`initiative create`. A project or initiative that was deleted or archived is found by `unarchive`
among the deleted ones too (by id, slug id, URL or name); the other commands only see live ones.
`--json` prints the workspace, the `id`, the identifier (`identifier` for an issue, `slugId` and
`name` for a project or initiative), the `url` and the `action` (`deleted`, `archived` or
`unarchived`); `--quiet` prints the identifier or slug id. There is no `project archive`: Linear
has deprecated `projectArchive` in favour of `projectDelete`. Deleting comments,
labels or relations, and deleting in bulk, is not supported.

## Webhooks

```sh
linear webhook list [--limit N | --all]
linear webhook create --url https://example.com/linear --resource-types Issue,Comment \
  (--team ENG | --all-public-teams) [--label "CI"]
linear webhook delete "CI"          # an id, a label or a URL
linear webhook verify --signature <hex> [--body-file body.json] [--secret-file secret] [--at <ms>]
```

- `list` shows the id, label, URL, scope (a team key, `all public teams`), resource types and
  whether the webhook is enabled. It never selects the signing secret.
- `create` needs a URL, at least one resource type (`Issue`, `Comment`, `Project`, ...) and
  either `--team` (a key, name or id, resolved before anything is created; an unknown team is
  exit 2) or `--all-public-teams`. Linear generates the signing secret and `create` prints it
  (`secret` with `--json`; `--quiet` prints only the id); no other command does, so keep it. Who
  may manage webhooks is Linear's decision (an admin); its refusal is passed on (exit 1). No
  ownership rule or validator applies: a webhook belongs to the workspace, not to a project.
- `delete` looks the argument up among the workspace's webhooks (an id, or a label or URL, ignoring
  case) and deletes that one. A name that matches nothing, or several (two webhooks can deliver
  to one URL), is exit 2 and deletes nothing.
- `verify` is offline (no workspace, no credentials, no network). It checks a delivery you received:
  the HMAC-SHA256 signature of the body (the `Linear-Signature` header, `--signature`) and that
  its `webhookTimestamp` is within a minute of now. It is the same check as `verify_webhook` in the
  WebAssembly package. The body is read byte for byte from `--body-file` or standard input (not a
  re-serialization: one changed byte is another signature), and the secret from `--secret-file` or
  the `LINEAR_WEBHOOK_SECRET` environment variable, never from an argument, which other users of
  the machine can see. A valid delivery exits 0 and prints the action and resource type (`--json`:
  `{"status":"valid","event":{...}}`). A rejected one exits 1 and says why: `malformed-signature`,
  `signature-mismatch`, `malformed-body`, `missing-timestamp` or `stale-timestamp` (`--json` prints
  `{"status":"invalid","reason":...}` on standard output, and the usual error object on standard
  error). A missing or empty secret is exit 2. `--at <epoch ms>` judges the timestamp against
  another time, to check a saved delivery.

There is no listener: receiving deliveries is your server's job; `verify` is for the check it makes
(or for trying one by hand).

### Labels

```sh
linear label create --name "ship" [--team ENG] [--group area] [--color "#4EA7FC"] [--description "..."]
linear label create --name "area" --is-group [--group-type single-select]
linear label update "area/ship" [--new-name "released"] [--color "#4EA7FC"] [--description "..."] \
  [--group other-group | --no-group] [--group-type single-select]
```

- `label create` makes a label (or, with `--is-group`, a group) in a team (`--team KEY`) or, without
  `--team`, in the whole workspace; with `--group` and no `--team` it takes the group's team. It
  does not fall back to `default_team`: a label with no team is visible to every team. `--color`
  is `#RRGGBB`. A label that already exists in the place asked for (same name ignoring case, same
  team, same group) is returned instead of made again (`"existing": true` with `--json`).
- `label update` takes a name, a `group/name` path or an id, and sends only what differs from now
  (`"changed": false` when nothing does). `--description ""` clears the description. `--group`
  moves the label into a group, `--no-group` takes it out; `--group-type` changes a group's
  selection mode (single-select, or multi-select where the workspace has it). Turning a label into
  a group or back is not supported.
- Checked before anything is sent (exit 2): the name is not empty and no other label has it
  (Linear keeps names unique across the workspace and its teams, whatever the case and group, so a
  team label cannot take a workspace label's name either); the group exists and is a group, of the
  label's own team (or the workspace's, for a workspace label); a group is not put in a group;
  `--group-type` is only for groups; a name that two labels share is given as an id.
- `label-groups-exclusive` (see [Validator rules](#validator-rules)) also covers `label update`:
  moving a label into a single-select group, or making a group single-select, is refused
  (exit 5, naming the issues) when an issue that is already in Linear would end up with two labels
  of that group. It reads the issues that carry the label (or the group's labels), the first 1000
  of them, and says so when there were more. A label that is being created is on no issue, so a
  creation has nothing to hold against the rule, and a multi-select group never conflicts.
- No ownership rule applies (a label belongs to a team or the workspace, not to a project or an
  issue); Linear decides who may add workspace labels. Deleting a label is not supported.

### Documents

```sh
linear document create --title "Design notes" --project "My Project" --body-file notes.md [--template "Plan"]
linear document create --title "Roadmap" --initiative "Long effort" --body-file roadmap.md
linear document update design-notes-1a2b3c4d5e6f [--title "Notes"] [--body-file notes.md] [--template "Plan"]
```

- `document list` (`--project`, `--initiative`, `--title`, `--limit`, `--all`) shows the documents
  of the workspace, or of one project or initiative, with what each hangs off. `document view`
  takes an id, a slug id or a document URL and prints the document with its body (`--json` has the
  body as `content`; `--quiet` prints the slug id). Documents of an issue, a team or a cycle are
  read like any other.
- Writing is limited to the documents **of a project you lead or an initiative you own** (exit 4
  otherwise, before anything is sent). `create` needs exactly one of `--project` and
  `--initiative`. An initiative that has no owner is nobody's: a new one made with
  `initiative create` has to get an owner in Linear before the CLI writes under it. Lenient
  ownership relaxes issue writes only, so it changes nothing here. `update` works out the parent
  from the document itself, and refuses (exit 2) one whose parent is neither a project nor an
  initiative.
- `create` with a title the parent already has returns that document (`"existing": true` with
  `--json`) and creates nothing; `update` sends only what differs (`"changed": false` when nothing
  does). A body file that is empty is not sent (it is a usage error for `update`: a body is not
  cleared this way). `--body-file -` reads standard input.
- **The body is checked by `template-sections`**, like an issue's or a project's: with the rule
  on, `--template` names a Linear *document* template (made in Linear; `linear template create`
  makes issue and project templates only) and the body has to fill every section of it, otherwise
  the write is refused (exit 5). `document create` always needs a template then, `document update`
  only when it replaces the body (`--body-file`); a rename is not checked. Without the rule,
  `--template` is ignored with a note. To leave documents out of the rule, name the other
  operations in `rule_operations` (`document_create` and `document_update` are the two it
  knows for documents). `source-attachment` and `label-groups-exclusive` are about issues and
  do not apply.

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
  for milestones, initiatives and templates, `comment_write.rs` and `issue_relation.rs` for
  `comment update|delete` and `issue relate|unrelate`, and `api_mutation.rs` for `linear api --mutation`.
- `crates/cli/tests/live*.rs` talk to a real (sandbox) workspace and are ignored by default;
  `live_write.rs`, `live_attachment_meta.rs` and `live_comment_relation.rs` create issues there and cancel them when they are done;
  `live_structure.rs` creates initiatives, projects and issues and removes them with `delete` and
  `archive` (after exercising `unarchive`):

  ```sh
  LINEAR_API_KEY_SANDBOX=... cargo test -p linear --test live_write -- --ignored
  ```

  `live_initiative_write.rs` and `live_files.rs` do the same for initiative updates, project
  links and status updates, and for file upload, attach and download (they delete the files they
  uploaded through the cleanup mutation).

  `live_audit.rs` needs the discrepancies `scripts/seed-sandbox-audit.py` plants (run it once;
  it is idempotent and refuses any workspace but the sandbox).

## License

MIT
