//! `linear milestone list|view|create|update|delete` (the writes live in `write::milestone`).

use super::format::{fields, milestone_status, opt_date, opt_text, percent};
use super::listing::{resolve_project, Session};
use super::{write, Ctx};
use crate::error::Result;
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::matching::match_milestone;
use linear_core::read::{self, MilestoneDetail, MilestoneView, MilestonesOfProject};
use linear_core::types::Milestone;
use linear_core::InWorkspace;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum MilestoneCommand {
    /// List the milestones of a project
    List(ListCmd),
    /// Show one milestone with its issues
    View(ViewCmd),
    /// Add a milestone to a project you lead (a name already there returns that one)
    Create(write::milestone::CreateCmd),
    /// Rename a milestone, or change its target date or description
    Update(write::milestone::UpdateCmd),
    /// Delete a milestone that has no issues left in it
    Delete(write::milestone::DeleteCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Milestone name (ignoring case) or id
    pub milestone: String,
    /// Project: id, slug id, URL or name
    #[arg(long, value_name = "PROJECT")]
    pub project: String,
}

pub fn run(ctx: &Ctx, cmd: &MilestoneCommand) -> Result<()> {
    match cmd {
        MilestoneCommand::List(args) => list(ctx, args),
        MilestoneCommand::View(args) => view(ctx, args),
        MilestoneCommand::Create(args) => write::milestone::create(ctx, args),
        MilestoneCommand::Update(args) => write::milestone::update(ctx, args),
        MilestoneCommand::Delete(args) => write::milestone::delete(ctx, args),
    }
}

/// The milestones of a project, in the order Linear shows them.
fn milestones(session: &Session, project: &str) -> Result<Vec<Milestone>> {
    let project = resolve_project(&session.client, project)?;
    let data: MilestonesOfProject = session
        .client
        .execute(&read::milestones_of_project(project.id.inner()))?;
    let mut rows: Vec<Milestone> = data.project.project_milestones.into();
    rows.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
    Ok(rows)
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let rows = milestones(&session, &args.project)?;
    let tagged = InWorkspace::tag_all(&session.workspace, rows.clone());
    ctx.out.emit_selectable(
        &tagged,
        || {
            if rows.is_empty() {
                return "No milestones found.".to_owned();
            }
            let body: Vec<Vec<String>> = rows
                .iter()
                .map(|m| {
                    vec![
                        m.name.clone(),
                        milestone_status(&m.status),
                        opt_date(&m.target_date),
                        percent(m.progress),
                    ]
                })
                .collect();
            table(&["NAME", "STATUS", "TARGET", "PROGRESS"], &body)
        },
        || {
            rows.iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

#[derive(Serialize)]
struct MilestoneViewOut<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    milestone: &'a Milestone,
    #[serde(flatten)]
    detail: &'a MilestoneDetail,
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let session = ctx.session()?;
    let rows = milestones(&session, &args.project)?;
    let found = match_milestone(&rows, &args.milestone)?;
    let data: MilestoneView = session
        .client
        .execute(&read::milestone_view(found.id.inner()))?;
    let (m, d) = (&data.project_milestone, &data.detail);

    let value = MilestoneViewOut {
        workspace: &session.workspace,
        milestone: m,
        detail: d,
    };
    ctx.out.emit_selectable(
        &value,
        || {
            let mut text = format!(
                "{}\n\n{}",
                m.name,
                fields(&[
                    ("Project", m.project.name.clone()),
                    ("Status", milestone_status(&m.status)),
                    ("Target", opt_date(&m.target_date)),
                    ("Progress", percent(m.progress)),
                ])
            );
            if let Some(desc) = m.description.as_deref().filter(|s| !s.trim().is_empty()) {
                text.push_str(&format!("\n\n{}", desc.trim_end()));
            }
            text.push_str(&format!("\n\nIssues ({})", d.issues.len()));
            if !d.issues.is_empty() {
                let body: Vec<Vec<String>> = d
                    .issues
                    .iter()
                    .map(|i| {
                        vec![
                            i.identifier.clone(),
                            i.state.name.clone(),
                            opt_text(i.assignee.as_ref().map(|u| u.name.as_str())),
                            i.title.clone(),
                        ]
                    })
                    .collect();
                text.push('\n');
                text.push_str(&table(&["ID", "STATE", "ASSIGNEE", "TITLE"], &body));
            }
            text
        },
        || m.name.clone(),
    )?;
    Ok(())
}
