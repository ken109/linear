//! A minimal scan of a GraphQL document, enough to tell which kinds of
//! operation it defines.
//!
//! `linear api` sends user-written documents. It must refuse a mutation by
//! default, so it has to know what a document contains *before* sending it.
//! This is not a validator (Linear validates against its schema); it only
//! tokenizes far enough to find the keyword that starts each top-level
//! definition. Strings, block strings and comments are skipped so that a
//! `mutation` inside them is not mistaken for an operation.

use crate::error::{Error, Result};

/// The kind of a top-level operation definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Query,
    Mutation,
    Subscription,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Mutation => "mutation",
            Self::Subscription => "subscription",
        }
    }
}

/// The kinds of all operations the document defines, in order.
///
/// Fragment definitions are skipped. A shorthand `{ ... }` is a query.
/// Fails when the document is not balanced (an unterminated string or an
/// unmatched bracket) or defines no operation.
pub fn operation_kinds(document: &str) -> Result<Vec<OperationKind>> {
    let mut kinds = Vec::new();
    let mut depth: usize = 0;
    // True before the first token of a top-level definition.
    let mut at_definition_start = true;
    let mut chars = document.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        match c {
            // Whitespace, commas and the byte order mark are all insignificant.
            c if c.is_whitespace() || c == ',' || c == '\u{feff}' => {}
            '#' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' || c == '\r' {
                        break;
                    }
                }
            }
            '"' => skip_string(document, i, &mut chars)?,
            '{' => {
                if depth == 0 && at_definition_start {
                    kinds.push(OperationKind::Query);
                }
                at_definition_start = false;
                depth += 1;
            }
            '(' | '[' => {
                at_definition_start = false;
                depth += 1;
            }
            '}' | ')' | ']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::Usage(format!("unbalanced `{c}` in the document")))?;
                // Only a closing brace ends a definition; `)` and `]` never do.
                if depth == 0 && c == '}' {
                    at_definition_start = true;
                }
            }
            c if is_name_start(c) => {
                let mut end = i + c.len_utf8();
                while let Some(&(j, n)) = chars.peek() {
                    if is_name_continue(n) {
                        end = j + n.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                if depth == 0 && at_definition_start {
                    match &document[i..end] {
                        "query" => kinds.push(OperationKind::Query),
                        "mutation" => kinds.push(OperationKind::Mutation),
                        "subscription" => kinds.push(OperationKind::Subscription),
                        _ => {}
                    }
                    at_definition_start = false;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(Error::Usage("unbalanced brackets in the document".into()));
    }
    if kinds.is_empty() {
        return Err(Error::Usage(
            "the document defines no operation (expected `query`, `mutation` or `{ ... }`)".into(),
        ));
    }
    Ok(kinds)
}

fn is_name_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_name_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Consume a string whose opening quote is at byte `start`. Handles `"..."`
/// with escapes and `"""..."""` block strings (where only `\"""` escapes).
fn skip_string(
    document: &str,
    start: usize,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Result<()> {
    let unterminated = || Error::Usage("unterminated string in the document".into());
    if document[start..].starts_with("\"\"\"") {
        chars.next();
        chars.next();
        while let Some((j, c)) = chars.next() {
            if c == '\\' && document[j..].starts_with("\\\"\"\"") {
                chars.next();
                chars.next();
                chars.next();
            } else if c == '"' && document[j..].starts_with("\"\"\"") {
                chars.next();
                chars.next();
                return Ok(());
            }
        }
        return Err(unterminated());
    }
    while let Some((_, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' => return Ok(()),
            // A plain string cannot span lines.
            '\n' | '\r' => return Err(unterminated()),
            _ => {}
        }
    }
    Err(unterminated())
}
