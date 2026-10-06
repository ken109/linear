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
| `issue_comments.json`       | `queries::issue_comments`     |                                                    |
| `templates.json`            | `queries::templates`          | `templateData` is a JSON document inside a string  |
| `issue_update.json`         | `inputs::issue_update`        |                                                    |
| `error_unauthenticated.json`| any, with a bad key           | HTTP 401                                           |
| `error_too_complex.json`    | `projects` at 20 per page     | HTTP 400, `INPUT_ERROR`                            |
| `initiatives.json`          | `queries::initiatives`        | **hand-written**: the free plan disables initiatives, so the sandbox cannot produce one |
