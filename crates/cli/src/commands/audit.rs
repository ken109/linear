//! Fetching what `audit` looks at.

use super::listing::paginate;
use crate::error::{CliError, Result};
use crate::http::Client;
use chrono::{DateTime, Duration, Utc};
use linear_core::audit::{AuditConfig, Snapshot};
use linear_core::config::Rule;
use linear_core::filters;
use linear_core::queries::{self, IssueById, Projects, Templates, PROJECTS_PAGE_SIZE};
use linear_core::read::{self, IssueList, IssueListVars, ISSUE_LIST_PAGE_SIZE};
use linear_core::types::Issue;

/// Fetch a workspace's issues, projects and (when `template-sections` is
/// enabled) templates.
///
/// A project lists only its first 100 issues, so the issues are fetched on
/// their own, from the issue side: every open issue, and every issue updated
/// recently enough to matter. A state change updates the issue, and a state
/// change only matters to a project's status update while that update is
/// younger than `status_update_days`, so a day beyond that is enough.
///
/// `named` are issue identifiers that must be in the snapshot even when they
/// are old and closed (`--issues`). Those that do not exist in this workspace
/// are left out; the audit reports them as unresolved.
pub fn collect(
    client: &Client,
    workspace: &str,
    config: &AuditConfig,
    named: &[String],
    now: DateTime<Utc>,
) -> Result<Snapshot> {
    let since = now - Duration::days(i64::from(config.status_update_days) + 1);
    let filter = filters::audit_issues(since);
    let mut issues = paginate(ISSUE_LIST_PAGE_SIZE, None, |page| {
        let vars = IssueListVars::new(page, Some(filter.clone()));
        let data: IssueList = client.execute(&read::issue_list(vars))?;
        Ok(data.issues)
    })?
    .items;

    for identifier in named {
        if issues
            .iter()
            .any(|i| i.identifier.eq_ignore_ascii_case(identifier.trim()))
        {
            continue;
        }
        if let Some(issue) = fetch_named(client, identifier)? {
            if !issues.iter().any(|i| i.id == issue.id) {
                issues.push(issue);
            }
        }
    }

    let projects = paginate(PROJECTS_PAGE_SIZE, None, |vars| {
        let data: Projects = client.execute(&queries::projects(vars))?;
        Ok(data.projects)
    })?
    .items;

    let templates = if config.validators.contains(&Rule::TemplateSections) {
        let data: Templates = client.execute(&queries::templates())?;
        data.templates
    } else {
        Vec::new()
    };

    Ok(Snapshot {
        workspace: workspace.to_owned(),
        issues,
        projects,
        templates,
    })
}

/// One issue by identifier, or `None` when this workspace has no such issue.
fn fetch_named(client: &Client, identifier: &str) -> Result<Option<Issue>> {
    match client.execute::<_, _, IssueById>(&queries::issue(identifier.trim())) {
        Ok(data) => Ok(Some(data.issue)),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Linear answers an unknown issue with an "Entity not found" API error.
fn is_not_found(e: &CliError) -> bool {
    e.message.to_ascii_lowercase().contains("not found")
}
