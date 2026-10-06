//! `linear issue list|view`.

use super::format::{date_time, fields, indent, opt_date, opt_text, person};
use super::listing::{paginate, resolve_project, warn_truncated, ListArgs};
use super::Ctx;
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::filters::IssueQuery;
use linear_core::matching::label_path;
use linear_core::read::{
    self, IssueDetail, IssueList, IssueListVars, IssueView, ISSUE_LIST_PAGE_SIZE,
};
use linear_core::types::Issue;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// List issues, optionally narrowed by assignee, state, project, label or origin URL
    List(ListCmd),
    /// Show one issue with its description and comments
    View(ViewCmd),
}

const STATE_TYPES: [&str; 6] = [
    "triage",
    "backlog",
    "unstarted",
    "started",
    "completed",
    "canceled",
];

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Assignee: `me`, `none`, an email, or a name
    #[arg(long, value_name = "WHO")]
    pub assignee: Option<String>,
    /// State type (repeatable or comma-separated)
    #[arg(long, value_name = "TYPE", value_delimiter = ',', value_parser = STATE_TYPES)]
    pub state_type: Vec<String>,
    /// State name, ignoring case (repeatable; any of them)
    #[arg(long, value_name = "NAME")]
    pub state: Vec<String>,
    /// Only issues that are not completed or canceled
    #[arg(long, conflicts_with = "state_type")]
    pub open: bool,
    /// Team key
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,
    /// Milestone name, ignoring case
    #[arg(long, value_name = "NAME")]
    pub milestone: Option<String>,
    /// Label name, ignoring case (repeatable; the issue needs all of them)
    #[arg(long, value_name = "NAME")]
    pub label: Vec<String>,
    /// The exact URL of an attachment, i.e. where the issue came from
    #[arg(long, value_name = "URL")]
    pub source_url: Option<String>,
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
}

pub fn run(ctx: &Ctx, cmd: &IssueCommand) -> Result<()> {
    match cmd {
        IssueCommand::List(args) => list(ctx, args),
        IssueCommand::View(args) => view(ctx, args),
    }
}

/// An issue as `--json` prints it: Linear's fields, the workspace, and the
/// origin URL derived from the first attachment.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IssueOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    issue: &'a Issue,
    source_url: Option<&'a str>,
}

fn out<'a>(workspace: &'a str, issue: &'a Issue) -> IssueOut<'a> {
    IssueOut {
        workspace,
        issue,
        source_url: issue.source_url(),
    }
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let project_id = match &args.project {
        Some(reference) => Some(resolve_project(&session.client, reference)?.id.into_inner()),
        None => None,
    };
    let filter = IssueQuery {
        assignee: args.assignee.clone(),
        state_types: args.state_type.clone(),
        state_names: args.state.clone(),
        open: args.open,
        team_key: args.team.clone(),
        project_id,
        milestone: args.milestone.clone(),
        labels: args.label.clone(),
        source_url: args.source_url.clone(),
    }
    .filter();

    let listing = paginate(ISSUE_LIST_PAGE_SIZE, args.page.limit(), |page| {
        let vars = IssueListVars::new(page, filter.clone());
        let data: IssueList = session.client.execute(&read::issue_list(vars))?;
        Ok(data.issues)
    })?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }

    let rows: Vec<IssueOut> = listing
        .items
        .iter()
        .map(|i| out(&session.workspace, i))
        .collect();
    ctx.out.emit(
        &rows,
        || {
            if listing.items.is_empty() {
                return "No issues found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|i| {
                    vec![
                        i.identifier.clone(),
                        i.state.name.clone(),
                        person(&i.assignee),
                        i.project.as_ref().map_or("-".into(), |p| p.name.clone()),
                        i.title.clone(),
                    ]
                })
                .collect();
            table(&["ID", "STATE", "ASSIGNEE", "PROJECT", "TITLE"], &body)
        },
        || {
            listing
                .items
                .iter()
                .map(|i| i.identifier.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IssueViewOut<'a> {
    #[serde(flatten)]
    base: IssueOut<'a>,
    #[serde(flatten)]
    detail: &'a IssueDetail,
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let data: IssueView = session.client.execute(&read::issue_view(&args.issue))?;
    let (i, d) = (&data.issue, &data.detail);

    let value = IssueViewOut {
        base: out(&session.workspace, i),
        detail: d,
    };
    ctx.out.emit(
        &value,
        || {
            let labels = if i.labels.is_empty() {
                "-".to_owned()
            } else {
                i.labels
                    .iter()
                    .map(label_path)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let milestone =
                i.project_milestone
                    .as_ref()
                    .map_or("-".to_owned(), |m| match m.target_date {
                        Some(t) => format!("{} ({t})", m.name),
                        None => m.name.clone(),
                    });
            let mut text = format!(
                "{}  {}\n{}\n\n{}",
                i.identifier,
                i.title,
                i.url,
                fields(&[
                    ("State", format!("{} ({})", i.state.name, i.state.type_)),
                    ("Assignee", person(&i.assignee)),
                    ("Team", i.team.key.clone()),
                    (
                        "Project",
                        i.project
                            .as_ref()
                            .map_or("-".to_owned(), |p| p.name.clone())
                    ),
                    ("Milestone", milestone),
                    ("Labels", labels),
                    ("Priority", d.priority_label.clone()),
                    ("Due", opt_date(&i.due_date)),
                    (
                        "Estimate",
                        i.estimate.map_or("-".to_owned(), |e| e.to_string())
                    ),
                    (
                        "Parent",
                        i.parent
                            .as_ref()
                            .map_or("-".to_owned(), |p| p.identifier.clone())
                    ),
                    ("Source", opt_text(i.source_url())),
                    ("Created", date_time(&i.created_at)),
                    ("Updated", date_time(&i.updated_at)),
                ])
            );
            if let Some(desc) = i.description.as_deref().filter(|s| !s.trim().is_empty()) {
                text.push_str(&format!("\n\n{}", desc.trim_end()));
            }
            if !d.comments.is_empty() {
                text.push_str(&format!("\n\nComments ({})", d.comments.len()));
                for c in d.comments.iter() {
                    text.push_str(&format!(
                        "\n\n  {}, {}:\n{}",
                        person(&c.user),
                        date_time(&c.created_at),
                        indent(&c.body, 4)
                    ));
                }
            }
            text
        },
        || i.identifier.clone(),
    );
    Ok(())
}
