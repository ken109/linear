//! `linear issue create|update|comment|reorder`.

use super::{read_text, resolve, retry, Rollback, WriteSession, ATTACH_WAITS};
use crate::commands::format::person;
use crate::commands::issue::out as issue_out;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use chrono::NaiveDate;
use clap::Args;
use linear_core::config::Rule;
use linear_core::guard::{Placement, Write};
use linear_core::inputs::{
    self, AttachmentCreate, AttachmentCreateInput, CommentCreate, CommentCreateInput, IssueCreate,
    IssueCreateInput, IssueDelete, IssueUpdate, IssueUpdateInput, Patch,
};
use linear_core::matching::match_state;
use linear_core::read::{self, IssueWriteView};
use linear_core::reorder::{self, OrderRow};
use linear_core::rules::source_attachment;
use linear_core::rules::{Draft, Operation, Outcome};
use linear_core::types::Issue;
use serde::Serialize;
use std::path::PathBuf;

/// The title an origin attachment gets when `--source-title` is not given.
const DEFAULT_SOURCE_TITLE: &str = "Source";

// ---------------------------------------------------------------- create

#[derive(Debug, Args)]
pub struct CreateCmd {
    /// The issue's title
    #[arg(long, value_name = "TITLE")]
    pub title: String,
    /// Project to create it in: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
    /// Read the description from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear template the description must follow (checked by the
    /// `template-sections` rule)
    #[arg(long, value_name = "NAME")]
    pub template: Option<String>,
    /// Where the issue came from: an http(s) URL, attached to the issue. With
    /// the `source-attachment` rule it is required, and an issue that already
    /// carries it is returned instead of creating another
    #[arg(long, value_name = "URL")]
    pub source: Option<String>,
    /// Title of the source attachment
    #[arg(long, value_name = "TITLE", requires = "source")]
    pub source_title: Option<String>,
    /// Milestone of the project, by name
    #[arg(long, value_name = "NAME")]
    pub milestone: Option<String>,
    /// Assignee: `me`, an email or a name (default: you)
    #[arg(long, value_name = "WHO")]
    pub assignee: Option<String>,
    /// Label name (repeatable)
    #[arg(long, value_name = "NAME")]
    pub label: Vec<String>,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Allow creating, in a project somebody else leads, an issue assigned to you
    #[arg(long)]
    pub allow_foreign: bool,
}

/// What `create` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    id: &'a str,
    identifier: &'a str,
    url: &'a str,
    /// `true` when an issue with the same source already existed and nothing was created.
    existing: bool,
    source_url: Option<&'a str>,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    // Everything that can be judged from the arguments alone comes first.
    let body = cmd.body_file.as_deref().map(read_text).transpose()?;
    let source = cmd.source.as_deref().map(str::trim);

    let ws = ctx.write_session()?;

    // An origin that is not an http(s) URL is never attached. With the
    // `source-attachment` rule on, the validators report it (exit 5) together
    // with any other violation; without it, it is a usage error.
    if let Some(s) = source {
        if !ws
            .rules
            .applies(Rule::SourceAttachment, Operation::IssueCreate)
            && source_attachment::validate(Some(s)).is_err()
        {
            return Err(CliError::usage(format!(
                "the source must be an http(s) URL, got {s:?}"
            )));
        }
    }

    // Resolve names to ids (read-only; unknown names stop here).
    let team = resolve::team(&ws, cmd.team.as_deref())?;
    let project = resolve::project(&ws, &cmd.project)?;
    let milestone = cmd
        .milestone
        .as_deref()
        .map(|m| resolve::milestone(&project, m))
        .transpose()?;
    let assignee = match cmd.assignee.as_deref() {
        Some(who) => resolve::user_id(&ws, who)?,
        None => ws.viewer.id.clone(),
    };
    let labels = resolve::labels(&ws, &cmd.label)?;

    // Guard, then validators.
    ws.guard(
        &Write::IssueCreate {
            assignee: Some(&assignee),
            placement: project.placement(),
        },
        cmd.allow_foreign,
    )?;
    if cmd.template.is_some()
        && !ws
            .rules
            .applies(Rule::TemplateSections, Operation::IssueCreate)
    {
        ws.note(
            "--template is ignored: the template-sections rule is not enabled for this workspace",
        );
    }
    let mut draft = Draft::new(Operation::IssueCreate).labels(labels.clone());
    if let Some(t) = &cmd.template {
        draft = draft.template(t);
    }
    if let Some(b) = &body {
        draft = draft.body(b);
    }
    if let Some(s) = source {
        draft = draft.source(s);
    }
    if let Outcome::AlreadyExists(existing) = ws.validate(&draft)? {
        emit_created(
            ctx,
            &ws,
            existing.id.inner(),
            &existing.identifier,
            &existing.url,
            true,
            source,
        );
        return Ok(());
    }

    // Mutate: create, then attach the origin; a failed attachment takes the issue with it.
    let input = IssueCreateInput {
        team_id: team.id.inner().to_owned(),
        title: Some(cmd.title.clone()),
        description: body.as_ref().map(|b| b.trim_end().to_owned()),
        assignee_id: Some(assignee),
        project_id: Some(project.id.inner().to_owned()),
        project_milestone_id: milestone.map(|m| m.id.inner().to_owned()),
        label_ids: (!labels.is_empty())
            .then(|| labels.iter().map(|l| l.id.inner().to_owned()).collect()),
    };
    let data: IssueCreate = ws.client.execute(&inputs::issue_create(input))?;
    let issue = match data.issue_create {
        p if p.success => p
            .issue
            .ok_or_else(|| CliError::general("Linear created the issue but returned none"))?,
        _ => return Err(CliError::general("Linear could not create the issue")),
    };

    if let Some(source) = source {
        let mut rollback = Rollback::new();
        rollback.on_failure(format!("deleted {}", issue.identifier), || {
            let r: IssueDelete = ws.client.execute(&inputs::issue_delete(issue.id.inner()))?;
            if r.issue_delete.success {
                Ok(())
            } else {
                Err(CliError::general("Linear refused to delete it"))
            }
        });
        let attached = retry(ws.out, "attaching the source", &ATTACH_WAITS, || {
            let input = AttachmentCreateInput {
                issue_id: issue.id.inner().to_owned(),
                url: source.to_owned(),
                title: cmd
                    .source_title
                    .clone()
                    .unwrap_or_else(|| DEFAULT_SOURCE_TITLE.to_owned()),
                subtitle: None,
                metadata: None,
            };
            let r: AttachmentCreate = ws.client.execute(&inputs::attachment_create(input))?;
            if r.attachment_create.success {
                Ok(())
            } else {
                Err(CliError::general("Linear could not attach the source"))
            }
        });
        if let Err(cause) = attached {
            return Err(rollback.fail(cause));
        }
    }

    emit_created(
        ctx,
        &ws,
        issue.id.inner(),
        &issue.identifier,
        &issue.url,
        false,
        source,
    );
    Ok(())
}

fn emit_created(
    ctx: &Ctx,
    ws: &WriteSession,
    id: &str,
    identifier: &str,
    url: &str,
    existing: bool,
    source: Option<&str>,
) {
    let value = Created {
        workspace: &ws.workspace,
        id,
        identifier,
        url,
        existing,
        source_url: source,
    };
    ctx.out.emit(
        &value,
        || {
            let how = if existing {
                "already exists, nothing created"
            } else {
                "created"
            };
            format!("{identifier}  {url}  ({how})")
        },
        || identifier.to_owned(),
    );
}

// ---------------------------------------------------------------- update

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    /// Workflow state, by name (ignoring case)
    #[arg(long, value_name = "NAME")]
    pub state: Option<String>,
    /// Move to this project: id, slug id, URL or name. The milestone is looked up in
    /// the new project; without --milestone it is cleared
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,
    /// Milestone of the (new) project, by name
    #[arg(long, value_name = "NAME")]
    pub milestone: Option<String>,
    /// Due date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub due: Option<NaiveDate>,
    /// Assignee: `me`, an email or a name
    #[arg(long, value_name = "WHO")]
    pub assignee: Option<String>,
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    if cmd.state.is_none()
        && cmd.project.is_none()
        && cmd.milestone.is_none()
        && cmd.due.is_none()
        && cmd.assignee.is_none()
    {
        return Err(CliError::usage(
            "nothing to change: pass --state, --project, --milestone, --due or --assignee",
        ));
    }

    let ws = ctx.write_session()?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    let issue = &view.issue;
    let current = view.write.project.as_ref();

    // Resolve names to ids.
    let mut input = IssueUpdateInput::default();
    if let Some(name) = &cmd.state {
        let state = match_state(&view.write.team.states, name)?;
        input.state_id = Some(state.id.inner().to_owned());
    }
    // The project the milestone is looked up in: the destination when moving, else the current one.
    let mut target = None;
    let mut milestones_of = current;
    if let Some(reference) = &cmd.project {
        let dest = resolve::project(&ws, reference)?;
        if Some(dest.id.inner()) != current.map(|p| p.id.inner()) {
            input.project_id = Some(dest.id.inner().to_owned());
            if cmd.milestone.is_none() {
                // The old project's milestone would be left dangling in the new one.
                input.project_milestone_id = Patch::Clear;
            }
            target = Some(dest);
        }
    }
    if target.is_some() {
        milestones_of = target.as_ref();
    }
    if let Some(name) = &cmd.milestone {
        let project = milestones_of.ok_or_else(|| {
            CliError::usage(format!(
                "{} is not in a project, so it cannot have a milestone",
                issue.identifier
            ))
        })?;
        input.project_milestone_id =
            Patch::Set(resolve::milestone(project, name)?.id.inner().to_owned());
    }
    input.due_date = cmd.due;
    if let Some(who) = &cmd.assignee {
        input.assignee_id = Some(resolve::user_id(&ws, who)?);
    }

    // Guard, then validators.
    ws.guard(
        &Write::IssueUpdate {
            assignee: issue.assignee.as_ref().map(|u| u.id.inner()),
            placement: placement_of(&view),
            moves_to: target.as_ref().map(|p| p.placement()),
        },
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;

    let data: IssueUpdate = ws
        .client
        .execute(&inputs::issue_update(issue.id.inner(), input))?;
    let updated = changed_issue(data.issue_update, &issue.identifier)?;

    show_issue(ctx, &ws, &updated, "updated");
    Ok(())
}

fn show_issue(ctx: &Ctx, ws: &WriteSession, issue: &Issue, verb: &str) {
    let value = issue_out(&ws.workspace, issue);
    ctx.out.emit(
        &value,
        || {
            format!(
                "{}  {}  {}\n{}  ({verb}; {}, {})",
                issue.identifier,
                issue.state.name,
                issue.title,
                issue.url,
                issue
                    .project
                    .as_ref()
                    .map_or("no project".to_owned(), |p| p.name.clone()),
                person(&issue.assignee),
            )
        },
        || issue.identifier.clone(),
    );
}

// ---------------------------------------------------------------- comment

#[derive(Debug, Args)]
pub struct CommentCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    /// Read the comment from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: PathBuf,
}

/// What `comment` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Commented<'a> {
    workspace: &'a str,
    id: &'a str,
    url: &'a str,
    issue: &'a str,
}

pub fn comment(ctx: &Ctx, cmd: &CommentCmd) -> Result<()> {
    let body = read_text(&cmd.body_file)?;
    if body.trim().is_empty() {
        return Err(CliError::usage("the comment is empty"));
    }

    let ws = ctx.write_session()?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    // A comment is a write to the issue: it follows the same ownership as changing it.
    ws.guard(
        &Write::IssueUpdate {
            assignee: view.issue.assignee.as_ref().map(|u| u.id.inner()),
            placement: placement_of(&view),
            moves_to: None,
        },
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;

    let input = CommentCreateInput {
        issue_id: view.issue.id.inner().to_owned(),
        body: body.trim().to_owned(),
    };
    let data: CommentCreate = ws.client.execute(&inputs::comment_create(input))?;
    if !data.comment_create.success {
        return Err(CliError::general("Linear could not write the comment"));
    }
    let comment = data.comment_create.comment;
    let value = Commented {
        workspace: &ws.workspace,
        id: comment.id.inner(),
        url: &comment.url,
        issue: &view.issue.identifier,
    };
    ctx.out.emit(
        &value,
        || format!("{}  {}", view.issue.identifier, comment.url),
        || comment.url.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- reorder

#[derive(Debug, Args)]
pub struct ReorderCmd {
    /// The issues in the order they should end up, top first (identifiers,
    /// space- or comma-separated). They must all be in the same project
    #[arg(required = true, num_args = 2.., value_delimiter = ',', value_name = "ISSUE")]
    pub issues: Vec<String>,
}

/// What `reorder` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reordered<'a> {
    workspace: &'a str,
    /// The issues that were written, in the requested order.
    updated: Vec<&'a str>,
    /// `true` when they already sat in the requested order.
    unchanged: bool,
}

pub fn reorder(ctx: &Ctx, cmd: &ReorderCmd) -> Result<()> {
    let ws = ctx.write_session()?;

    let views = cmd
        .issues
        .iter()
        .map(|reference| fetch_issue(&ws, reference))
        .collect::<Result<Vec<_>>>()?;

    // Only issues of one project have values that can be compared.
    let project_id = |v: &IssueWriteView| v.write.project.as_ref().map(|p| p.id.inner().to_owned());
    let first = project_id(&views[0]);
    if first.is_none() || views.iter().any(|v| project_id(v) != first) {
        return Err(CliError::usage(
            "only issues in the same project can be reordered",
        ));
    }

    // Every issue must be writable before the first one is written.
    for view in &views {
        ws.guard(
            &Write::IssueUpdate {
                assignee: view.issue.assignee.as_ref().map(|u| u.id.inner()),
                placement: placement_of(view),
                moves_to: None,
            },
            false,
        )?;
    }
    ws.validate(&Draft::new(Operation::IssueUpdate))?;

    let rows: Vec<OrderRow> = views
        .iter()
        .map(|v| OrderRow {
            identifier: v.issue.identifier.clone(),
            sort_order: v.issue.sort_order,
            priority_sort_order: v.issue.priority_sort_order,
        })
        .collect();
    let wanted: Vec<String> = rows.iter().map(|r| r.identifier.clone()).collect();
    let plan = reorder::plan(&rows, &wanted)?;

    // Each write is undone (back to the old values) if a later one fails.
    let mut rollback = Rollback::new();
    for change in &plan {
        let view = views
            .iter()
            .find(|v| v.issue.identifier == change.identifier)
            .expect("the plan only names the issues given");
        let input = IssueUpdateInput {
            sort_order: change.sort_order,
            priority_sort_order: change.priority_sort_order,
            ..Default::default()
        };
        let step = ws
            .client
            .execute::<_, _, IssueUpdate>(&inputs::issue_update(view.issue.id.inner(), input))
            .and_then(|d| changed_issue(d.issue_update, &change.identifier).map(|_| ()));
        if let Err(cause) = step {
            return Err(rollback.fail(cause));
        }
        let (client, issue) = (&ws.client, &view.issue);
        let restore = IssueUpdateInput {
            sort_order: change.sort_order.map(|_| issue.sort_order),
            priority_sort_order: change
                .priority_sort_order
                .map(|_| issue.priority_sort_order),
            ..Default::default()
        };
        rollback.on_failure(format!("restored {}", issue.identifier), move || {
            client
                .execute::<_, _, IssueUpdate>(&inputs::issue_update(issue.id.inner(), restore))
                .map(|_| ())
        });
    }

    let value = Reordered {
        workspace: &ws.workspace,
        updated: plan.iter().map(|c| c.identifier.as_str()).collect(),
        unchanged: plan.is_empty(),
    };
    ctx.out.emit(
        &value,
        || {
            if plan.is_empty() {
                "Already in that order; nothing changed.".to_owned()
            } else {
                format!("Reordered: {}", wanted.join(", "))
            }
        },
        || {
            plan.iter()
                .map(|c| c.identifier.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

// ---------------------------------------------------------------- shared

/// The issue as a write needs to see it: its fields, its team's states and its project's lead.
fn fetch_issue(ws: &WriteSession, reference: &str) -> Result<IssueWriteView> {
    ws.client.execute(&read::issue_write_view(reference.trim()))
}

fn placement_of(view: &IssueWriteView) -> Placement<'_> {
    match &view.write.project {
        Some(project) => project.placement(),
        None => Placement::NoProject,
    }
}

/// The issue a successful `issueUpdate` returned.
fn changed_issue(payload: inputs::IssuePayload, identifier: &str) -> Result<Issue> {
    if !payload.success {
        return Err(CliError::general(format!(
            "Linear could not update {identifier}"
        )));
    }
    payload
        .issue
        .ok_or_else(|| CliError::general(format!("Linear updated {identifier} but returned none")))
}
