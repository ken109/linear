# Parity: `tools/linear.ts` to `linear`

Temporary document. It is removed together with `scripts/parity/` when the old tool is retired.

The old tool is `tools/linear.ts` in ken109/monorepo (the same file, with fewer commands, also lives in
lt-three/monorepo). This lists every command it has and what replaces it in the `linear` binary,
the differences in behaviour that the parity run found, and the gaps that remain.

How it was checked:

- Every `linear` command and flag named in this document in a code span that starts with `linear`
  is verified against `--help` by `scripts/parity/check_help.py` (it parses this file).
- The write scenarios (`scripts/parity/run.sh`) ran the same steps through both tools in the sandbox
  workspace (`ken109-sandbox`, team `SAND`) and compared the resulting Linear state.
- The read commands were compared against the real `ken109` and `lt-three` workspaces with
  `scripts/parity/read_compare.py` (read-only).

Status values: **same** (same command shape, same result), **renamed** (same behaviour, other
name or flag), **merged** (folded into another command), **partial** (works, but something is
missing), **not provided**, **new** (no old equivalent).

## Command table

Old commands are listed from the code (`switch (command)` in `main()`), not only from the header comment.
In the old tool a flag takes its value as `--flag value`; `<...>` below are values.

| Old command | New command | Status | Notes |
| --- | --- | --- | --- |
| `login` | `linear workspace login --with-token` | partial | The old tool does an OAuth PKCE login in the browser and refreshes its token. `linear` stores a personal API key (stdin or prompt, or `LINEAR_API_KEY_<NAME>`). `workspace add --auth oauth` exists but OAuth is not implemented. The issuer is the key's owner in both cases. |
| `whoami` | `linear workspace whoami` | renamed | Prints workspace, user and where the credential came from. Shape differs (see below). |
| `initiatives` | `linear initiative list --status --owner --limit --all` | renamed | Old: first 100, any status, JSON with `description`. New: default 50, `--all` for every page; the list JSON has no `description` (`initiative view` does). lt-three's old tool leaves out Completed initiatives; use `--status active,planned,proposed,canceled`. |
| `create-initiative --name --description-file` | `linear initiative create --name --description-file` | same | Both return the existing initiative when the name is taken. Linear refuses initiatives on the free plan. |
| `projects` | `linear project list --open --lead --status-type --initiative --limit --all` | renamed | The old tool lists only unfinished projects (first 100, with `initiative`, `status`, `targetDate`, `url`). The equivalent is `project list --open --all`. Without `--open` the new command also lists completed and canceled ones. JSON is Linear's own shape (`status` is an object, initiatives are `initiatives.nodes`). |
| `templates` | `linear template list --type` | renamed | Same set. |
| `skeletons` | `linear template skeleton` | renamed | Output identical on ken109 and lt-three. |
| `template <name>` | `linear template skeleton <TEMPLATE>` | renamed | Name is positional in both. `linear template view <TEMPLATE>` also shows the sections as a list. |
| `create-template --name --body-file --description` | `linear template create --name --body-file --description --team` | same | Same name returns the existing template; a body without a `## heading` is refused. |
| `create-issue --template --project --title --body-file --source --source-title --milestone --assignee --label` | `linear issue create --template --project --title --body-file --source --source-title --milestone --assignee --label --team --allow-foreign` | same | See "Behaviour". `--template`, `--source` and `--body-file` are required by the old tool; in `linear` the first two are required only when the `template-sections` / `source-attachment` rules are on (`--body-file` is optional). `--allow-foreign` and the ownership rules are new. |
| `update-issue --id --state --project --milestone --due --assignee` | `linear issue update <ISSUE> --state --project --milestone --due --assignee` | renamed | `--id KK-1` became the positional argument. `--assignee` takes `me`, an email or a name (old: email only). |
| `issues --project --milestone` | `linear issue list --project --milestone --open --state-type --state --assignee --label --source-url --team --limit --all` | partial | Old: open issues of one project in `sortOrder` order (the order on screen). New: filter with `--open`; the order is Linear's default, not `sortOrder` (the JSON has `sortOrder` to sort by). See gap G5. |
| `reorder-issues --ids KK-1,KK-2` | `linear issue reorder <ISSUE>...` | renamed | Identifiers are positional, space or comma separated. Both write `sortOrder` and `prioritySortOrder`. |
| `comment --id --body-file` | `linear issue comment <ISSUE> --body-file` | renamed | |
| `create-project --name --summary --content-file --target-date --initiative --lead` | `linear project create --name --summary --body-file --target-date --initiative --lead --template --team` | renamed | `--content-file` is `--body-file`. In the old tool `--summary` and the body file are required; here they are optional. |
| `update-project --id --name --summary --content-file --initiative --state --target-date --lead` | `linear project update <PROJECT> --name --summary --body-file --status --target-date --initiative --lead --template` | renamed | `--id` is positional, `--content-file` is `--body-file`, `--state` is `--status`. |
| `milestones --project` | `linear milestone list --project` | renamed | Also `linear milestone view <MILESTONE> --project`. |
| `create-milestone --project --name --target-date --description-file` | `linear milestone create --project --name --target-date --description-file` | same | |
| `update-milestone --project --name --new-name --target-date --description-file` | `linear milestone update <MILESTONE> --project --new-name --target-date --description-file` | renamed | The current name (`--name`) is positional. |
| `delete-milestone --project --name` | `linear milestone delete <MILESTONE> --project` | renamed | The name is positional. Refuses while issues remain, in both. |
| `status` | `linear brief --stale-days --json` | renamed | The same brief of unfinished projects that are in progress or have a status update: health, first 3 lines of the latest update (list markers removed, lines cut at 120 characters), the initiative, milestones done and the next one, "no status update" for an in-progress project without one, a stale mark, newest update first. English, different layout. Differences: the stale threshold is the workspace's `audit.status_update_days` (14, as in `linear audit`; `--stale-days` overrides); days are calendar days in the machine's time zone (the old tool used JST; `linear audit` counts elapsed whole days, so the two can differ by one day at the boundary); `--json` is new. It asks Linear (no `--cached`: the cache holds a different set of projects). `linear project list --open` still lists the same projects without the brief. |
| `status --project <id>` | `linear project view <PROJECT>` | renamed | Same facts: latest update in full, earlier updates (old shows 3, new up to 5), milestones with progress. English, different layout. |
| `status --session` | `linear brief --session` | same | The SessionStart variant: prints nothing in CI (`CI` set, before anything else), prints nothing and exits 0 on any failure (no credentials, offline, bad key, Linear error), 4 s in all, markdown on stdout. The old tool allowed 8 more seconds for refreshing an OAuth token; `linear` has none to refresh. |
| `status-update --project --health --body-file` | `linear project status-update <PROJECT> --health --body-file` | renamed | Same health values (`onTrack`, `atRisk`, `offTrack`). Not idempotent in both: writing it twice gives two updates. |

Only in lt-three's old tool:

| Old command | New command | Status | Notes |
| --- | --- | --- | --- |
| `open-issues` | `linear issue list --open --all` | partial | The old command also returns issues closed within the last 14 days (for duplicate checks). `linear` has no completed-since filter. See gap G5. |
| `cycle <date>` | | not provided | Resolves the cycle a meeting's commitments go into. Needed for lt-three's minutes workflow. |
| `create-issue --held-on` | | not provided | Puts the new issue (or an existing one without a cycle) in that cycle. |
| client credentials (`LINEAR_CLIENT_SECRET`, app token for CI) | | not provided | `linear` has no OAuth / client-credentials auth, so lt-three's CI cannot use it yet. |

New in `linear` (no old equivalent):

| New command | Notes |
| --- | --- |
| `linear issue view <ISSUE>` | One issue with description and comments. |
| `linear issue update --body-file --template --source --source-title --meta --labels --add-labels --remove-labels` | The old `update-issue` could not change the description, labels or source. |
| `linear issue create --meta` | Metadata of the source attachment. |
| `linear project view <PROJECT> --content` | Includes issue counts and the body. |
| `linear project reorder <PROJECT>...` | Orders projects. |
| `linear milestone view <MILESTONE> --project` | The milestone's issues. |
| `linear initiative view <INITIATIVE>` | The initiative's projects. |
| `linear template view <TEMPLATE>` | Sections as a list. |
| `linear label list`, `linear team list`, `linear user view <USER>` | |
| `linear audit --issues --since --fail-on --cached --ttl` | Finds drifted data. |
| `linear cache refresh`, `linear cache show`, `linear cache clear` | A cache for hooks and the statusline. |
| `linear api --query-file --var --var-json --variables-file --operation-name --mutation` | Raw GraphQL. |
| `linear workspace add`, `linear workspace list` | Several workspaces (`-w` / `LINEAR_WORKSPACE` / `.linear.toml`). |

## Behaviour that differs

Observed in the sandbox runs (`scripts/parity/run.sh`), unless marked "by reading the code".

### Exit codes and messages

The old tool exits 1 for everything and prints a Japanese message on stderr. `linear` uses the
codes of the README (1 general, 2 usage, 3 authentication, 4 refused by the ownership rules, 5 refused
by a validator) and English messages; with `--json` the error is `{"error":{"code","message"}}` on stderr.

| Case | Old exit | New exit | Linear changed |
| --- | --- | --- | --- |
| nothing to change (`update-project`, `update-milestone`, `update-issue`) | 1 | 2 | no / no |
| unknown project, milestone, state, status, label, lead, assignee, initiative | 1 | 2 | no / no |
| empty or whitespace-only body (status update, comment, project body) | 1 | 2 | no / no |
| invalid `--health` | 1 | 2 | no / no |
| milestone without `--target-date` | 1 | 2 | no / no |
| rename onto an existing project or milestone name | 1 | 2 | no / no |
| reorder: one issue, the same issue twice, issues of two projects | 1 | 2 | no / no |
| template body without a `## heading` | 1 | 2 | no / no |
| issue body that misses or leaves empty a template section | 1 | 5 | no / no |
| unknown `--template` on `create-issue` | 1 | 5 | no / no |
| `--source` that is not an http(s) URL | 1 | 5 | no / no (see below) |
| delete a milestone that still holds an issue | 1 | 1 | no / no |
| unknown issue (`update-issue`, `comment`, `reorder`) | 1 | 1 | no / no |

Scripts that test only `$? -ne 0` keep working. Anything that parses the message text or relies on
"always 1" does not.

### Where the new CLI refuses and the old tool accepts

| Input | Old tool | `linear` |
| --- | --- | --- |
| a date that is not on the calendar but passes JavaScript's `Date.parse`: `2027-02-30`, `2026-02-31`, `2026-11-31` (`--target-date`, `--due`, milestone date) | accepted; Linear stores the next valid day (`2027-03-02`, `2026-03-03`, `2026-12-01`) | refused, exit 2 (`input is out of range`) |
| `create-project` without `--summary` / body file | refused (both are required) | accepted (both optional) |
| `--source not-a-url` | creates the issue, the attachment fails three times (about 4 s), the issue is deleted again, exit 1; nothing remains | refused before anything is sent, exit 5 |

Month 13 (`2026-13-45`) and non-dates (`nope`) are refused by both.

### Same behaviour

- **Idempotence**: `create-project` (same name, unfinished), `create-milestone` (same name in the
  project), `create-issue` (same `--source`), `create-template` and `create-initiative` (same name)
  return the existing object and create nothing. `status-update` is not idempotent in either tool (two
  identical updates give two updates).
- **Validation before writing**: unknown names (state, project, status, milestone, label, user,
  template), empty bodies, missing or empty template sections, label groups, a rename onto an existing
  name, a milestone that still has issues, a reorder across projects: all are refused with nothing
  changed, in both.
- **Moving an issue to another project clears its milestone** unless `--milestone` is given, in both.
- Names are matched ignoring case (`done` finds `Done`, `m1R` finds `M1r`, `planned` finds `Planned`).
- Created objects have the same fields (name, summary, body, target date, lead = you, status,
  milestone description and date, issue description, assignee, due date, labels, comments, status
  updates, template sections).

### Output shape

Success output of the old tool is a small JSON object (`{id, url}`, `{id, identifier, url}`,
`{id, name, targetDate}`, ...) and, for `whoami`, `template`, `skeletons` and `status`, text.
`linear --json` prints Linear's own shape for the object plus `workspace`, and `existing` on creates and
`changed` on updates. Without `--json` the output is for people. What a caller has to change:

| Old output | `linear --json` |
| --- | --- |
| `update-issue` → `state` (name), `project` (name), `milestone` (name) | the whole issue: `state.name`, `project.name`, `projectMilestone.name` |
| `update-project` → `status` (name), `initiatives` (names) | the whole project: `status.name`, `initiatives.nodes[].name` |
| `create-*` → `existing` only when true | `existing` is always present (`true` / `false`) |
| `create-milestone` → `{id, name, targetDate}` | the milestone with `description`, `progress`, `status`, `sortOrder`, `project` |
| `status-update` → `{id, url, health}` | the status update with `body`, `createdAt`, `user`, `project` |
| `reorder-issues` → `{updated, unchanged}` | same keys (+ `workspace`) |
| `projects` → `status` is a string, `initiative` the first initiative name | `project list --json`: `status.name`, `initiatives.nodes[].name` |
| `delete-milestone` → `{id, name, deleted}` | same keys (+ `workspace`) |

### Other differences

- **Source attachment title**: without `--source-title` the old tool titles the attachment `出どころ`,
  `linear` titles it `Source`. The URL (the idempotence key) is the same.
- **`--project` can be a name** in every `linear` command. The old `create-issue`, `milestones`,
  `create-milestone`, `status` pass the value straight to Linear as an id (by reading the code), so
  they need the id or slug id; the other old commands also accept a name.
- **Updates send only what differs.** `linear project update` and `milestone update` with the values
  they already have change nothing and say so (`"changed": []`); the old tool always sends the mutation.
- **Ownership rules (new, exit 4)**: `linear` refuses to write a project you do not lead, or an issue
  that is neither assigned to you nor in a project you lead. The old tool has no such check. Not
  exercised here (every scratch object is yours); described in the README and covered by its tests.
- **`template-sections` also covers project bodies by default.** `linear project create --body-file`
  without `--template` is refused with exit 5 when the rule is on. The old tool only checks issues.
  To mirror it, set `rule_operations = { "template-sections" = ["issue_create"] }` for the workspace.
  The harness does.
- **Listing limits**: `linear` lists 50 by default (`--limit`, `--all`); the old `projects`,
  `initiatives` and `issues` returned up to 100 / 100 / all open.
- **Timeouts**: `linear` gives up after 30 s per request and does not retry. One `template create` hit
  that during the run (Linear was slow); the harness retries once, and the second try worked.

## Read comparison on the real workspaces

`scripts/parity/read_compare.py` ran the old read commands and their `linear` equivalents against the
real workspaces (read-only; same login, so the same data), 2026-10-07.

| Comparison | ken109 | lt-three |
| --- | --- | --- |
| open projects (`projects` / `project list --open --all`): set, name, status, target date, URL, initiative | same (8) | same (11) |
| initiatives (`initiatives` / `initiative list --all`): set and name | same (1) | same (7; the old tool leaves out Completed) |
| initiative `description` | **not in `initiative list --json`** (it is in `initiative view`) | same |
| templates (`templates` / `template list`): id, name, type, description | same (3) | same (5) |
| `skeletons` / `template skeleton`, text | identical (27 lines) | identical (27 lines) |
| `template <name>` / `template skeleton <name>` for every issue template | identical (3) | identical (3) |
| `milestones` / `milestone list`: id, name, target date, for every open project | same (8 projects) | same (11 projects) |
| `issues --project` / `issue list --project --open --all`: set, title, state, milestone | same set (47 issues in 8 projects) | the old tool has no `issues` |
| `issues --project` order | **differs in 5 of 8 projects**: the old tool sorts by `sortOrder` (screen order), `linear` uses Linear's default order | n/a |
| `status` / `project list`: which projects the old rule shows (started, or with an update), their health and "no update" marks | same (5 projects) | same (2) |
| `status --project` / `project view`: latest update body, milestone names | same (5 projects) | same (2) |
| `status --project`: earlier updates | old shows 3, `linear` 4 (it fetches 5 and shows them) | same |
| `open-issues` / `issue list --open --all` | n/a | old 63, new 45: the 18 only in the old output are issues closed in the last 14 days (LT3-201 ... LT3-258) that `linear` has no filter for |
| `cycle`, `create-issue --held-on` | n/a | not provided |

Not compared: write commands (nothing is written to a real workspace), and `status --session` (it
only prints when run from the hook).

## Gaps

Severity: **blocks** (the old tool cannot be retired / the workspace cannot switch before this is
done), **acceptable** (a known difference that callers can adapt to), **improvement** (nice to have),
**closed** (resolved; the row stays so that the numbers do not move).

| # | Gap | Severity |
| --- | --- | --- |
| G1 | Closed: `linear brief` replaces `status` (the markdown brief of unfinished projects with health, preview and a stale mark) and `linear brief --session` replaces `status --session` (the SessionStart hook: silent on failure, skipped in CI, 4 s budget). ken109's `session-start.sh` can call it instead of `tools/linear.ts`. | closed |
| G2 | No OAuth: `linear` uses a personal API key (`workspace add --auth oauth` is "not implemented"). Fine for ken109 (the creator is the key's owner, as with the OAuth login), but lt-three's CI creates issues as the app through client credentials (`LINEAR_CLIENT_SECRET`), which `linear` cannot do. | acceptable for ken109; blocks lt-three's CI |
| G3 | lt-three only: `cycle <date>`, `create-issue --held-on` (cycle assignment, also for an existing issue without a cycle) and `open-issues` with recently closed issues have no equivalent. | blocks lt-three's minutes workflow |
| G4 | Ownership rules (always on, exit 4) refuse writes the old tool allowed: updating an issue that is neither assigned to you nor in a project you lead, creating an issue assigned to someone else in a project you do not lead (`--allow-foreign` only covers issues assigned to you). lt-three's workflow creates issues for other members. | **closed**: `ownership = "lenient"` per workspace (default `strict`) allows creating issues for others and changing issues owned by others; projects, and canceling an issue that is not yours, stay refused (see the README) |
| G5 | `issue list` cannot sort by `sortOrder`, the order `issue reorder` writes and the screen shows. The old `issues --project` printed that order; the JSON has `sortOrder`, so `jq 'sort_by(.sortOrder)'` works. | acceptable (workaround); an `--order manual` would remove it |
| G6 | `initiative list --json` has no `description` (the old `initiatives` did); `initiative view` has it. | improvement |
| G7 | No "closed since" filter on `issue list` (`--completed-since`), which lt-three's duplicate check used (14 days). Part of G3. | improvement |
| G8 | Output shape of every write changed (see "Output shape"): skills and hooks that read `state`, `project`, `milestone`, `status` as strings must read `state.name`, ... Messages are English; the exit code is no longer always 1. | acceptable (one-time update of the skills) |
| G9 | The default title of the source attachment is `Source` instead of `出どころ`. | acceptable; a `source_title` default per workspace would keep old issues and new ones alike |
| G10 | `template-sections` is applied to project bodies by default (exit 5 without `--template`); the old tool only checked issues. | acceptable (`rule_operations` mirrors the old behaviour) |
| G11 | `linear` has a fixed 30 s timeout per request and no retry. One slow `templateCreate` failed with `request to Linear failed: timeout: global`; the next try worked. Retrying reads, or `--timeout`, would help. | improvement |
| G12 | Dates are checked more strictly than the old tool (`2027-02-30` is refused instead of being stored as `2027-03-02`). | none: a fix of an old bug; noted so nobody expects the old result |
| G13 | At creation time the position of a new issue in `prioritySortOrder` did not always match the old tool (the first run saw the old tool put the newest issue on top and `linear` leave the first one there). After `issue reorder` both orders match. Not reproduced in isolation; Linear computes it. | improvement (check) |
| G14 | The old tool normalised `create-project --summary` and the body file as required; `linear` makes them optional. | acceptable |

Not done in this check: ownership refusals (every scratch object is yours), `linear audit`, `cache`, `api`,
and `status --session`, which have no old counterpart to compare with.
