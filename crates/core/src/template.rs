//! Reading the structure of Linear's issue templates.
//!
//! A template's body is a ProseMirror document stored in `templateData`
//! (`descriptionData`). The section definitions live in Linear and nowhere
//! else: this module only reads them, it never keeps a copy.

use crate::error::{Error, Result};
use crate::types::Template;
use serde_json::Value;

/// The template body as a ProseMirror document.
///
/// `descriptionData` is a document, or a JSON document encoded in a string;
/// both are accepted. Errors when the template has no body.
pub fn description_doc(template: &Template) -> Result<Value> {
    let missing = || Error::Decode(format!("template {:?} has no body", template.name));
    let data = template.data().ok_or_else(missing)?;
    match data.get("descriptionData") {
        Some(Value::String(s)) => serde_json::from_str(s).map_err(|_| missing()),
        Some(Value::Null) | None => Err(missing()),
        Some(doc) => Ok(doc.clone()),
    }
}

/// The text of every heading in the document, in order.
pub fn headings_of(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_headings(doc, &mut out);
    out
}

fn collect_headings(node: &Value, out: &mut Vec<String>) {
    if node.get("type").and_then(Value::as_str) == Some("heading") {
        out.push(text_of(node));
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for child in children {
            collect_headings(child, out);
        }
    }
}

fn text_of(node: &Value) -> String {
    if let Some(text) = node.get("text").and_then(Value::as_str) {
        return text.to_owned();
    }
    node.get("content")
        .and_then(Value::as_array)
        .map(|children| children.iter().map(text_of).collect())
        .unwrap_or_default()
}

/// A markdown skeleton: one `## heading` per section, nothing else.
/// Sections are always written at level 2, whatever level they have in the
/// template, because that is what bodies written from the skeleton are checked
/// against.
pub fn skeleton_of(doc: &Value) -> String {
    headings_of(doc)
        .iter()
        .map(|h| format!("## {h}\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a template is for issues (as opposed to projects or documents).
pub fn is_issue_template(template: &Template) -> bool {
    template.type_ == "issue"
}
