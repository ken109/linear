//! `linear comment update|delete` (the writes live in `write::comment`).
//!
//! A comment is written with `linear issue comment`; these two correct or remove one.

use super::{write, Ctx};
use crate::error::Result;
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Show how to use these commands, briefly, for an AI agent
    Usage,
    /// Replace the text of a comment you wrote
    ///
    /// The comment is named by its id (the `id` that `issue comment --json` and
    /// `issue view --json` print). Only the comment's author may change it: in a strict
    /// workspace somebody else's comment is refused (exit 4), and so is one with no author
    /// (made by an integration). A lenient workspace lets anyone edit a comment. The new
    /// text equal to the old one sends nothing.
    Update(write::comment::UpdateCmd),
    /// Delete a comment you wrote (needs --yes: a deleted comment cannot be restored)
    ///
    /// Only the comment's author may delete it, in a lenient workspace too. Without --yes
    /// nothing is sent and the command fails with exit 2.
    Delete(write::comment::DeleteCmd),
}

pub fn run(ctx: &Ctx, cmd: &CommentCommand) -> Result<()> {
    match cmd {
        CommentCommand::Usage => unreachable!("handled before the context is built"),
        CommentCommand::Update(args) => write::comment::update(ctx, args),
        CommentCommand::Delete(args) => write::comment::delete(ctx, args),
    }
}
