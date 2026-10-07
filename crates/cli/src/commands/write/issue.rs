//! `linear issue create|update|comment|link-pr|unlink|relate|unrelate|delete|archive|unarchive|reorder`.
//!
//! `update` is the write that can touch the most: fields of the issue, its
//! description, its labels and its source attachment. It sends only what
//! differs from now, and puts the issue's fields back if the attachment, the
//! last step, cannot be made.

use super::dry_run::{Plan, Target, NEW_ISSUE_ID};
use super::{read_text, resolve, retry, ForceArg, Rollback, WriteSession, ATTACH_WAITS};
use crate::commands::cycle;
use crate::commands::format::person;
use crate::commands::issue::{out as issue_out, IssueOut};
use crate::commands::listing::resolve_issue;
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use chrono::NaiveDate;
use clap::Args;
use linear_core::config::Rule;
use linear_core::cycle::parse_number as cycle_number;
use linear_core::filters::priority_number;
use linear_core::guard::{Placement, Write};
use linear_core::inputs::{
    self, AttachmentCreate, AttachmentCreateInput, CommentCreate, CommentCreateInput, IssueCreate,
    IssueCreateInput, IssueDelete, IssueRelationCreate, IssueRelationCreateInput,
    IssueRelationDelete, IssueRelationType, IssueUpdate, IssueUpdateInput, Patch,
};
use linear_core::markdown::same_description;
use linear_core::matching::match_state;
use linear_core::metadata::AttachmentMetadata;
use linear_core::pull_request::{parse_pull_request_url, PullRequest};
use linear_core::queries;
use linear_core::read::{self, IssueRelationsQuery, IssueWriteView};
use linear_core::relation;
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
    /// Priority: 0-4 or none, urgent, high, medium, low
    #[arg(long, value_name = "LEVEL", value_parser = priority_number)]
    pub priority: Option<i32>,
    /// Estimate, a whole number in the team's scale
    #[arg(long, value_name = "N", value_parser = parse_estimate)]
    pub estimate: Option<i32>,
    /// Make it a sub-issue of this issue (identifier such as KK-12, or id)
    #[arg(long, value_name = "ISSUE")]
    pub parent: Option<String>,
    /// Put it in the cycle with this number (`42` or `#42`) of the team; the command fails
    /// before creating anything when the team has none. As with --held-on, an issue that
    /// already exists for the same --source gets the cycle only when it has none
    #[arg(long, value_name = "N", value_parser = cycle_number, conflicts_with = "held_on")]
    pub cycle: Option<u32>,
    /// Team key (default: the workspace's `default_team`)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Allow creating, in a project somebody else leads, an issue assigned to you
    #[arg(long)]
    pub allow_foreign: bool,
    #[command(flatten)]
    pub force: ForceArg,
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

    let ws = ctx.write_session_with(cmd.force)?;

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
    // A day no cycle contains, and a number no cycle has, stop here, before anything is written.
    let cycle = match (cmd.held_on, cmd.cycle) {
        (Some(day), _) => Some(cycle::find(&ws.client, &team.key, day)?),
        (None, Some(number)) => Some(cycle::find_with_number(&ws.client, &team.key, number)?),
        (None, None) => None,
    };
    let parent = cmd
        .parent
        .as_deref()
        .map(|reference| resolve_issue(&ws.client, reference))
        .transpose()?;

    // Guard, then validators.
    ws.guard(
        &format!("new issue in project {:?}", project.name),
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
        let skipped: Vec<&str> = [
            ("--priority", cmd.priority.is_some()),
            ("--estimate", cmd.estimate.is_some()),
            ("--parent", cmd.parent.is_some()),
        ]
        .into_iter()
        .filter_map(|(flag, given)| given.then_some(flag))
        .collect();
        if !skipped.is_empty() {
            ws.note(&format!(
                "{} already exists for this source, so {} not applied (`issue update` changes it)",
                existing.identifier,
                skipped.join(", ") + if skipped.len() == 1 { " was" } else { " were" }
            ));
        }
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
        if ws.dry_run {
            let mut written = changed.clone();
            if metadata_updated {
                written.push("sourceMetadata");
            }
            return ws.finish_dry_run(
                Plan::new(
                    "issue create",
                    Target::existing("issue", &existing.identifier, existing.id.inner()),
                )
                .changed(written)
                .reason("an issue with this source already exists, so none is created"),
            );
        }
        emit_created(
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
        priority: cmd.priority,
        estimate: cmd.estimate,
        parent_id: parent.as_ref().map(|p| p.id.inner().to_owned()),
    };
    if ws.dry_run {
        ws.record(&inputs::issue_create(input));
        if let Some(source) = source {
            ws.record(&inputs::attachment_create(source_input(
                &ws,
                cmd,
                NEW_ISSUE_ID,
                source,
                &metadata,
            )));
            ws.record_undo(&inputs::issue_delete(NEW_ISSUE_ID));
        }
        return ws.finish_dry_run(Plan::new("issue create", Target::new("issue", &cmd.title)));
    }
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
            let input = source_input(&ws, cmd, issue.id.inner(), source, &metadata);
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

/// The attachment that makes `issue_id` carry the source of `issue create`.
fn source_input(
    ws: &WriteSession,
    cmd: &CreateCmd,
    issue_id: &str,
    source: &str,
    metadata: &Option<AttachmentMetadata>,
) -> AttachmentCreateInput {
    AttachmentCreateInput {
        issue_id: issue_id.to_owned(),
        url: source.to_owned(),
        title: cmd
            .source_title
            .clone()
            .unwrap_or_else(|| ws.source_title.clone()),
        subtitle: None,
        metadata: metadata.clone(),
    }
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
        &format!("issue {}", view.issue.identifier),
        &Write::IssueUpdate {
            assignee: view.issue.assignee.as_ref().map(|u| u.id.inner()),
            placement: placement_of(&view),
            moves_to: None,
        },
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;
    let input = IssueUpdateInput {
        cycle_id: Patch::Set(wanted.id.inner().to_owned()),
        ..Default::default()
    };
    let op = inputs::issue_update(view.issue.id.inner(), input);
    if ws.dry_run {
        ws.record(&op);
        return Ok((Some(wanted.clone()), vec!["cycle"]));
    }
    let data: IssueUpdate = ws.client.execute(&op)?;
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
    let op = inputs::attachment_create(input);
    if ws.dry_run {
        ws.record(&op);
        return Ok(true);
    }
    let r: AttachmentCreate = ws.client.execute(&op)?;
    if r.attachment_create.success {
        Ok(true)
    } else {
        Err(CliError::general(
            "Linear could not update the metadata of the source attachment",
        ))
    }
}

fn emit_created(ws: &WriteSession, made: Made<'_>, source: Option<&str>) {
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
    ws.emit(
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
    /// Priority: 0-4 or none, urgent, high, medium, low
    #[arg(long, value_name = "LEVEL", value_parser = priority_number)]
    pub priority: Option<i32>,
    /// Estimate, a whole number in the team's scale, or `none` to remove it
    #[arg(long, value_name = "N", value_parser = estimate_or_none)]
    pub estimate: Option<Patch<i32>>,
    /// Make it a sub-issue of this issue (identifier such as KK-12, or id), or `none` to make
    /// it a top-level issue again
    #[arg(long, value_name = "ISSUE")]
    pub parent: Option<String>,
    /// Put it in the cycle with this number (`42` or `#42`) of the issue's team, or `none` to
    /// take it out of its cycle
    #[arg(long, value_name = "N", value_parser = cycle_or_none)]
    pub cycle: Option<Patch<u32>>,
    #[command(flatten)]
    pub force: ForceArg,
}

/// `--estimate`: a whole number, at least 0.
fn parse_estimate(spec: &str) -> std::result::Result<i32, String> {
    spec.trim()
        .parse::<i32>()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| format!("{spec:?} is not an estimate; expected a whole number such as 3"))
}

fn is_none(spec: &str) -> bool {
    spec.trim().eq_ignore_ascii_case("none")
}

fn estimate_or_none(spec: &str) -> std::result::Result<Patch<i32>, String> {
    if is_none(spec) {
        return Ok(Patch::Clear);
    }
    parse_estimate(spec).map(Patch::Set)
}

fn cycle_or_none(spec: &str) -> std::result::Result<Patch<u32>, String> {
    if is_none(spec) {
        return Ok(Patch::Clear);
    }
    cycle_number(spec).map(Patch::Set)
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
        && cmd.priority.is_none()
        && cmd.estimate.is_none()
        && cmd.parent.is_none()
        && cmd.cycle.is_none()
    {
        return Err(CliError::usage(
            "nothing to change: pass --state, --project, --milestone, --due, --assignee, \
             --body-file, --source, --labels, --add-labels, --remove-labels, --priority, \
             --estimate, --parent or --cycle",
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

    let ws = ctx.write_session_with(cmd.force)?;
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
    // `Patch::Keep`: not asked for; `Clear`: asked to be taken away; `Set`: the issue or cycle.
    let parent = match cmd.parent.as_deref().map(str::trim) {
        None => Patch::Keep,
        Some(spec) if is_none(spec) => Patch::Clear,
        Some(reference) => {
            let found = resolve_issue(&ws.client, reference)?;
            if found.id == issue.id {
                return Err(CliError::usage(format!(
                    "{} cannot be its own parent",
                    issue.identifier
                )));
            }
            Patch::Set(found)
        }
    };
    let cycle = match &cmd.cycle {
        None => Patch::Keep,
        Some(Patch::Keep) => Patch::Keep,
        Some(Patch::Clear) => Patch::Clear,
        Some(Patch::Set(number)) => Patch::Set(cycle::find_with_number(
            &ws.client,
            &issue.team.key,
            *number,
        )?),
    };

    // Guard, then validators.
    ws.guard(
        &format!("issue {}", issue.identifier),
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
            &format!("issue {}", issue.identifier),
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
    if let Some(p) = cmd.priority {
        let old = issue.priority as i32;
        if p != old {
            input.priority = Some(p);
            restore.priority = Some(old);
            changed.push("priority");
        }
    }
    let old_estimate = issue.estimate.map(|e| e as i32);
    match cmd.estimate {
        Some(Patch::Set(e)) if old_estimate != Some(e) => {
            input.estimate = Patch::Set(e);
            restore.estimate = put_back(old_estimate);
            changed.push("estimate");
        }
        Some(Patch::Clear) if old_estimate.is_some() => {
            input.estimate = Patch::Clear;
            restore.estimate = put_back(old_estimate);
            changed.push("estimate");
        }
        _ => {}
    }
    let old_parent = issue.parent.as_ref().map(|p| p.id.inner().to_owned());
    match &parent {
        Patch::Set(p) if old_parent.as_deref() != Some(p.id.inner()) => {
            input.parent_id = Patch::Set(p.id.inner().to_owned());
            restore.parent_id = put_back(old_parent);
            changed.push("parent");
        }
        Patch::Clear if old_parent.is_some() => {
            input.parent_id = Patch::Clear;
            restore.parent_id = put_back(old_parent);
            changed.push("parent");
        }
        _ => {}
    }
    let old_cycle = view.write.cycle.as_ref().map(|c| c.id.inner().to_owned());
    match &cycle {
        Patch::Set(c) if old_cycle.as_deref() != Some(c.id.inner()) => {
            input.cycle_id = Patch::Set(c.id.inner().to_owned());
            restore.cycle_id = put_back(old_cycle);
            changed.push("cycle");
        }
        Patch::Clear if old_cycle.is_some() => {
            input.cycle_id = Patch::Clear;
            restore.cycle_id = put_back(old_cycle);
            changed.push("cycle");
        }
        _ => {}
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

    if ws.dry_run {
        // The same two steps, recorded: the fields, then the source. The fields are put back
        // only when the source (the later step) fails.
        if !input.is_empty() {
            ws.record(&inputs::issue_update(issue.id.inner(), input));
            if attachment.is_some() {
                ws.record_undo(&inputs::issue_update(issue.id.inner(), restore));
            }
        }
        if let Some(input) = attachment {
            ws.record(&inputs::attachment_create(input));
        }
        return ws.finish_dry_run(
            Plan::new(
                "issue update",
                Target::existing("issue", &issue.identifier, issue.id.inner()),
            )
            .changed(changed)
            .reason("every field is already as asked"),
        );
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
    show_issue(&ws, &now, &changed);
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

fn show_issue(ws: &WriteSession, issue: &Issue, changed: &[&'static str]) {
    let value = Updated {
        issue: issue_out(&ws.workspace, issue),
        changed: changed.to_vec(),
    };
    ws.emit(
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
    #[command(flatten)]
    pub force: ForceArg,
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

    let ws = ctx.write_session_with(cmd.force)?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    // A comment is a write to the issue: it follows the same ownership as changing it.
    ws.guard(
        &format!("issue {}", view.issue.identifier),
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
    let op = inputs::comment_create(input);
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new("issue comment", issue_target(&view)));
    }
    let data: CommentCreate = ws.client.execute(&op)?;
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
    ws.emit(
        &value,
        || format!("{}  {}", view.issue.identifier, comment.url),
        || comment.url.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- link-pr

#[derive(Debug, Args)]
pub struct LinkPrCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    /// The pull request's URL: https://github.com/<owner>/<repo>/pull/<number>
    pub url: String,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `link-pr` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Linked<'a> {
    workspace: &'a str,
    issue: &'a str,
    url: &'a str,
    /// `true` when the issue already carried this pull request, so nothing was sent.
    already_linked: bool,
    /// The attachment Linear holds (absent when nothing was sent).
    attachment: Option<&'a linear_core::types::Attachment>,
    /// What the integration reports about the pull request, if the attachment is one.
    pull_request: Option<PullRequest>,
}

pub fn link_pr(ctx: &Ctx, cmd: &LinkPrCmd) -> Result<()> {
    let Some((url, number)) = parse_pull_request_url(&cmd.url) else {
        return Err(CliError::usage(format!(
            "{:?} is not a GitHub pull request URL (expected https://<host>/<owner>/<repo>/pull/<number>)",
            cmd.url
        )));
    };

    let ws = ctx.write_session_with(cmd.force)?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    // Linking is a write to the issue: it follows the same ownership as changing it.
    ws.guard(
        &format!("issue {}", view.issue.identifier),
        &Write::update_issue(&view.issue, placement_of(&view)),
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;
    let identifier = view.issue.identifier.as_str();

    // Linking twice would only make Linear answer with the attachment it has.
    let linked = view
        .issue
        .attachments
        .iter()
        .find(|a| a.pull_request().is_some_and(|p| p.url == url));
    if let Some(attachment) = linked {
        if ws.dry_run {
            return ws.finish_dry_run(
                Plan::new("issue link-pr", issue_target(&view))
                    .reason("the issue already carries this pull request"),
            );
        }
        return emit_linked(&ws, identifier, &url, number, true, attachment);
    }

    // Linear refuses a link the workspace has no GitHub integration for with an error that
    // says little. Ask first. If that cannot be asked (a key that may not read integrations),
    // the mutation decides.
    let integrations: Result<queries::Integrations> = ws.client.execute(&queries::integrations());
    if let Ok(found) = integrations {
        if !found.has_github() {
            return Err(CliError::general(format!(
                "workspace {:?} has no GitHub integration, so a pull request cannot be linked \
                 (install it in Linear: Settings > Integrations > GitHub)",
                ws.workspace
            )));
        }
    }

    let op = inputs::attachment_link_github_pr(view.issue.id.inner(), &url);
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new("issue link-pr", issue_target(&view)));
    }
    let data: inputs::AttachmentLinkGitHubPr = ws.client.execute(&op)?;
    let payload = data.attachment_link_git_hub_pr;
    if !payload.success {
        return Err(CliError::general(format!(
            "Linear could not link {url} to {identifier}"
        )));
    }
    if payload.attachment.pull_request().is_none() {
        eprintln!(
            "warning: Linear made the attachment but it is not a GitHub pull request one \
             (sourceType {:?}), so its state will not follow GitHub",
            payload.attachment.source_type
        );
    }
    emit_linked(&ws, identifier, &url, number, false, &payload.attachment)
}

fn emit_linked(
    ws: &WriteSession,
    identifier: &str,
    url: &str,
    number: u64,
    already_linked: bool,
    attachment: &linear_core::types::Attachment,
) -> Result<()> {
    let pull_request = attachment.pull_request();
    let value = Linked {
        workspace: &ws.workspace,
        issue: identifier,
        url,
        already_linked,
        attachment: Some(attachment),
        pull_request: pull_request.clone(),
    };
    ws.emit(
        &value,
        || {
            let state = pull_request
                .as_ref()
                .map(|p| format!(", {}", p.status.as_str()))
                .unwrap_or_default();
            let how = if already_linked {
                "already linked, nothing sent"
            } else {
                "linked"
            };
            format!("{identifier}  #{number}  {url}  ({how}{state})")
        },
        || url.to_owned(),
    );
    Ok(())
}

// ---------------------------------------------------------------- unlink

#[derive(Debug, Args)]
pub struct UnlinkCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    /// The exact URL of the attachment to delete from the issue
    pub url: String,
    /// Delete it. Required: Linear documents no way to bring an attachment back
    /// (without it, the command prints what it would delete and sends nothing)
    #[arg(long)]
    pub yes: bool,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `unlink` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Unlinked<'a> {
    workspace: &'a str,
    issue: &'a str,
    url: &'a str,
    /// `true` when the issue had no attachment with this URL, so nothing was sent.
    not_linked: bool,
    /// The id of the attachment that was deleted (absent when nothing was sent).
    attachment_id: Option<&'a str>,
}

pub fn unlink(ctx: &Ctx, cmd: &UnlinkCmd) -> Result<()> {
    let url = cmd.url.trim();
    if url.is_empty() {
        return Err(CliError::usage("the URL is empty"));
    }

    let ws = ctx.write_session_with(cmd.force)?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    // Deleting an attachment is a write to the issue: it follows the same ownership as changing it.
    ws.guard(
        &format!("issue {}", view.issue.identifier),
        &Write::update_issue(&view.issue, placement_of(&view)),
        false,
    )?;
    let identifier = view.issue.identifier.as_str();

    // `attachmentsForURL` finds the attachments of every issue with this URL; keep this issue's.
    let found: read::AttachmentTargetsQuery = ws.client.execute(&read::attachment_targets(url))?;
    let target = found
        .attachments_for_url
        .iter()
        .find(|a| a.url == url && a.issue.id == view.issue.id);
    let Some(target) = target else {
        if ws.dry_run {
            return ws.finish_dry_run(
                Plan::new("issue unlink", issue_target(&view))
                    .reason("the issue has no attachment with this URL"),
            );
        }
        let value = Unlinked {
            workspace: &ws.workspace,
            issue: identifier,
            url,
            not_linked: true,
            attachment_id: None,
        };
        ws.emit(
            &value,
            || format!("{identifier}  {url}  (not linked, nothing sent)"),
            || url.to_owned(),
        );
        return Ok(());
    };

    if !cmd.yes {
        return Err(CliError::usage(format!(
            "would delete the attachment {:?} ({url}) from {identifier}; Linear documents no way \
             to bring an attachment back, so nothing was sent. Run again with --yes to delete it",
            target.title
        )));
    }
    let op = inputs::attachment_delete(target.id.inner());
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new("issue unlink", issue_target(&view)));
    }
    let data: inputs::AttachmentDelete = ws.client.execute(&op)?;
    if !data.attachment_delete.success {
        return Err(CliError::general(format!(
            "Linear could not delete the attachment {url} from {identifier}"
        )));
    }
    let value = Unlinked {
        workspace: &ws.workspace,
        issue: identifier,
        url,
        not_linked: false,
        attachment_id: Some(target.id.inner()),
    };
    ws.emit(
        &value,
        || format!("{identifier}  {url}  (unlinked)"),
        || url.to_owned(),
    );
    Ok(())
}

// ---------------------------------------------------------------- delete, archive, unarchive

#[derive(Debug, Args)]
pub struct IssueTargetCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `delete`, `archive` and `unarchive` print.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Archived<'a> {
    workspace: &'a str,
    id: &'a str,
    identifier: &'a str,
    url: &'a str,
    /// What was done: `deleted` (trashed), `archived` or `unarchived`.
    action: &'static str,
}

/// Trash an issue. Linear keeps it for a while and `unarchive` brings it back.
pub fn delete(ctx: &Ctx, cmd: &IssueTargetCmd) -> Result<()> {
    change_archive(ctx, cmd, "issue delete", "deleted", |ws, id| {
        ws.send_checked(&inputs::issue_delete(id), |r: IssueDelete| {
            r.issue_delete.success
        })
    })
}

pub fn archive(ctx: &Ctx, cmd: &IssueTargetCmd) -> Result<()> {
    change_archive(ctx, cmd, "issue archive", "archived", |ws, id| {
        ws.send_checked(&inputs::issue_archive(id), |r: inputs::IssueArchive| {
            r.issue_archive.success
        })
    })
}

pub fn unarchive(ctx: &Ctx, cmd: &IssueTargetCmd) -> Result<()> {
    change_archive(ctx, cmd, "issue unarchive", "unarchived", |ws, id| {
        ws.send_checked(&inputs::issue_unarchive(id), |r: inputs::IssueUnarchive| {
            r.issue_unarchive.success
        })
    })
}

/// The shared path of the three: read the issue, ask the ownership rules (the
/// same as for changing it), send `mutate`, print one line.
fn change_archive(
    ctx: &Ctx,
    cmd: &IssueTargetCmd,
    command: &'static str,
    action: &'static str,
    mutate: impl FnOnce(&WriteSession, &str) -> Result<bool>,
) -> Result<()> {
    let ws = ctx.write_session_with(cmd.force)?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    ws.guard(
        &format!("issue {}", view.issue.identifier),
        &Write::update_issue(&view.issue, placement_of(&view)),
        false,
    )?;
    let issue = &view.issue;
    if !mutate(&ws, issue.id.inner())? {
        return Err(CliError::general(format!(
            "Linear could not change {} ({action})",
            issue.identifier
        )));
    }
    if ws.dry_run {
        return ws.finish_dry_run(Plan::new(command, issue_target(&view)));
    }
    let value = Archived {
        workspace: &ws.workspace,
        id: issue.id.inner(),
        identifier: &issue.identifier,
        url: &issue.url,
        action,
    };
    ws.emit(
        &value,
        || format!("{}  {}  ({action})", issue.identifier, issue.title),
        || issue.identifier.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- relate, unrelate

/// The kind of relation, given as the flag that names the other issue. Exactly one.
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct RelationFlag {
    /// ISSUE blocks OTHER
    #[arg(long, value_name = "OTHER")]
    pub blocks: Option<String>,
    /// ISSUE is related to OTHER (no direction)
    #[arg(long, value_name = "OTHER")]
    pub related: Option<String>,
    /// ISSUE is a duplicate of OTHER (the one that stays)
    #[arg(long, value_name = "OTHER")]
    pub duplicate: Option<String>,
}

impl RelationFlag {
    /// The kind and the other issue, as typed.
    fn parts(&self) -> (IssueRelationType, &str) {
        match (&self.blocks, &self.related, &self.duplicate) {
            (Some(o), _, _) => (IssueRelationType::Blocks, o),
            (_, Some(o), _) => (IssueRelationType::Related, o),
            (_, _, Some(o)) => (IssueRelationType::Duplicate, o),
            // clap's group demands one.
            _ => unreachable!("a relation flag is required"),
        }
    }
}

#[derive(Debug, Args)]
pub struct RelateCmd {
    /// Issue identifier (such as KK-12) or id: the issue the relation is written on
    pub issue: String,
    #[command(flatten)]
    pub relation: RelationFlag,
    #[command(flatten)]
    pub force: ForceArg,
}

#[derive(Debug, Args)]
pub struct UnrelateCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    #[command(flatten)]
    pub relation: RelationFlag,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `relate` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Related<'a> {
    workspace: &'a str,
    /// The relation's id.
    id: &'a str,
    issue: &'a str,
    /// `blocks`, `related` or `duplicate`.
    #[serde(rename = "type")]
    type_: &'static str,
    related_issue: &'a str,
    /// `true` when the relation already existed, so nothing was sent.
    already_related: bool,
}

/// What `unrelate` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Unrelated<'a> {
    workspace: &'a str,
    issue: &'a str,
    #[serde(rename = "type")]
    type_: &'static str,
    related_issue: &'a str,
    /// The ids of the relations that were removed; empty when there was none.
    removed: Vec<&'a str>,
}

/// "KK-1 blocks KK-2", "KK-1 is a duplicate of KK-2", "KK-1 is related to KK-2".
fn sentence(issue: &str, kind: IssueRelationType, other: &str) -> String {
    match kind {
        IssueRelationType::Blocks => format!("{issue} blocks {other}"),
        IssueRelationType::Duplicate => format!("{issue} is a duplicate of {other}"),
        IssueRelationType::Related | IssueRelationType::Similar => {
            format!("{issue} is related to {other}")
        }
    }
}

/// What `relate` and `unrelate` both settle before they send anything: the issue the
/// relation is written on (ownership already checked), the other one, and the relations
/// between them of the asked kind.
struct Pair {
    issue: IssueRelationsQuery,
    other: IssueRelationsQuery,
}

/// `closes` is for a write that makes Linear close the issue (a new duplicate relation).
fn pair(ws: &WriteSession, issue: &str, closes: bool, other: &str) -> Result<Pair> {
    let view = fetch_issue(ws, issue)?;
    // A relation is a write to the issue it starts from: the same ownership as changing it.
    ws.guard(
        &format!("issue {}", view.issue.identifier),
        &Write::update_issue(&view.issue, placement_of(&view)),
        false,
    )?;
    // Linear moves a duplicate to its Duplicate state: ask what canceling asks, which a lenient
    // workspace does not relax. (Removing the relation puts the issue back, which is not a cancel.)
    if closes {
        ws.guard(
            &format!("issue {}", view.issue.identifier),
            &Write::IssueCancel {
                assignee: view.issue.assignee.as_ref().map(|u| u.id.inner()),
                placement: placement_of(&view),
            },
            false,
        )?;
    }
    ws.validate(&Draft::new(Operation::IssueUpdate))?;

    let issue: IssueRelationsQuery = ws
        .client
        .execute(&read::issue_relations(view.issue.id.inner()))?;
    let other: IssueRelationsQuery = ws.client.execute(&read::issue_relations(other))?;
    if issue.issue.id.inner() == other.issue.id.inner() {
        return Err(CliError::usage("an issue cannot be related to itself"));
    }
    Ok(Pair { issue, other })
}

/// Both references named the same issue, judged from the text alone (nothing sent yet).
fn same_reference(issue: &str, other: &str) -> bool {
    issue.trim().eq_ignore_ascii_case(other.trim())
}

fn check_references(issue: &str, other: &str) -> Result<()> {
    if other.trim().is_empty() {
        return Err(CliError::usage("the other issue is empty"));
    }
    if same_reference(issue, other) {
        return Err(CliError::usage("an issue cannot be related to itself"));
    }
    Ok(())
}

pub fn relate(ctx: &Ctx, cmd: &RelateCmd) -> Result<()> {
    let (kind, other) = cmd.relation.parts();
    check_references(&cmd.issue, other)?;

    let ws = ctx.write_session_with(cmd.force)?;
    let Pair { issue, other } = pair(&ws, &cmd.issue, kind == IssueRelationType::Duplicate, other)?;
    let (a, b) = (
        issue.issue.identifier.as_str(),
        other.issue.identifier.as_str(),
    );

    let existing = relation::matching(&issue.issue, other.issue.id.inner(), kind);
    let (id, already) = match existing.first() {
        Some(found) => {
            if ws.dry_run {
                return ws.finish_dry_run(
                    Plan::new(
                        "issue relate",
                        Target::existing("issue", a, issue.issue.id.inner()),
                    )
                    .reason("the relation is already there"),
                );
            }
            (found.id.inner().to_owned(), true)
        }
        None => {
            let input = IssueRelationCreateInput {
                issue_id: issue.issue.id.inner().to_owned(),
                related_issue_id: other.issue.id.inner().to_owned(),
                type_: kind,
            };
            let op = inputs::issue_relation_create(input);
            if ws.dry_run {
                ws.record(&op);
                return ws.finish_dry_run(Plan::new(
                    "issue relate",
                    Target::existing("issue", a, issue.issue.id.inner()),
                ));
            }
            let data: IssueRelationCreate = ws.client.execute(&op)?;
            if !data.issue_relation_create.success {
                return Err(CliError::general(format!(
                    "Linear could not relate {a} to {b}"
                )));
            }
            (
                data.issue_relation_create
                    .issue_relation
                    .id
                    .inner()
                    .to_owned(),
                false,
            )
        }
    };

    let value = Related {
        workspace: &ws.workspace,
        id: &id,
        issue: a,
        type_: kind.as_str(),
        related_issue: b,
        already_related: already,
    };
    ws.emit(
        &value,
        || {
            let note = if already {
                "  (already there, nothing sent)"
            } else {
                ""
            };
            format!("{}{note}", sentence(a, kind, b))
        },
        || id.clone(),
    );
    Ok(())
}

pub fn unrelate(ctx: &Ctx, cmd: &UnrelateCmd) -> Result<()> {
    let (kind, other) = cmd.relation.parts();
    check_references(&cmd.issue, other)?;

    let ws = ctx.write_session_with(cmd.force)?;
    let Pair { issue, other } = pair(&ws, &cmd.issue, false, other)?;
    let (a, b) = (
        issue.issue.identifier.as_str(),
        other.issue.identifier.as_str(),
    );

    let found = relation::matching(&issue.issue, other.issue.id.inner(), kind);
    for relation in &found {
        let sent = ws.send_checked(
            &inputs::issue_relation_delete(relation.id.inner()),
            |r: IssueRelationDelete| r.issue_relation_delete.success,
        )?;
        if !sent {
            return Err(CliError::general(format!(
                "Linear could not remove the relation of {a} and {b}"
            )));
        }
    }

    if ws.dry_run {
        return ws.finish_dry_run(
            Plan::new(
                "issue unrelate",
                Target::existing("issue", a, issue.issue.id.inner()),
            )
            .reason("there is no such relation"),
        );
    }
    let removed: Vec<&str> = found.iter().map(|r| r.id.inner()).collect();
    let value = Unrelated {
        workspace: &ws.workspace,
        issue: a,
        type_: kind.as_str(),
        related_issue: b,
        removed: removed.clone(),
    };
    ws.emit(
        &value,
        || {
            let note = if removed.is_empty() {
                "no such relation, nothing sent"
            } else {
                "removed"
            };
            format!("{}  ({note})", sentence(a, kind, b))
        },
        || removed.join("\n"),
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
    #[command(flatten)]
    pub force: ForceArg,
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
    let ws = ctx.write_session_with(cmd.force)?;

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
            &format!("issue {}", view.issue.identifier),
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
        if ws.dry_run {
            let issue = &view.issue;
            ws.record(&inputs::issue_update(
                issue.id.inner(),
                IssueUpdateInput {
                    sort_order: change.sort_order,
                    priority_sort_order: change.priority_sort_order,
                    ..Default::default()
                },
            ));
            ws.record_undo(&inputs::issue_update(
                issue.id.inner(),
                IssueUpdateInput {
                    sort_order: change.sort_order.map(|_| issue.sort_order),
                    priority_sort_order: change
                        .priority_sort_order
                        .map(|_| issue.priority_sort_order),
                    ..Default::default()
                },
            ));
            continue;
        }
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

    if ws.dry_run {
        return ws.finish_dry_run(
            Plan::new("issue reorder", Target::several("issue", &wanted))
                .reason("the issues already sit in that order"),
        );
    }
    let value = Reordered {
        workspace: &ws.workspace,
        updated: plan.iter().map(|c| c.identifier.as_str()).collect(),
        unchanged: plan.is_empty(),
    };
    ws.emit(
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
pub(super) fn fetch_issue(ws: &WriteSession, reference: &str) -> Result<IssueWriteView> {
    ws.client.execute(&read::issue_write_view(reference.trim()))
}

/// The issue a write is aimed at, as a dry run names it.
pub(super) fn issue_target(view: &IssueWriteView) -> Target {
    Target::existing("issue", &view.issue.identifier, view.issue.id.inner())
}

pub(super) fn placement_of(view: &IssueWriteView) -> Placement<'_> {
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
