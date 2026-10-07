//! `linear comment update|delete`.
//!
//! A comment is its author's: the ownership rule looks at who wrote it, not at who owns
//! the issue it is on ([`Write::CommentUpdate`], [`Write::CommentDelete`]). Neither write
//! changes the issue itself, so no validator applies.

use super::dry_run::{Plan, Target};
use super::{read_text, ForceArg, WriteSession};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::Args;
use linear_core::guard::Write;
use linear_core::inputs::{self, CommentDelete, CommentUpdate, CommentUpdateInput};
use linear_core::read::{self, CommentQuery};
use linear_core::types::Comment;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct UpdateCmd {
    /// Comment id
    pub comment: String,
    /// Read the new text from a file (`-` for standard input)
    #[arg(long, value_name = "FILE")]
    pub body_file: PathBuf,
    #[command(flatten)]
    pub force: ForceArg,
}

#[derive(Debug, Args)]
pub struct DeleteCmd {
    /// Comment id
    pub comment: String,
    /// Confirm the deletion (a deleted comment cannot be restored)
    #[arg(long)]
    pub yes: bool,
    #[command(flatten)]
    pub force: ForceArg,
}

/// What `update` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Updated<'a> {
    workspace: &'a str,
    id: &'a str,
    url: &'a str,
    /// The issue the comment is on.
    issue: &'a str,
    /// `false` when the text was already this, so nothing was sent.
    changed: bool,
}

/// What `delete` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Deleted<'a> {
    workspace: &'a str,
    id: &'a str,
    issue: &'a str,
    deleted: bool,
}

/// The comment `reference` names. Only a comment on an issue can be changed here.
fn fetch(ws: &WriteSession, reference: &str) -> Result<Comment> {
    let id = reference.trim();
    if id.is_empty() {
        return Err(CliError::usage("the comment id is empty"));
    }
    let data: CommentQuery = ws.client.execute(&read::comment(id))?;
    let comment = data.comment;
    if comment.issue.is_none() {
        return Err(CliError::usage(format!(
            "{id} is not a comment on an issue (only those can be changed here)"
        )));
    }
    Ok(comment)
}

fn author(comment: &Comment) -> Option<&str> {
    comment.user.as_ref().map(|u| u.id.inner())
}

/// The comment a write is aimed at, named by the issue it is on.
fn comment_target(comment: &Comment) -> Target {
    Target::existing(
        "comment",
        format!("on {}", issue_of(comment)),
        comment.id.inner(),
    )
}

fn issue_of(comment: &Comment) -> &str {
    comment.issue.as_ref().map_or("", |i| i.identifier.as_str())
}

pub fn update(ctx: &Ctx, cmd: &UpdateCmd) -> Result<()> {
    let body = read_text(&cmd.body_file)?;
    let body = body.trim();
    if body.is_empty() {
        return Err(CliError::usage("the comment is empty"));
    }

    let ws = ctx.write_session_with(cmd.force)?;
    let comment = fetch(&ws, &cmd.comment)?;
    ws.guard(
        &format!("comment {} on {}", cmd.comment, issue_of(&comment)),
        &Write::CommentUpdate {
            author: author(&comment),
        },
        false,
    )?;

    let changed = comment.body.trim() != body;
    if ws.dry_run {
        if changed {
            let input = CommentUpdateInput {
                body: body.to_owned(),
            };
            ws.record(&inputs::comment_update(comment.id.inner(), input));
        }
        return ws.finish_dry_run(
            Plan::new("comment update", comment_target(&comment))
                .changed(if changed { vec!["body"] } else { Vec::new() })
                .reason("the comment already has this text"),
        );
    }
    let comment = if changed {
        let input = CommentUpdateInput {
            body: body.to_owned(),
        };
        let data: CommentUpdate = ws
            .client
            .execute(&inputs::comment_update(comment.id.inner(), input))?;
        if !data.comment_update.success {
            return Err(CliError::general("Linear could not update the comment"));
        }
        data.comment_update.comment
    } else {
        comment
    };

    let value = Updated {
        workspace: &ws.workspace,
        id: comment.id.inner(),
        url: &comment.url,
        issue: issue_of(&comment),
        changed,
    };
    ws.emit(
        &value,
        || {
            let note = if changed {
                "updated"
            } else {
                "already this text, nothing sent"
            };
            format!("{}  {}  ({note})", issue_of(&comment), comment.url)
        },
        || comment.url.clone(),
    );
    Ok(())
}

pub fn delete(ctx: &Ctx, cmd: &DeleteCmd) -> Result<()> {
    // Judged before anything is sent: a deletion nobody confirmed is a usage error.
    if !cmd.yes {
        return Err(CliError::usage(
            "deleting a comment cannot be undone; pass --yes to delete it",
        ));
    }

    let ws = ctx.write_session_with(cmd.force)?;
    let comment = fetch(&ws, &cmd.comment)?;
    // Refused in a lenient workspace too: this is the one write of a comment that cannot be
    // taken back.
    ws.guard(
        &format!("comment {} on {}", cmd.comment, issue_of(&comment)),
        &Write::CommentDelete {
            author: author(&comment),
        },
        false,
    )?;

    let op = inputs::comment_delete(comment.id.inner());
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new("comment delete", comment_target(&comment)));
    }
    let data: CommentDelete = ws.client.execute(&op)?;
    if !data.comment_delete.success {
        return Err(CliError::general("Linear could not delete the comment"));
    }
    let value = Deleted {
        workspace: &ws.workspace,
        id: comment.id.inner(),
        issue: issue_of(&comment),
        deleted: true,
    };
    ws.emit(
        &value,
        || format!("{}  deleted {}", issue_of(&comment), comment.url),
        || comment.id.inner().to_owned(),
    );
    Ok(())
}
