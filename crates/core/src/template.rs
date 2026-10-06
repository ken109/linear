//! Reading the structure of Linear's issue templates.
//!
//! A template's body is a ProseMirror document stored in `templateData`
//! (`descriptionData`). The section definitions live in Linear and nowhere
//! else: this module only reads them, it never keeps a copy.

use crate::error::{Error, Result};
use crate::types::Template;
use serde_json::{json, Value};

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

/// A ProseMirror document for a template body written as markdown.
///
/// Only what a template needs is understood: headings (`#` to `######`),
/// paragraphs, bullet (`-`, `*`) and numbered (`1.`) lists, and `**bold**`.
/// Blank lines produce nothing. Anything else is kept as paragraph text.
pub fn markdown_to_doc(markdown: &str) -> Value {
    let mut content: Vec<Value> = Vec::new();
    // The kind of list the last block is, while consecutive lines extend it.
    let mut open_list: Option<&'static str> = None;

    for raw in markdown.lines() {
        let line = raw.trim_end();
        if let Some((ordered, text)) = list_item(line) {
            let kind = if ordered {
                "ordered_list"
            } else {
                "bullet_list"
            };
            if open_list != Some(kind) {
                content.push(if ordered {
                    json!({ "type": kind, "attrs": { "order": 1 }, "content": [] })
                } else {
                    json!({ "type": kind, "content": [] })
                });
                open_list = Some(kind);
            }
            let item = json!({ "type": "list_item", "content": [paragraph(text.trim())] });
            if let Some(items) = content
                .last_mut()
                .and_then(|l| l.get_mut("content"))
                .and_then(Value::as_array_mut)
            {
                items.push(item);
            }
            continue;
        }
        open_list = None;
        if let Some((level, text)) = heading(line) {
            content.push(json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": inline(text.trim()),
            }));
        } else if !line.trim().is_empty() {
            content.push(paragraph(line.trim()));
        }
    }
    json!({ "type": "doc", "content": content })
}

/// `## Title` -> (2, "Title"). Needs a space after the hashes, like markdown.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    rest.starts_with(char::is_whitespace)
        .then(|| (hashes, rest.trim_start()))
}

/// (ordered, text) of a list item line.
fn list_item(line: &str) -> Option<(bool, &str)> {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some((false, rest));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        if let Some(rest) = line[digits..].strip_prefix('.') {
            return Some((true, rest));
        }
    }
    None
}

fn paragraph(text: &str) -> Value {
    if text.is_empty() {
        json!({ "type": "paragraph" })
    } else {
        json!({ "type": "paragraph", "content": inline(text) })
    }
}

/// Text nodes, with `**bold**` as a `strong` mark.
fn inline(text: &str) -> Vec<Value> {
    let mut nodes = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match bold_span(rest) {
            Some((before, bold, after)) => {
                if !before.is_empty() {
                    nodes.push(json!({ "type": "text", "text": before }));
                }
                nodes
                    .push(json!({ "type": "text", "text": bold, "marks": [{ "type": "strong" }] }));
                rest = after;
            }
            None => {
                nodes.push(json!({ "type": "text", "text": rest }));
                break;
            }
        }
    }
    nodes
}

/// The first `**non-empty, no asterisks**` in `text`: (before, inside, after).
fn bold_span(text: &str) -> Option<(&str, &str, &str)> {
    let mut from = 0;
    while let Some(open) = text[from..].find("**").map(|i| i + from) {
        let inner_start = open + 2;
        if let Some(len) = text[inner_start..].find('*') {
            if len > 0 && text[inner_start + len..].starts_with("**") {
                return Some((
                    &text[..open],
                    &text[inner_start..inner_start + len],
                    &text[inner_start + len + 2..],
                ));
            }
        }
        from = open + 1;
    }
    None
}
