//! `template-sections`: the body fills every section of its template.
//!
//! The template is read from Linear and never copied into code or
//! configuration, so there is nothing to drift. A section counts as filled
//! when the body has a heading with that text and some non-blank line before
//! the next heading.

use super::{Draft, Fetched, Violation, ViolationKind};
use crate::config::Rule;
use crate::types::Template;
use serde_json::Value;

/// Why a section of the template is not satisfied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionProblem {
    pub section: String,
    /// `false`: there is no such heading. `true`: the heading is there but
    /// nothing is under it.
    pub empty: bool,
}

/// The template name to read for `draft`, if there is a template to hold it to.
///
/// A creation is checked whenever it names a template (naming none is itself
/// a violation, reported when checking). An update is checked only when it both
/// replaces the body and names the template to hold it to.
pub fn template_to_check(draft: &Draft) -> Option<&str> {
    let name = draft
        .template
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    if draft.operation.is_create() || draft.body.is_some() {
        Some(name)
    } else {
        None
    }
}

pub(super) fn check(draft: &Draft, fetched: &Fetched) -> Vec<Violation> {
    let rule = Rule::TemplateSections;
    let create = draft.operation.is_create();

    let Some(name) = template_to_check(draft) else {
        if create {
            return vec![Violation::new(
                rule,
                ViolationKind::TemplateRequired,
                None,
                "a template is required (pass --template)",
            )];
        }
        return Vec::new();
    };

    let kind = draft.operation.template_kind();
    let of_kind: Vec<&Template> = fetched
        .templates
        .iter()
        .filter(|t| t.type_ == kind.linear_type())
        .collect();
    let Some(template) = of_kind.iter().find(|t| t.name == name) else {
        let names: Vec<&str> = of_kind.iter().map(|t| t.name.as_str()).collect();
        return vec![Violation::new(
            rule,
            ViolationKind::TemplateNotFound,
            Some(name.to_owned()),
            format!(
                "no {} template named {name:?} (available: {})",
                kind.linear_type(),
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            ),
        )];
    };

    let Some(headings) = headings(template) else {
        return vec![Violation::new(
            rule,
            ViolationKind::TemplateUnreadable,
            Some(name.to_owned()),
            format!("the template {name:?} has no readable body"),
        )];
    };

    missing_sections(draft.body.as_deref().unwrap_or(""), &headings)
        .into_iter()
        .map(|p| {
            let (kind, what) = if p.empty {
                (ViolationKind::SectionEmpty, "is empty")
            } else {
                (ViolationKind::SectionMissing, "is missing")
            };
            Violation::new(
                rule,
                kind,
                Some(p.section.clone()),
                format!("section {:?} of template {name:?} {what}", p.section),
            )
        })
        .collect()
}

/// The section headings of a template, in order, or `None` when the template
/// has no readable body.
///
/// Linear stores the body as a ProseMirror document. Headings are `heading`
/// nodes; a template that was written through the API as plain markdown
/// ends up as paragraphs whose lines start with `#`, which count too.
pub fn headings(template: &Template) -> Option<Vec<String>> {
    let data = template.data()?;
    let desc = data.get("descriptionData")?;
    let doc = match desc {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v) if v.is_object() => v,
            // Not a JSON document: the string is markdown itself.
            _ => return Some(markdown_headings(s)),
        },
        v if v.is_object() => v.clone(),
        _ => return None,
    };
    let mut out = Vec::new();
    walk(&doc, &mut out);
    Some(out)
}

fn walk(node: &Value, out: &mut Vec<String>) {
    match node.get("type").and_then(Value::as_str) {
        Some("heading") => out.push(text_of(node).trim().to_owned()),
        Some("paragraph") => out.extend(markdown_headings(&text_of(node))),
        _ => {
            if let Some(children) = node.get("content").and_then(Value::as_array) {
                for c in children {
                    walk(c, out);
                }
            }
        }
    }
}

fn text_of(node: &Value) -> String {
    if let Some(t) = node.get("text").and_then(Value::as_str) {
        return t.to_owned();
    }
    node.get("content")
        .and_then(Value::as_array)
        .map(|cs| cs.iter().map(text_of).collect::<String>())
        .unwrap_or_default()
}

fn markdown_headings(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(heading_text)
        .map(str::to_owned)
        .collect()
}

/// The text of a markdown heading line (`## Goal` gives `Goal`), if it is one.
fn heading_text(line: &str) -> Option<&str> {
    let line = line.trim_start();
    line.starts_with('#')
        .then(|| line.trim_start_matches('#').trim())
}

/// The sections of `headings` that `body` does not satisfy, in template order.
///
/// A section is satisfied by a heading line with the same text and at least
/// one non-blank line before the next heading. A section that is present but
/// empty is reported as such, so an empty skeleton cannot be submitted.
pub fn missing_sections(body: &str, headings: &[String]) -> Vec<SectionProblem> {
    let lines: Vec<&str> = body.lines().collect();
    let mut problems = Vec::new();

    for h in headings {
        let h = h.trim();
        let at = lines.iter().position(|l| heading_text(l) == Some(h));
        let Some(at) = at else {
            problems.push(SectionProblem {
                section: h.to_owned(),
                empty: false,
            });
            continue;
        };
        let rest = &lines[at + 1..];
        let until = rest
            .iter()
            .position(|l| l.trim_start().starts_with('#'))
            .unwrap_or(rest.len());
        if rest[..until].iter().all(|l| l.trim().is_empty()) {
            problems.push(SectionProblem {
                section: h.to_owned(),
                empty: true,
            });
        }
    }
    problems
}
