//! The ownership-based write guard.
//!
//! Writes are limited by who owns what. This is the `strict` behaviour, the
//! default of the workspace's `ownership` setting:
//!
//! - A **project** may be written (created or changed) only when its lead is
//!   the viewer.
//! - An **issue** may be changed when it is assigned to the viewer or its
//!   project is led by the viewer.
//! - An issue may be **created** in a project the viewer leads (assigned to
//!   anyone), or in no project when assigned to the viewer. Creating one in a
//!   project somebody else leads is refused unless the caller passes
//!   `--allow-foreign` *and* the issue is assigned to the viewer. That is the
//!   only thing the flag allows.
//!
//! A workspace set to `lenient` ([`Ownership::Lenient`]) relaxes the issue
//! rules for teams that work on each other's issues: an issue may be created
//! for anyone, in any project, and an issue may be changed by anyone. What
//! stays refused is the same in both: writing a project the viewer does not
//! lead, canceling an issue that is not the viewer's ([`Write::IssueCancel`]) and
//! deleting somebody else's comment ([`Write::CommentDelete`]).
//!
//! A **comment** is its author's: strict, only the author may edit or delete it
//! ([`Write::CommentUpdate`], [`Write::CommentDelete`]). A lenient workspace lets
//! anyone edit a comment (as it lets anyone change an issue), but still not delete
//! one that is not theirs.
//!
//! "Me" is the workspace's viewer ([`Viewer`]); the CLI fetches and caches it
//! per workspace. Everything here is a pure decision over ids; a refusal is a
//! [`Denied`], which maps to exit code 4 ([`ErrorCode::WriteDenied`]), and it
//! happens before anything is sent.

use crate::config::Ownership;
use crate::error::ErrorCode;
use crate::rules::Operation;
use crate::types::{Issue, Project, User};
use serde::{Deserialize, Serialize};
use std::fmt;

/// "Me" in one workspace: the viewer the credentials belong to.
///
/// Serializable so the CLI can cache it per workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub workspace: String,
    /// The viewer's Linear user id.
    pub id: String,
}

impl Viewer {
    pub fn new(workspace: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            id: id.into(),
        }
    }

    pub fn from_user(workspace: impl Into<String>, user: &User) -> Self {
        Self::new(workspace, user.id.inner())
    }

    fn is(&self, id: Option<&str>) -> bool {
        id == Some(self.id.as_str())
    }
}

/// Where an issue lives, as far as ownership is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement<'a> {
    /// The issue belongs to no project.
    NoProject,
    /// The issue is in a project with this lead (`None`: nobody leads it).
    Project { lead: Option<&'a str> },
}

impl<'a> Placement<'a> {
    pub fn of_project(project: &'a Project) -> Self {
        Self::Project {
            lead: project.lead.as_ref().map(|u| u.id.inner()),
        }
    }
}

/// A write, described by the ids that decide whether it is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write<'a> {
    /// Create a project that will have this lead.
    ProjectCreate { lead: Option<&'a str> },
    /// Change a project that currently has this lead.
    ProjectUpdate { lead: Option<&'a str> },
    /// Change an initiative that currently has this owner (the CLI writes
    /// documents under an initiative, and nothing else).
    InitiativeUpdate { owner: Option<&'a str> },
    /// Create an issue with this assignee in this place.
    IssueCreate {
        assignee: Option<&'a str>,
        placement: Placement<'a>,
    },
    /// Change an issue as it is now. `moves_to` is the destination when the
    /// change moves it to another project.
    IssueUpdate {
        assignee: Option<&'a str>,
        placement: Placement<'a>,
        moves_to: Option<Placement<'a>>,
    },
    /// Cancel (or mark as a duplicate) an issue as it is now. Asked in addition
    /// to [`Write::IssueUpdate`] when the change sets such a state: closing
    /// somebody else's work needs ownership even in a lenient workspace.
    IssueCancel {
        assignee: Option<&'a str>,
        placement: Placement<'a>,
    },
    /// Edit a comment written by this user (`None`: a comment with no author, such as
    /// one made by an integration).
    CommentUpdate { author: Option<&'a str> },
    /// Delete a comment written by this user. Asked as strictly in a lenient workspace:
    /// a deletion cannot be taken back.
    CommentDelete { author: Option<&'a str> },
}

impl<'a> Write<'a> {
    pub fn operation(&self) -> Operation {
        match self {
            Self::ProjectCreate { .. } => Operation::ProjectCreate,
            Self::ProjectUpdate { .. } => Operation::ProjectUpdate,
            Self::InitiativeUpdate { .. } => Operation::DocumentUpdate,
            Self::IssueCreate { .. } => Operation::IssueCreate,
            // A comment is a write to its issue.
            Self::IssueUpdate { .. }
            | Self::IssueCancel { .. }
            | Self::CommentUpdate { .. }
            | Self::CommentDelete { .. } => Operation::IssueUpdate,
        }
    }

    /// An update of `issue`, whose project (resolved with its lead) is `placement`.
    pub fn update_issue(issue: &'a Issue, placement: Placement<'a>) -> Self {
        Self::IssueUpdate {
            assignee: issue.assignee.as_ref().map(|u| u.id.inner()),
            placement,
            moves_to: None,
        }
    }

    /// An update of `project`.
    pub fn update_project(project: &'a Project) -> Self {
        Self::ProjectUpdate {
            lead: project.lead.as_ref().map(|u| u.id.inner()),
        }
    }
}

/// Why a write was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// The project's lead is not the viewer.
    ProjectNotLed,
    /// The issue is neither assigned to the viewer nor in a project the viewer leads.
    IssueNotOwned,
    /// The project is led by someone else (or nobody).
    ForeignProject,
    /// The comment was written by someone else (or by nobody).
    CommentNotOwned,
    /// The initiative is owned by someone else (or nobody).
    InitiativeNotOwned,
}

/// A write refused by the ownership rules. Maps to exit code 4
/// ([`ErrorCode::WriteDenied`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct Denied {
    pub operation: Operation,
    pub reason: DenyReason,
    pub message: String,
}

impl Denied {
    pub fn code(&self) -> ErrorCode {
        ErrorCode::WriteDenied
    }
}

fn deny(op: Operation, reason: DenyReason, message: impl fmt::Display) -> Result<(), Denied> {
    Err(Denied {
        operation: op,
        reason,
        message: message.to_string(),
    })
}

/// Decide whether the viewer may perform `write` under the strict rules.
///
/// `allow_foreign` is `--allow-foreign`. It matters only for
/// [`Write::IssueCreate`] in a project somebody else leads, and only when the
/// issue is assigned to the viewer; anywhere else it changes nothing.
pub fn check(viewer: &Viewer, write: &Write<'_>, allow_foreign: bool) -> Result<(), Denied> {
    check_with(viewer, Ownership::Strict, write, allow_foreign)
}

/// [`check`] under the workspace's `ownership` setting.
pub fn check_with(
    viewer: &Viewer,
    ownership: Ownership,
    write: &Write<'_>,
    allow_foreign: bool,
) -> Result<(), Denied> {
    let op = write.operation();
    if ownership == Ownership::Lenient
        && matches!(
            write,
            Write::IssueCreate { .. } | Write::IssueUpdate { .. } | Write::CommentUpdate { .. }
        )
    {
        return Ok(());
    }
    match *write {
        Write::ProjectCreate { lead } => {
            if viewer.is(lead) {
                Ok(())
            } else {
                deny(
                    op,
                    DenyReason::ProjectNotLed,
                    "a project you create must have you as its lead",
                )
            }
        }
        Write::ProjectUpdate { lead } => {
            if viewer.is(lead) {
                Ok(())
            } else {
                deny(
                    op,
                    DenyReason::ProjectNotLed,
                    format!("you are not the lead of this project ({})", who(lead)),
                )
            }
        }
        Write::InitiativeUpdate { owner } => {
            if viewer.is(owner) {
                Ok(())
            } else {
                deny(
                    op,
                    DenyReason::InitiativeNotOwned,
                    format!(
                        "you are not the owner of this initiative ({})",
                        match owner {
                            Some(_) => "owned by someone else",
                            None => "it has no owner",
                        }
                    ),
                )
            }
        }
        Write::IssueCreate {
            assignee,
            placement,
        } => match placement {
            Placement::NoProject => {
                if viewer.is(assignee) {
                    Ok(())
                } else {
                    deny(
                        op,
                        DenyReason::IssueNotOwned,
                        "an issue without a project must be assigned to you",
                    )
                }
            }
            Placement::Project { lead } if viewer.is(lead) => Ok(()),
            Placement::Project { lead } => match (viewer.is(assignee), allow_foreign) {
                (true, true) => Ok(()),
                (true, false) => deny(
                    op,
                    DenyReason::ForeignProject,
                    format!(
                        "this project is not yours ({}); to create an issue assigned to you in it, pass --allow-foreign",
                        who(lead)
                    ),
                ),
                (false, _) => deny(
                    op,
                    DenyReason::ForeignProject,
                    format!(
                        "this project is not yours ({}); --allow-foreign only covers issues assigned to you",
                        who(lead)
                    ),
                ),
            },
        },
        Write::IssueCancel {
            assignee,
            placement,
        } => {
            if owns_issue(viewer, assignee, placement) {
                Ok(())
            } else {
                deny(
                    op,
                    DenyReason::IssueNotOwned,
                    "canceling an issue needs it to be assigned to you or its project to be led by you",
                )
            }
        }
        Write::CommentUpdate { author } | Write::CommentDelete { author } => {
            if viewer.is(author) {
                Ok(())
            } else {
                let what = if matches!(write, Write::CommentDelete { .. }) {
                    "delete"
                } else {
                    "edit"
                };
                deny(
                    op,
                    DenyReason::CommentNotOwned,
                    format!("you can only {what} your own comments (this one was written by {})", writer(author)),
                )
            }
        }
        Write::IssueUpdate {
            assignee,
            placement,
            moves_to,
        } => {
            if !owns_issue(viewer, assignee, placement) {
                return deny(
                    op,
                    DenyReason::IssueNotOwned,
                    "this issue is not assigned to you and its project is not led by you",
                );
            }
            match moves_to {
                // Moving into a project somebody else leads is refused:
                // `--allow-foreign` covers creating there, not moving there.
                Some(Placement::Project { lead }) if !viewer.is(lead) => deny(
                    op,
                    DenyReason::ForeignProject,
                    format!("the destination project is not yours ({})", who(lead)),
                ),
                _ => Ok(()),
            }
        }
    }
}

fn owns_issue(viewer: &Viewer, assignee: Option<&str>, placement: Placement<'_>) -> bool {
    viewer.is(assignee) || matches!(placement, Placement::Project { lead } if viewer.is(lead))
}

fn writer(author: Option<&str>) -> &'static str {
    match author {
        Some(_) => "someone else",
        None => "nobody (an integration or a deleted user)",
    }
}

fn who(lead: Option<&str>) -> &'static str {
    match lead {
        Some(_) => "led by someone else",
        None => "it has no lead",
    }
}
