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
| `error_unauthenticated.json`| any, with a bad key           | HTTP 401                                           |
| `error_too_complex.json`    | `projects` at 20 per page     | HTTP 400, `INPUT_ERROR`                            |
| `initiatives.json`          | `queries::initiatives`        | **hand-written**: the free plan disables initiatives, so the sandbox cannot produce one |

The live read tests (`crates/cli/tests/live_read.rs`) also rely on this seed data in the sandbox: a second
status update on `Fixture Project` and a completed project without a lead (`Finished Project`), and an issue template with heading
nodes (`Sectioned Template`).
