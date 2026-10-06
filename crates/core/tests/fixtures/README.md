# Fixtures

Responses captured from a real (empty-then-seeded) Linear sandbox workspace, with names,
emails, ids and URL keys replaced by stable placeholders (`Alice Example`,
`alice@example.com`, `00000000-0000-4000-8000-0000000000NN`, `example`, team `EX`).
Structure, nullability and value formats are exactly Linear's.

| File                        | Query                         | Notes                                              |
| --------------------------- | ----------------------------- | -------------------------------------------------- |
| `whoami.json`               | `queries::whoami`             |                                                    |
| `issue.json`                | `queries::issue`              | project, milestone, grouped labels, attachment     |
| `assigned_issues.json`      | `queries::assigned_started_issues` |                                               |
| `projects.json`             | `queries::projects`           | status update, milestone                           |
| `issue_list.json`           | `read::issue_list`            | `assigned_issues.json`'s page under `issues`       |
| `issue_view.json`           | `read::issue_view`            | `issue.json` plus the live shape of the `detail` alias (description, priority, comments) |
| `project_view.json`         | `read::project_view`          | `projects.json` with a second, newer status update (newest first) and an initiative; `detail` shape verified live |
| `milestones.json`           | `read::milestones_of_project` | `issue.json`'s milestone plus a second one, out of order |
| `milestone_view.json`       | `read::milestone_view`        | `detail` alias: the milestone's issues             |
| `initiative_view.json`      | `read::initiative_view`       | **hand-written** (initiatives are disabled on the free plan) |
| `labels.json`               | `read::labels`                | a group, a child and a plain label, in creation order |
| `teams.json`, `users.json`  | `read::teams`, `read::users`  | `users.json` includes Linear's own bot user        |
| `templates_sections.json`   | `queries::templates`          | `templates.json` plus a template with real heading nodes and a project template |
| `issue_comments.json`       | `queries::issue_comments`     |                                                    |
| `templates.json`            | `queries::templates`          | `templateData` is a JSON document inside a string  |
| `issue_update.json`         | `inputs::issue_update`        |                                                    |
| `issue_write_view.json`     | `read::issue_write_view`      | **derived**: `issue.json` plus the `write` alias (team states, project with lead and milestone, `cycle`: null); shape verified against the sandbox |
| `attachments_for_url.json`, `attachments_for_url_none.json` | `read::attachments_for_url` | **derived** from `issue.json`; shape verified against the sandbox |
| `cycles.json`               | `read::cycles`                | **hand-written**: the sandbox team has no cycles. The shape (fields, nullability) was checked against lt-three's real cycles, read-only; cycle #41 is Tue 2026-10-05T15:00Z, a week long |
| `error_unauthenticated.json`| any, with a bad key           | HTTP 401                                           |
| `error_too_complex.json`    | `projects` at 20 per page     | HTTP 400, `INPUT_ERROR`                            |
| `initiatives.json`          | `queries::initiatives`        | **hand-written**: the free plan disables initiatives, so the sandbox cannot produce one |

The live read tests (`crates/cli/tests/live_read.rs`) also rely on this seed data in the sandbox: a second
status update on `Fixture Project` and a completed project without a lead (`Finished Project`), and an issue template with heading
nodes (`Sectioned Template`).

The live write test (`crates/cli/tests/live_write.rs`) relies on `Sectioned Template`, `Fixture Project`
(led by the key's owner) and `Finished Project` (no lead, so writes to it are refused). It cancels the issues it creates.

The live attachment metadata test (`crates/cli/tests/live_attachment_meta.rs`) uses `Fixture Project` only. It creates one issue
per run with a unique source URL (and metadata on its attachment) and cancels it at the end, so repeated runs leave only canceled
issues behind; it needs no other seed data.

The live structure test (`crates/cli/tests/live_structure.rs`) uses `Fixture Project` (and `Milestone 1`, which must keep an
issue in it: the test checks that such a milestone cannot be deleted) and `Finished Project`. It removes the milestones it
creates. It creates the issue template `Live Created Template` once and leaves it (the CLI cannot delete a template); later runs
find it. Initiatives cannot be created on the free plan, so the test only checks that Linear's refusal is reported.

The live audit test (`crates/cli/tests/live_audit.rs`) relies on the projects, milestone and issues named
`audit-seed ...` that `scripts/seed-sandbox-audit.py` plants (an overdue project without a lead, a completed project with
an open issue, issues that are late, half-filled, canceled and duplicate). Do not delete them.

The `description` and `canceledAt` of the issues in `issue.json`, `issue_view.json`, `issue_list.json`,
`assigned_issues.json` and `issue_update.json` were added by hand (the captures predate selecting them; the
`description` is the one the live `Write the fixture issue` has).

The `metadata` of the attachments in `issue.json`, `issue_view.json`, `issue_list.json`, `assigned_issues.json`,
`issue_update.json` and `issue_write_view.json`, and the `title`, `subtitle` and `metadata` of `attachments_for_url.json`, were
added by hand (the captures predate selecting them); `metadata` is the empty object, as Linear returns it for a plain attachment.

The `sortOrder` and `prioritySortOrder` of the issues in `issue.json`, `issue_view.json`, `issue_list.json`,
`assigned_issues.json` and `issue_update.json` were added by hand (the captures predate selecting them).
