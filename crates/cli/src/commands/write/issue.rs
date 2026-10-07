//! `linear issue create|update|comment|reorder`.
//!
//! `update` is the write that can touch the most: fields of the issue, its
//! description, its labels and its source attachment. It sends only what
//! differs from now, and puts the issue's fields back if the attachment, the
//! last step, cannot be made.

use super::{read_text, resolve, retry, Rollback, WriteSession, ATTACH_WAITS};
use crate::commands::cycle;
use crate::commands::format::person;
use crate::commands::issue::{out as issue_out, IssueOut};
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
use linear_core::markdown::same_description;
use linear_core::matching::match_state;
use linear_core::metadata::AttachmentMetadata;
use linear_core::read::{self, IssueWriteView};
use linear_core::reorder::{self, OrderRow};
use linear_core::rules::source_attachment::{self, metadata_needs_update};
use linear_core::rules::{Draft, Operation, Outcome};
use linear_core::types::{Cycle, Issue, Label, StateType};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

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
    /// Title of the source attachment (default: the workspace's `source_title`, or `Source`)
    #[arg(long, value_name = "TITLE", requires = "source")]
    pub source_title: Option<String>,
    /// Metadata of the source attachment, KEY=VALUE (repeatable). A value that
    /// reads as a number (42, 3.5, 1e3) is sent as a number; write KEY=str:123 to
    /// send the text "123". Values are flat: strings and numbers only. If an issue
    /// with the same source already exists and the metadata differs, the stored
    /// metadata is replaced (`metadataUpdated`); identical metadata sends nothing
    #[arg(long = "meta", value_name = "KEY=VALUE", requires = "source")]
    pub meta: Vec<String>,
    /// Milestone of the project, by name
    #[arg(long, value_name = "NAME")]
    pub milestone: Option<String>,
    /// Assignee: `me`, an email or a name (default: you)
    #[arg(long, value_name = "WHO")]
    pub assignee: Option<String>,
    /// Label name (repeatable)
    #[arg(long, value_name = "NAME")]
    pub label: Vec<String>,
    /// The day of the meeting the issue came out of (YYYY-MM-DD): the issue goes into the
    /// cycle that contains the day after it (see `linear cycle`), and the command fails
    /// before creating anything when there is none. If an issue with the same --source
    /// already exists (the `source-attachment` rule looks for it), its cycle is set only
    /// when it has none; its other fields are not touched
    #[arg(long, value_name = "DATE")]
    pub held_on: Option<NaiveDate>,
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
    /// `true` when the issue already existed and `--meta` replaced the metadata of its source attachment.
    metadata_updated: bool,
    /// The cycle the issue is in, when `--held-on` asked for one (an existing issue that
    /// already had another cycle keeps it, and this is that one).
    #[serde(skip_serializing_if = "Option::is_none")]
    cycle: Option<&'a Cycle>,
    /// The fields this run wrote onto an issue that already existed (`cycle`); empty for a new one.
    changed: Vec<&'static str>,
}

pub fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    // Everything that can be judged from the arguments alone comes first.
    let body = cmd.body_file.as_deref().map(read_text).transpose()?;
    let source = cmd.source.as_deref().map(str::trim);
    let metadata = if cmd.meta.is_empty() {
        None
    } else {
        Some(
            AttachmentMetadata::from_pairs(&cmd.meta)
                .map_err(|e| CliError::usage(e.to_string()))?,
        )
    };

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
    // A day no cycle contains stops here, before anything is written.
    let cycle = cmd
        .held_on
        .map(|day| cycle::find(&ws.client, &team.key, day))
        .transpose()?;

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
    if let Some(m) = &metadata {
        draft = draft.source_metadata(m.clone());
    }
    if let Outcome::AlreadyExists(existing) = ws.validate(&draft)? {
        // The source is already attached: nothing is created, but metadata
        // that differs from what is stored is written onto that attachment.
        let metadata_updated = match (source, &metadata) {
            (Some(url), Some(wanted)) => refresh_source_metadata(
                &ws,
                existing.id.inner(),
                url,
                cmd.source_title.as_deref(),
                wanted,
            )?,
            _ => false,
        };
        // The cycle is the one thing aligned afterwards, and only when it is empty.
        let (cycle, changed) = match &cycle {
            Some(wanted) => put_in_cycle(&ws, existing.id.inner(), wanted)?,
            None => (None, Vec::new()),
        };
        emit_created(
            ctx,
            &ws,
            Made {
                id: existing.id.inner(),
                identifier: &existing.identifier,
                url: &existing.url,
                existing: true,
                metadata_updated,
                cycle: cycle.as_ref(),
                changed,
            },
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
        cycle_id: cycle.as_ref().map(|c| c.id.inner().to_owned()),
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
                    .unwrap_or_else(|| ws.source_title.clone()),
                subtitle: None,
                metadata: metadata.clone(),
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
        Made {
            id: issue.id.inner(),
            identifier: &issue.identifier,
            url: &issue.url,
            existing: false,
            metadata_updated: false,
            cycle: cycle.as_ref(),
            changed: Vec::new(),
        },
        source,
    );
    Ok(())
}

/// Put an issue that already exists in `wanted`, but only if it is in no cycle: a cycle
/// somebody chose is not taken back. Returns the cycle the issue is in afterwards and what
/// was written (`cycle`, or nothing).
///
/// This is a write to an existing issue, so it takes the whole path: the ownership guard
/// for an update, the update validators, then the one mutation.
fn put_in_cycle(
    ws: &WriteSession,
    issue_id: &str,
    wanted: &Cycle,
) -> Result<(Option<Cycle>, Vec<&'static str>)> {
    let view = fetch_issue(ws, issue_id)?;
    if let Some(current) = view.write.cycle.clone() {
        if current.id != wanted.id {
            ws.note(&format!(
                "{} is already in cycle {}; it was not moved to {}",
                view.issue.identifier,
                linear_core::cycle::label(&current),
                linear_core::cycle::label(wanted)
            ));
        }
        return Ok((Some(current), Vec::new()));
    }
    ws.guard(
        &Write::IssueUpdate {
            assignee: view.issue.assignee.as_ref().map(|u| u.id.inner()),
            placement: placement_of(&view),
            moves_to: None,
        },
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;
    let input = IssueUpdateInput {
        cycle_id: Some(wanted.id.inner().to_owned()),
        ..Default::default()
    };
    let data: IssueUpdate = ws
        .client
        .execute(&inputs::issue_update(view.issue.id.inner(), input))?;
    changed_issue(data.issue_update, &view.issue.identifier)?;
    Ok((Some(wanted.clone()), vec!["cycle"]))
}

/// What `create` found or made, for `emit_created`.
struct Made<'a> {
    id: &'a str,
    identifier: &'a str,
    url: &'a str,
    /// An issue with the same source already existed and nothing was created.
    existing: bool,
    metadata_updated: bool,
    cycle: Option<&'a Cycle>,
    changed: Vec<&'static str>,
}

/// Write `wanted` onto the attachment that holds `url`, if it differs from
/// what that attachment stores. Linear upserts on the URL, so this is the same
/// `attachmentCreate` that attaches the source in the first place, sent with
/// the stored title (and subtitle) so only the metadata changes. Identical
/// metadata sends nothing. Returns whether anything was written.
fn refresh_source_metadata(
    ws: &WriteSession,
    issue_id: &str,
    url: &str,
    title: Option<&str>,
    wanted: &AttachmentMetadata,
) -> Result<bool> {
    let data: read::AttachmentsForUrlQuery = ws.client.execute(&read::attachments_for_url(url))?;
    let Some(stored) = data.attachments_for_url.nodes.into_iter().next() else {
        return Ok(false);
    };
    if !metadata_needs_update(Some(wanted), &stored.metadata) {
        return Ok(false);
    }
    let input = AttachmentCreateInput {
        issue_id: issue_id.to_owned(),
        url: url.to_owned(),
        title: title.map_or(stored.title, str::to_owned),
        subtitle: stored.subtitle,
        metadata: Some(wanted.clone()),
    };
    let r: AttachmentCreate = ws.client.execute(&inputs::attachment_create(input))?;
    if r.attachment_create.success {
        Ok(true)
    } else {
        Err(CliError::general(
            "Linear could not update the metadata of the source attachment",
        ))
    }
}

fn emit_created(ctx: &Ctx, ws: &WriteSession, made: Made<'_>, source: Option<&str>) {
    let Made {
        id,
        identifier,
        url,
        existing,
        metadata_updated,
        cycle,
        changed,
    } = made;
    let value = Created {
        workspace: &ws.workspace,
        id,
        identifier,
        url,
        existing,
        source_url: source,
        metadata_updated,
        cycle,
        changed: changed.clone(),
    };
    ctx.out.emit(
        &value,
        || {
            let mut how = match (existing, metadata_updated) {
                (false, _) => "created".to_owned(),
                (true, false) => "already exists, nothing created".to_owned(),
                (true, true) => {
                    "already exists, nothing created; source metadata updated".to_owned()
                }
            };
            if let Some(c) = cycle {
                let verb = if changed.contains(&"cycle") {
                    "cycle set to"
                } else {
                    "in cycle"
                };
                how.push_str(&format!("; {verb} {}", linear_core::cycle::label(c)));
            }
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
    /// Replace the description with the contents of a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// The Linear template the new description must follow (checked by the
    /// `template-sections` rule, which requires it to replace a description)
    #[arg(long, value_name = "NAME", requires = "body_file")]
    pub template: Option<String>,
    /// Attach this http(s) URL as the issue's source. An attachment with the same URL
    /// is updated, never duplicated; one that is already as asked sends nothing. With
    /// the `source-attachment` rule, a URL that another issue carries is refused
    #[arg(long, value_name = "URL")]
    pub source: Option<String>,
    /// Title of the source attachment (default: the stored title; for a new
    /// attachment, the workspace's `source_title`, or `Source`)
    #[arg(long, value_name = "TITLE", requires = "source")]
    pub source_title: Option<String>,
    /// Metadata of the source attachment, KEY=VALUE (repeatable), read as in
    /// `issue create`: a number is sent as a number, KEY=str:123 sends the text "123".
    /// The stored metadata is replaced, not merged; identical metadata sends nothing
    #[arg(long = "meta", value_name = "KEY=VALUE", requires = "source")]
    pub meta: Vec<String>,
    /// Set the labels the issue has to exactly these, by name (repeatable or
    /// comma-separated); the labels it has now are dropped
    #[arg(
        long,
        visible_alias = "label",
        value_name = "NAME",
        value_delimiter = ',',
        conflicts_with_all = ["add_labels", "remove_labels"]
    )]
    pub labels: Vec<String>,
    /// Add these labels to the ones the issue has (repeatable or comma-separated)
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub add_labels: Vec<String>,
    /// Remove these labels from the ones the issue has (repeatable or comma-separated);
    /// one it does not have is ignored
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub remove_labels: Vec<String>,
}

/// What `update` prints: the issue as it is now, and which fields were written.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated<'a> {
    #[serde(flatten)]
    issue: IssueOut<'a>,
    /// The fields this run changed (empty: everything already was as asked, nothing was sent).
    changed: Vec<&'static str>,
}

/// An old value of an optional field, as the patch that puts it back.
fn put_back<T>(old: Option<T>) -> Patch<T> {
    match old {
        Some(v) => Patch::Set(v),
        None => Patch::Clear,
    }
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    let relabel =
        !cmd.labels.is_empty() || !cmd.add_labels.is_empty() || !cmd.remove_labels.is_empty();
    if cmd.state.is_none()
        && cmd.project.is_none()
        && cmd.milestone.is_none()
        && cmd.due.is_none()
        && cmd.assignee.is_none()
        && cmd.body_file.is_none()
        && cmd.source.is_none()
        && !relabel
    {
        return Err(CliError::usage(
            "nothing to change: pass --state, --project, --milestone, --due, --assignee, \
             --body-file, --source, --labels, --add-labels or --remove-labels",
        ));
    }
    // Everything that can be judged from the arguments alone comes first.
    let body = cmd.body_file.as_deref().map(read_text).transpose()?;
    if body.as_deref().is_some_and(|b| b.trim().is_empty()) {
        return Err(CliError::usage("the body file is empty"));
    }
    let source = cmd.source.as_deref().map(str::trim);
    let metadata = if cmd.meta.is_empty() {
        None
    } else {
        Some(
            AttachmentMetadata::from_pairs(&cmd.meta)
                .map_err(|e| CliError::usage(e.to_string()))?,
        )
    };

    let ws = ctx.write_session()?;
    // An origin that is not an http(s) URL is never attached. With the
    // `source-attachment` rule on, the validators report it (exit 5) together
    // with any other violation; without it, it is a usage error.
    if let Some(s) = source {
        if !ws
            .rules
            .applies(Rule::SourceAttachment, Operation::IssueUpdate)
            && source_attachment::validate(Some(s)).is_err()
        {
            return Err(CliError::usage(format!(
                "the source must be an http(s) URL, got {s:?}"
            )));
        }
    }
    let view = fetch_issue(&ws, &cmd.issue)?;
    let issue = &view.issue;
    let current = view.write.project.as_ref();

    // Resolve names to ids (read-only; unknown names stop here).
    let state = cmd
        .state
        .as_deref()
        .map(|name| match_state(&view.write.team.states, name))
        .transpose()?;
    // The project the milestone is looked up in: the destination when moving, else the current one.
    let mut target = None;
    if let Some(reference) = &cmd.project {
        let dest = resolve::project(&ws, reference)?;
        if Some(dest.id.inner()) != current.map(|p| p.id.inner()) {
            target = Some(dest);
        }
    }
    let milestone = match &cmd.milestone {
        Some(name) => {
            let project = target.as_ref().or(current).ok_or_else(|| {
                CliError::usage(format!(
                    "{} is not in a project, so it cannot have a milestone",
                    issue.identifier
                ))
            })?;
            Some(resolve::milestone(project, name)?)
        }
        None => None,
    };
    let assignee = cmd
        .assignee
        .as_deref()
        .map(|who| resolve::user_id(&ws, who))
        .transpose()?;
    let labels = if relabel {
        Some(labels_after(&ws, issue, cmd)?)
    } else {
        None
    };

    // Guard, then validators.
    ws.guard(
        &Write::IssueUpdate {
            assignee: issue.assignee.as_ref().map(|u| u.id.inner()),
            placement: placement_of(&view),
            moves_to: target.as_ref().map(|p| p.placement()),
        },
        false,
    )?;
    // Closing somebody else's work stays refused even where the rules are lenient.
    if state.is_some_and(|s| {
        s.id != issue.state.id
            && matches!(s.state_type(), StateType::Canceled | StateType::Duplicate)
    }) {
        ws.guard(
            &Write::IssueCancel {
                assignee: issue.assignee.as_ref().map(|u| u.id.inner()),
                placement: placement_of(&view),
            },
            false,
        )?;
    }
    if cmd.template.is_some()
        && !ws
            .rules
            .applies(Rule::TemplateSections, Operation::IssueUpdate)
    {
        ws.note(
            "--template is ignored: the template-sections rule is not enabled for this workspace",
        );
    }
    let mut draft = Draft::new(Operation::IssueUpdate).issue(issue.id.inner());
    if let Some(t) = &cmd.template {
        draft = draft.template(t);
    }
    if let Some(b) = &body {
        draft = draft.body(b);
    }
    if let Some(s) = source {
        draft = draft.source(s);
    }
    if let Some(m) = &metadata {
        draft = draft.source_metadata(m.clone());
    }
    if let Some(l) = &labels {
        draft = draft.labels(l.clone());
    }
    ws.validate(&draft)?;

    // What to write, and what puts it back: only the fields that actually differ.
    let mut input = IssueUpdateInput::default();
    let mut restore = IssueUpdateInput::default();
    let mut changed: Vec<&'static str> = Vec::new();
    if let Some(b) = &body {
        let b = b.trim_end();
        if !same_description(issue.description.as_deref(), b) {
            input.description = Patch::Set(b.to_owned());
            restore.description = put_back(issue.description.clone());
            changed.push("description");
        }
    }
    if let Some(s) = state.filter(|s| s.id != issue.state.id) {
        input.state_id = Some(s.id.inner().to_owned());
        restore.state_id = Some(issue.state.id.inner().to_owned());
        changed.push("state");
    }
    let old_project = issue.project.as_ref().map(|p| p.id.inner().to_owned());
    let old_milestone = issue
        .project_milestone
        .as_ref()
        .map(|m| m.id.inner().to_owned());
    if let Some(dest) = &target {
        input.project_id = Patch::Set(dest.id.inner().to_owned());
        restore.project_id = put_back(old_project);
        changed.push("project");
        if milestone.is_none() && old_milestone.is_some() {
            // The old project's milestone would be left dangling in the new one.
            input.project_milestone_id = Patch::Clear;
            restore.project_milestone_id = put_back(old_milestone.clone());
            changed.push("milestone");
        }
    }
    if let Some(m) = milestone {
        let moved = target.is_some();
        if moved || Some(m.id.inner()) != old_milestone.as_deref() {
            input.project_milestone_id = Patch::Set(m.id.inner().to_owned());
            restore.project_milestone_id = put_back(old_milestone);
            changed.push("milestone");
        }
    }
    if let Some(d) = cmd.due.filter(|d| Some(*d) != issue.due_date) {
        input.due_date = Patch::Set(d);
        restore.due_date = put_back(issue.due_date);
        changed.push("dueDate");
    }
    if let Some(a) =
        assignee.filter(|a| Some(a.as_str()) != issue.assignee.as_ref().map(|u| u.id.inner()))
    {
        input.assignee_id = Patch::Set(a);
        restore.assignee_id = put_back(issue.assignee.as_ref().map(|u| u.id.inner().to_owned()));
        changed.push("assignee");
    }
    if let Some(wanted) = &labels {
        let ids = |labels: &[Label]| -> BTreeSet<String> {
            labels.iter().map(|l| l.id.inner().to_owned()).collect()
        };
        if ids(wanted) != ids(&issue.labels) {
            input.label_ids = Some(wanted.iter().map(|l| l.id.inner().to_owned()).collect());
            restore.label_ids = Some(
                issue
                    .labels
                    .iter()
                    .map(|l| l.id.inner().to_owned())
                    .collect(),
            );
            changed.push("labels");
        }
    }
    let attachment = match source {
        Some(url) => source_step(
            issue,
            url,
            cmd.source_title.as_deref(),
            &ws.source_title,
            metadata.as_ref(),
        ),
        None => None,
    };
    if attachment.is_some() {
        changed.push("source");
    }

    // Mutate. A source that cannot be attached puts the other fields back.
    let mut rollback = Rollback::new();
    let mut updated = None;
    if !input.is_empty() {
        let data: IssueUpdate = ws
            .client
            .execute(&inputs::issue_update(issue.id.inner(), input))?;
        updated = Some(changed_issue(data.issue_update, &issue.identifier)?);
        let client = &ws.client;
        let id = issue.id.inner();
        rollback.on_failure(
            format!("restored {}'s earlier values", issue.identifier),
            move || {
                let r: IssueUpdate = client.execute(&inputs::issue_update(id, restore))?;
                if r.issue_update.success {
                    Ok(())
                } else {
                    Err(CliError::general("Linear refused to restore them"))
                }
            },
        );
    }
    if let Some(input) = attachment {
        let attached = retry(ws.out, "attaching the source", &ATTACH_WAITS, || {
            let r: AttachmentCreate = ws
                .client
                .execute(&inputs::attachment_create(input.clone()))?;
            if r.attachment_create.success {
                Ok(())
            } else {
                Err(CliError::general("Linear could not attach the source"))
            }
        });
        if let Err(cause) = attached {
            return Err(rollback.fail(cause));
        }
        // The payload of the update predates the attachment.
        updated = None;
    }

    // Show the issue as it is now.
    let now = match updated {
        Some(issue) => issue,
        None if changed.is_empty() => view.issue.clone(),
        None => fetch_issue(&ws, issue.id.inner())?.issue,
    };
    show_issue(ctx, &ws, &now, &changed);
    Ok(())
}

/// The labels an issue ends up with: `--labels` as given, or the ones it has
/// now with `--add-labels` put in and `--remove-labels` taken out.
fn labels_after(ws: &WriteSession, issue: &Issue, cmd: &UpdateCmd) -> Result<Vec<Label>> {
    if !cmd.labels.is_empty() {
        return resolve::labels(ws, &cmd.labels);
    }
    let add = resolve::labels(ws, &cmd.add_labels)?;
    let remove = resolve::labels(ws, &cmd.remove_labels)?;
    let mut now: Vec<Label> = issue
        .labels
        .iter()
        .filter(|l| !remove.iter().any(|r| r.id == l.id))
        .cloned()
        .collect();
    for label in add {
        if !now.iter().any(|l| l.id == label.id) {
            now.push(label);
        }
    }
    Ok(now)
}

/// The attachment request that makes the issue carry `url` as asked, or `None`
/// when it already does. Linear upserts on the URL and replaces what the
/// attachment stores, so an attachment the issue has already is sent with its
/// stored title, subtitle and metadata wherever the caller did not give one.
/// A new attachment is titled `title`, else the workspace's `default_title`.
fn source_step(
    issue: &Issue,
    url: &str,
    title: Option<&str>,
    default_title: &str,
    metadata: Option<&AttachmentMetadata>,
) -> Option<AttachmentCreateInput> {
    let Some(stored) = issue.attachments.iter().find(|a| a.url == url) else {
        return Some(AttachmentCreateInput {
            issue_id: issue.id.inner().to_owned(),
            url: url.to_owned(),
            title: title.unwrap_or(default_title).to_owned(),
            subtitle: None,
            metadata: metadata.cloned(),
        });
    };
    let new_title = title.filter(|t| *t != stored.title);
    if new_title.is_none() && !metadata_needs_update(metadata, &stored.metadata) {
        return None;
    }
    Some(AttachmentCreateInput {
        issue_id: issue.id.inner().to_owned(),
        url: url.to_owned(),
        title: new_title.map_or_else(|| stored.title.clone(), str::to_owned),
        subtitle: stored.subtitle.clone(),
        // Replaced as a whole, so what is not given is sent as it is stored
        // (when it is flat enough to be sent back).
        metadata: metadata
            .cloned()
            .or_else(|| AttachmentMetadata::from_json(&stored.metadata).ok()),
    })
}

fn show_issue(ctx: &Ctx, ws: &WriteSession, issue: &Issue, changed: &[&'static str]) {
    let value = Updated {
        issue: issue_out(&ws.workspace, issue),
        changed: changed.to_vec(),
    };
    ctx.out.emit(
        &value,
        || {
            let verb = if changed.is_empty() {
                "already as asked, nothing changed".to_owned()
            } else {
                format!("updated {}", changed.join(", "))
            };
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
    #[arg(required = true, num_args = 1.., value_delimiter = ',', value_name = "ISSUE")]
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
    // Checked here, not by clap, so that `A,B` (one comma-separated value) counts as two.
    let references: Vec<&str> = cmd
        .issues
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if references.len() < 2 {
        return Err(CliError::usage("reordering needs at least two issues"));
    }
    let ws = ctx.write_session()?;

    let views = references
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
