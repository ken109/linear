//! Turning what a person typed into the ids a mutation needs.
//!
//! Resolution is strict (see `linear_core::matching`): a name that matches
//! nothing, or more than one thing, is a usage error that lists the
//! candidates, before anything is sent.

use super::WriteSession;
use crate::commands::listing::{paginate, resolve_project};
use crate::error::{CliError, Result};
use linear_core::matching::{label_path, match_labels, match_milestone, match_team, match_user};
use linear_core::read::{
    self, ProjectOwnership, ProjectOwnershipQuery, Teams, Users, LABELS_PAGE_SIZE, TEAMS_PAGE_SIZE,
    USERS_PAGE_SIZE,
};
use linear_core::types::{Label, Milestone, Team};

/// Who a person means by `me`, an email, a name or a display name. Returns the user's id.
pub fn user_id(ws: &WriteSession, reference: &str) -> Result<String> {
    if reference.trim().eq_ignore_ascii_case("me") {
        return Ok(ws.viewer.id.clone());
    }
    let users = paginate(USERS_PAGE_SIZE, None, |page| {
        let vars = read::UserListVars::new(page, false);
        let data: Users = ws.client.execute(&read::users(vars))?;
        Ok(data.users)
    })?
    .items;
    Ok(match_user(&users, reference)?.id.inner().to_owned())
}

/// The team named by `--team`, else the workspace's `default_team`.
pub fn team(ws: &WriteSession, flag: Option<&str>) -> Result<Team> {
    let reference = flag.or(ws.default_team.as_deref()).ok_or_else(|| {
        CliError::usage(
            "no team: pass --team <KEY> or set default_team for this workspace in workspaces.toml",
        )
    })?;
    let teams = paginate(TEAMS_PAGE_SIZE, None, |page| {
        let data: Teams = ws.client.execute(&read::teams(page))?;
        Ok(data.teams)
    })?
    .items;
    Ok(match_team(&teams, reference)?.clone())
}

/// A project by id, slug id, URL or name, with its lead and milestones.
pub fn project(ws: &WriteSession, reference: &str) -> Result<ProjectOwnership> {
    let found = resolve_project(&ws.client, reference)?;
    let data: ProjectOwnershipQuery = ws
        .client
        .execute(&read::project_ownership(found.id.inner()))?;
    Ok(data.project)
}

/// One milestone of `project`, by name or id.
pub fn milestone<'a>(project: &'a ProjectOwnership, reference: &str) -> Result<&'a Milestone> {
    Ok(match_milestone(&project.project_milestones, reference)?)
}

/// Labels by name, in the order given, each exactly once.
///
/// A name that more than one label carries (the same name in several teams)
/// is ambiguous and asked to be given as an id.
pub fn labels(ws: &WriteSession, references: &[String]) -> Result<Vec<Label>> {
    if references.is_empty() {
        return Ok(Vec::new());
    }
    let all = paginate(LABELS_PAGE_SIZE, None, |page| {
        let data: read::Labels = ws.client.execute(&read::labels(page))?;
        Ok(data.issue_labels)
    })?
    .items;
    let mut chosen: Vec<Label> = Vec::new();
    for reference in references {
        let hits = match_labels(&all, reference)?;
        let label = match hits.as_slice() {
            [one] => (*one).clone(),
            many => {
                return Err(CliError::usage(format!(
                    "label {reference:?} is ambiguous; it matches {} labels ({}); pass an id",
                    many.len(),
                    many.iter()
                        .map(|l| format!("{} [{}]", label_path(l), l.id.inner()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        };
        if !chosen.iter().any(|c| c.id == label.id) {
            chosen.push(label);
        }
    }
    Ok(chosen)
}
