//! `linear issue list|view|create|update|comment|link-pr|unlink|delete|archive|unarchive|reorder` (the writes live in `write::issue`).

use super::cached::{self, CachedArgs};
use super::format::{date_time, fields, indent, opt_date, opt_text, person};
use super::listing::{paginate, resolve_project, warn_truncated, ListArgs};
use super::{write, Ctx};
use crate::error::{CliError, Result};
use crate::output::table;
use chrono::Utc;
use clap::{Args, Subcommand, ValueEnum};
use linear_core::filters::{closed_since, IssueQuery};
use linear_core::matching::label_path;
use linear_core::pull_request::PullRequest;
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
    /// Create an issue (same origin URL: returns the existing one instead)
    Create(super::write::issue::CreateCmd),
    /// Change an issue's state, project, milestone, due date, assignee, description, labels or source
    ///
    /// Only the fields that differ from now are sent; a run that changes nothing sends
    /// nothing (`changed` is empty with --json). If attaching the source fails, the
    /// other fields are put back.
    Update(super::write::issue::UpdateCmd),
    /// Write a comment on an issue
    Comment(super::write::issue::CommentCmd),
    /// Link a GitHub pull request to an issue through the workspace's GitHub integration
    ///
    /// Makes the attachment that Linear keeps in sync with the pull request (its status shows
    /// in `issue view --json` under `pullRequests`, and `linear audit` warns about a merged or
    /// long-open one). Needs the GitHub integration installed in the workspace; without it
    /// the command fails and sends nothing. Linking a pull request the issue already has
    /// sends nothing. A branch named with the issue's `branchName` links its pull request
    /// without this command.
    LinkPr(super::write::issue::LinkPrCmd),
    /// Upload a file and attach it to an issue (a screenshot, a log, a document)
    ///
    /// Stores the file in the workspace (at most 25 MiB; a larger, empty or unreadable file is
    /// refused before anything is sent) and attaches its URL to the issue, with the file's name
    /// as the title unless --title says otherwise. The URL and the markdown that embeds the file
    /// in a description or comment are printed too. If attaching fails after the file was
    /// stored, the stored file is deleted again (or named in the error when that is not safe or
    /// not possible). Follows the ownership rules of changing the issue.
    AttachFile(super::write::file::AttachFileCmd),
    /// Delete the attachment of an issue that has a given URL (needs --yes)
    ///
    /// Looks the attachment up on the issue by its exact URL; an issue without one is left
    /// alone (the command says so and succeeds). Linear documents no way to bring a deleted
    /// attachment back, and whether it can be recovered is not known, so the command refuses
    /// without --yes: it prints what it would delete and sends nothing (exit code 2). Follows
    /// the ownership rules of changing the issue.
    Unlink(super::write::issue::UnlinkCmd),
    /// Move an issue to the trash (restore it with `issue unarchive`)
    ///
    /// Linear keeps a deleted issue for a while before removing it for good. Follows the
    /// ownership rules of changing the issue.
    Delete(super::write::issue::IssueTargetCmd),
    /// Archive an issue (restore it with `issue unarchive`)
    ///
    /// Follows the ownership rules of changing the issue.
    Archive(super::write::issue::IssueTargetCmd),
    /// Bring back an archived or deleted (trashed) issue
    ///
    /// Follows the ownership rules of changing the issue.
    Unarchive(super::write::issue::IssueTargetCmd),
    /// Put issues of one project in a given order
    Reorder(super::write::issue::ReorderCmd),
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
    /// Only issues completed or canceled at or after this time: `14d` (14 days back from now)
    /// or a date (YYYY-MM-DD, from 00:00 UTC). On its own it lists just those; next to --open it
    /// lists the open issues and the ones closed since (what a duplicate check wants); with
    /// --state-type, --state and the other filters it narrows them, like every filter
    #[arg(long, value_name = "SINCE")]
    pub completed_since: Option<String>,
    /// The order of the list: `default` is the order Linear returns; `manual` is `sortOrder`,
    /// the screen order that `issue reorder` writes, top first. It only means something within
    /// one project, so use it with --project. Linear cannot sort by it, so every page is
    /// fetched (and, with a limit, the first of the sorted list are kept)
    #[arg(long, value_enum, default_value_t = Order::Default)]
    pub order: Order,
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
    #[command(flatten)]
    pub cache: CachedArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Order {
    /// The order Linear returns
    Default,
    /// `sortOrder`, ascending: the screen order of the project
    Manual,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    #[command(flatten)]
    pub cache: CachedArgs,
}

pub fn run(ctx: &Ctx, cmd: &IssueCommand) -> Result<()> {
    match cmd {
        IssueCommand::List(args) => list(ctx, args),
        IssueCommand::View(args) => view(ctx, args),
        IssueCommand::Create(args) => write::issue::create(ctx, args),
        IssueCommand::Update(args) => write::issue::update(ctx, args),
        IssueCommand::Comment(args) => write::issue::comment(ctx, args),
        IssueCommand::LinkPr(args) => write::issue::link_pr(ctx, args),
        IssueCommand::AttachFile(args) => write::file::attach_file(ctx, args),
        IssueCommand::Unlink(args) => write::issue::unlink(ctx, args),
        IssueCommand::Delete(args) => write::issue::delete(ctx, args),
        IssueCommand::Archive(args) => write::issue::archive(ctx, args),
        IssueCommand::Unarchive(args) => write::issue::unarchive(ctx, args),
        IssueCommand::Reorder(args) => write::issue::reorder(ctx, args),
    }
}

/// An issue as `--json` prints it: Linear's fields, the workspace, and the
/// origin URL derived from the first attachment.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    issue: &'a Issue,
    source_url: Option<&'a str>,
}

pub(super) fn out<'a>(workspace: &'a str, issue: &'a Issue) -> IssueOut<'a> {
    IssueOut {
        workspace,
        issue,
        source_url: issue.source_url(),
    }
}

/// What `issue list --cached` can be combined with: the cache holds the issues
/// assigned to you whose state type is `started`, so only a filter that says
/// exactly that is accepted. Anything else would select issues it does not hold.
fn check_cached_filters(args: &ListCmd) -> Result<()> {
    let mut flags = Vec::new();
    if args.assignee.as_deref().is_some_and(|a| a != "me") {
        flags.push("--assignee other than `me`".to_owned());
    }
    if args.state_type.iter().any(|t| t != "started") {
        flags.push("--state-type other than `started`".to_owned());
    }
    if !args.state.is_empty() {
        flags.push("--state".to_owned());
    }
    if args.open {
        flags.push("--open".to_owned());
    }
    if args.team.is_some() {
        flags.push("--team".to_owned());
    }
    if args.project.is_some() {
        flags.push("--project".to_owned());
    }
    if args.milestone.is_some() {
        flags.push("--milestone".to_owned());
    }
    if !args.label.is_empty() {
        flags.push("--label".to_owned());
    }
    if args.source_url.is_some() {
        flags.push("--source-url".to_owned());
    }
    if args.completed_since.is_some() {
        flags.push("--completed-since".to_owned());
    }
    cached::refuse_outside_cache("the issues assigned to you that are In Progress", &flags)
}

/// The issues to list and the workspace they are from.
fn fetch_list(ctx: &Ctx, args: &ListCmd) -> Result<(String, Vec<Issue>)> {
    if args.cache.cached {
        check_cached_filters(args)?;
        let hit = cached::read(ctx, &args.cache)?;
        cached::announce(ctx, &hit);
        let mut items = hit.mine.issues;
        sort_manual(args.order, &mut items);
        if let Some(limit) = args.page.limit() {
            if items.len() > limit {
                items.truncate(limit);
                warn_truncated(items.len());
            }
        }
        return Ok((hit.workspace, items));
    }

    // Judged before anything is sent: a bad time is a usage error.
    let closed_since = args
        .completed_since
        .as_deref()
        .map(|spec| closed_since(spec, Utc::now()).map_err(CliError::usage))
        .transpose()?;

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
        closed_since,
    }
    .filter();

    // Linear cannot sort by `sortOrder`, so a manual order needs every page before it can
    // be cut to the limit.
    let manual = args.order == Order::Manual;
    let fetch_limit = if manual { None } else { args.page.limit() };
    let listing = paginate(ISSUE_LIST_PAGE_SIZE, fetch_limit, |page| {
        let vars = IssueListVars::new(page, filter.clone());
        let data: IssueList = session.client.execute(&read::issue_list(vars))?;
        Ok(data.issues)
    })?;
    let mut items = listing.items;
    sort_manual(args.order, &mut items);
    let mut truncated = listing.truncated;
    if let Some(limit) = args.page.limit().filter(|l| manual && items.len() > *l) {
        items.truncate(limit);
        truncated = true;
    }
    if truncated {
        warn_truncated(items.len());
    }
    Ok((session.workspace, items))
}

/// `--order manual`: by `sortOrder`, ascending (the sort is stable, so equal values keep
/// Linear's order).
fn sort_manual(order: Order, items: &mut [Issue]) {
    if order == Order::Manual {
        items.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
    }
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let (workspace, items) = fetch_list(ctx, args)?;
    let rows: Vec<IssueOut> = items.iter().map(|i| out(&workspace, i)).collect();
    ctx.out.emit(
        &rows,
        || {
            if items.is_empty() {
                return "No issues found.".to_owned();
            }
            let body: Vec<Vec<String>> = items
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
            items
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
    /// The GitHub pull requests the integration linked: what its attachments say, read
    /// leniently (the raw `sourceType` and `metadata` are under `attachments`).
    pull_requests: Vec<PullRequest>,
    /// Absent from the cache, which keeps the list fields only.
    #[serde(flatten)]
    detail: Option<&'a IssueDetail>,
}

/// An issue from the cache: the first one whose identifier or id is `reference`.
fn find_cached(issues: Vec<Issue>, reference: &str) -> Result<Issue> {
    let reference = reference.trim();
    issues
        .into_iter()
        .find(|i| i.identifier.eq_ignore_ascii_case(reference) || i.id.inner() == reference)
        .ok_or_else(|| {
            CliError::general(format!(
                "{reference} is not in the cache, which holds only the issues assigned to you \
                 that are In Progress; drop --cached to ask Linear"
            ))
        })
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    if args.cache.cached {
        let hit = cached::read(ctx, &args.cache)?;
        let issue = find_cached(hit.mine.issues.clone(), &args.issue)?;
        cached::announce(ctx, &hit);
        return show(ctx, &hit.workspace, &issue, None);
    }
    let session = ctx.session()?;
    let data: IssueView = session.client.execute(&read::issue_view(&args.issue))?;
    show(ctx, &session.workspace, &data.issue, Some(&data.detail))
}

fn show(ctx: &Ctx, workspace: &str, i: &Issue, d: Option<&IssueDetail>) -> Result<()> {
    let value = IssueViewOut {
        base: out(workspace, i),
        pull_requests: i.pull_requests(),
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
                    (
                        "Priority",
                        d.map_or("(not in the cache)".to_owned(), |d| d
                            .priority_label
                            .clone())
                    ),
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
                    ("Branch", i.branch_name.clone()),
                    ("Created", date_time(&i.created_at)),
                    ("Updated", date_time(&i.updated_at)),
                ])
            );
            if !value.pull_requests.is_empty() {
                text.push_str("\n\nPull requests");
                for p in &value.pull_requests {
                    text.push_str(&format!(
                        "\n  #{}  {}  {}\n    {}",
                        p.number,
                        p.status.as_str(),
                        p.title,
                        p.url
                    ));
                }
            }
            if let Some(desc) = i.description.as_deref().filter(|s| !s.trim().is_empty()) {
                text.push_str(&format!("\n\n{}", desc.trim_end()));
            }
            let comments = d.map(|d| &d.comments);
            if let Some(comments) = comments.filter(|c| !c.is_empty()) {
                text.push_str(&format!("\n\nComments ({})", comments.len()));
                for c in comments.iter() {
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
