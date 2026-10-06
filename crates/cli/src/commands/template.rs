//! `linear template list|view|skeleton|create` (the write lives in `write::template`).
//!
//! Section definitions are read from Linear every time; nothing is copied into
//! code or configuration.

use super::format::{date_time, fields, opt_text};
use super::{write, Ctx};
use crate::error::{CliError, Result};
use crate::output::table;
use clap::{Args, Subcommand};
use linear_core::matching::match_template;
use linear_core::queries::{self, Templates};
use linear_core::template::{description_doc, headings_of, is_issue_template, skeleton_of};
use linear_core::types::{Team, Template};
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List templates
    List(ListCmd),
    /// Show one template and its sections
    View(ViewCmd),
    /// Print the markdown skeleton (one `## heading` per section) of an issue template, or of all of them
    Skeleton(SkeletonCmd),
    /// Create an issue template from a markdown body (the same name returns the existing one instead)
    Create(write::template::CreateCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    /// Only templates of this type (issue, project, ...)
    #[arg(long = "type", value_name = "TYPE")]
    pub kind: Option<String>,
}

#[derive(Debug, Args)]
pub struct ViewCmd {
    /// Template name (ignoring case) or id
    pub template: String,
}

#[derive(Debug, Args)]
pub struct SkeletonCmd {
    /// Issue template name; without it, every issue template is printed under its name
    pub template: Option<String>,
}

pub fn run(ctx: &Ctx, cmd: &TemplateCommand) -> Result<()> {
    match cmd {
        TemplateCommand::List(args) => list(ctx, args),
        TemplateCommand::View(args) => view(ctx, args),
        TemplateCommand::Skeleton(args) => skeleton(ctx, args),
        TemplateCommand::Create(args) => write::template::create(ctx, args),
    }
}

fn all(ctx: &Ctx) -> Result<(String, Vec<Template>)> {
    let session = ctx.session()?;
    let data: Templates = session.client.execute(&queries::templates())?;
    Ok((session.workspace, data.templates))
}

/// A template without its (large) payload, as `--json` prints it in a list.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TemplateRow<'a> {
    workspace: &'a str,
    id: &'a cynic::Id,
    name: &'a str,
    description: Option<&'a str>,
    #[serde(rename = "type")]
    type_: &'a str,
    team: &'a Option<Team>,
    updated_at: &'a chrono::DateTime<chrono::Utc>,
}

fn row<'a>(workspace: &'a str, t: &'a Template) -> TemplateRow<'a> {
    TemplateRow {
        workspace,
        id: &t.id,
        name: &t.name,
        description: t.description.as_deref(),
        type_: &t.type_,
        team: &t.team,
        updated_at: &t.updated_at,
    }
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let (workspace, templates) = all(ctx)?;
    let shown: Vec<&Template> = templates
        .iter()
        .filter(|t| {
            args.kind
                .as_ref()
                .is_none_or(|k| t.type_.eq_ignore_ascii_case(k))
        })
        .collect();
    let rows: Vec<TemplateRow> = shown.iter().map(|t| row(&workspace, t)).collect();
    ctx.out.emit(
        &rows,
        || {
            if shown.is_empty() {
                return "No templates found.".to_owned();
            }
            let body: Vec<Vec<String>> = shown
                .iter()
                .map(|t| {
                    vec![
                        t.name.clone(),
                        t.type_.clone(),
                        t.team.as_ref().map_or("-".into(), |t| t.key.clone()),
                        date_time(&t.updated_at),
                        opt_text(t.description.as_deref()),
                    ]
                })
                .collect();
            table(&["NAME", "TYPE", "TEAM", "UPDATED", "DESCRIPTION"], &body)
        },
        || {
            shown
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}

#[derive(Serialize)]
struct TemplateViewOut<'a> {
    #[serde(flatten)]
    base: TemplateRow<'a>,
    /// The headings of the template body; null when it has no body.
    sections: Option<Vec<String>>,
    /// The decoded `templateData`.
    data: Option<serde_json::Value>,
}

fn view(ctx: &Ctx, args: &ViewCmd) -> Result<()> {
    let (workspace, templates) = all(ctx)?;
    let t = match_template(&templates, &args.template)?;
    let sections = description_doc(t).ok().map(|d| headings_of(&d));
    let value = TemplateViewOut {
        base: row(&workspace, t),
        sections: sections.clone(),
        data: t.data(),
    };
    ctx.out.emit(
        &value,
        || {
            let mut text = format!(
                "{}\n\n{}",
                t.name,
                fields(&[
                    ("Type", t.type_.clone()),
                    (
                        "Team",
                        t.team.as_ref().map_or("-".into(), |t| t.key.clone())
                    ),
                    ("Updated", date_time(&t.updated_at)),
                    ("Description", opt_text(t.description.as_deref())),
                ])
            );
            match &sections {
                Some(s) if !s.is_empty() => {
                    text.push_str("\n\nSections");
                    for (n, h) in s.iter().enumerate() {
                        text.push_str(&format!("\n  {}. {h}", n + 1));
                    }
                }
                Some(_) => text.push_str("\n\nSections: none (the body has no headings)"),
                None => text.push_str("\n\nSections: none (the template has no body)"),
            }
            text
        },
        || t.name.clone(),
    );
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkeletonOut<'a> {
    workspace: &'a str,
    name: &'a str,
    sections: Vec<String>,
    skeleton: String,
}

fn skeleton(ctx: &Ctx, args: &SkeletonCmd) -> Result<()> {
    let (workspace, templates) = all(ctx)?;
    let issue_templates: Vec<Template> = templates.into_iter().filter(is_issue_template).collect();

    let (picked, named): (Vec<&Template>, bool) = match &args.template {
        Some(name) => (vec![match_template(&issue_templates, name)?], true),
        None => (issue_templates.iter().collect(), false),
    };

    let mut outs = Vec::new();
    for t in picked {
        match description_doc(t) {
            Ok(doc) => {
                let sections = headings_of(&doc);
                if sections.is_empty() {
                    eprintln!("warning: template {:?} has no headings", t.name);
                }
                outs.push(SkeletonOut {
                    workspace: &workspace,
                    name: &t.name,
                    sections,
                    skeleton: skeleton_of(&doc),
                });
            }
            Err(e) if named => return Err(CliError::from(e)),
            // One broken template must not hide the others, but say so.
            Err(e) => eprintln!("warning: {e}; skipped"),
        }
    }
    ctx.out.emit(
        &outs,
        || {
            if named {
                return outs.first().map(|o| o.skeleton.clone()).unwrap_or_default();
            }
            outs.iter()
                .map(|o| format!("# {}\n\n{}", o.name, o.skeleton))
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            if named {
                return outs.first().map(|o| o.skeleton.clone()).unwrap_or_default();
            }
            outs.iter()
                .map(|o| format!("# {}\n\n{}", o.name, o.skeleton))
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    Ok(())
}
