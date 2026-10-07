//! Output formatting: human tables, `--json`, `--quiet`, and error reporting.
//!
//! Results go to stdout; diagnostics and errors go to stderr.

use crate::error::{CliError, Result};
use serde::Serialize;
use serde_json::Value;
use std::io::Write;

#[derive(Debug, Clone, Copy, Default)]
pub struct Output {
    pub json: bool,
    pub quiet: bool,
    /// What `--fields` and `--id-only` ask for. Only the read commands that print through
    /// [`Output::emit_selectable`] honour it; `commands::check_selection` refuses the flags
    /// on every other command before anything runs.
    pub select: Selection,
}

/// How much of a result to print.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Selection {
    /// Everything (the default).
    #[default]
    All,
    /// `--fields a,b,c`: only these top-level keys of each JSON object.
    Fields(&'static [String]),
    /// `--id-only`: only the `id` of each item.
    IdOnly,
}

impl Selection {
    /// From the flags as typed. Whether the combination is allowed is judged by
    /// [`Selection::check_flags`].
    pub fn from_flags(fields: &[String], id_only: bool) -> Self {
        let mut names: Vec<String> = Vec::new();
        for name in fields {
            let name = name.trim().to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        if id_only {
            Self::IdOnly
        } else if names.is_empty() {
            Self::All
        } else {
            // Built once per run and kept for its whole life, so `Output` stays `Copy`.
            Self::Fields(Box::leak(names.into_boxed_slice()))
        }
    }

    /// Whether `--fields` or `--id-only` was given.
    pub fn is_requested(&self) -> bool {
        *self != Self::All
    }

    /// The flag combinations that make no sense, as usage errors. `fields_given` and
    /// `id_only` are what was typed (before `--id-only` took precedence in [`from_flags`]).
    pub fn check_flags(
        fields_given: &[String],
        id_only: bool,
        json: bool,
        quiet: bool,
    ) -> Result<()> {
        if id_only && !fields_given.is_empty() {
            return Err(CliError::usage(
                "--id-only prints the id alone, so it cannot be combined with --fields",
            ));
        }
        if id_only && quiet {
            return Err(CliError::usage(
                "--id-only and --quiet both print one value per line; use --id-only for the id \
                 (Linear's uuid), --quiet for the short reference (such as KK-12)",
            ));
        }
        if !fields_given.is_empty() && !json {
            return Err(CliError::usage(
                "--fields picks keys of the JSON output, so it needs --json",
            ));
        }
        if fields_given.iter().any(|f| f.trim().is_empty()) {
            return Err(CliError::usage(
                "--fields has an empty name (write it as --fields id,name)",
            ));
        }
        Ok(())
    }

    /// Reduce a command's JSON value to what was asked for. A list is an array of objects
    /// and a view is one object; the keys are the ones the command prints, so the valid
    /// names can never drift from the output. With nothing in the list there is nothing to
    /// check names against, and the result is the empty list.
    pub fn apply(&self, value: Value) -> Result<Value> {
        match self {
            Self::All => Ok(value),
            Self::Fields(names) => pick_fields(names, value),
            Self::IdOnly => Ok(Value::Array(
                ids(value)?.into_iter().map(Value::String).collect(),
            )),
        }
    }
}

/// The objects of a result: the rows of a list, or the one object of a view.
fn rows(value: Value) -> Result<(Vec<serde_json::Map<String, Value>>, bool)> {
    let (items, list) = match value {
        Value::Array(items) => (items, true),
        other => (vec![other], false),
    };
    let rows = items
        .into_iter()
        .map(|item| match item {
            Value::Object(map) => Ok(map),
            _ => Err(CliError::usage(
                "this command's output has no fields to select",
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((rows, list))
}

fn pick_fields(names: &[String], value: Value) -> Result<Value> {
    let (rows, list) = rows(value)?;
    // The keys of the output, in order of first appearance (the union over the rows).
    let mut valid: Vec<&str> = Vec::new();
    for row in &rows {
        for key in row.keys() {
            if !valid.contains(&key.as_str()) {
                valid.push(key);
            }
        }
    }
    let unknown: Vec<String> = names
        .iter()
        .filter(|n| !rows.is_empty() && !valid.contains(&n.as_str()))
        .map(|n| format!("{n:?}"))
        .collect();
    if !unknown.is_empty() {
        let mut valid = valid;
        valid.sort_unstable();
        return Err(CliError::usage(format!(
            "unknown field{} {} for --fields; the valid fields are: {}",
            if unknown.len() == 1 { "" } else { "s" },
            unknown.join(", "),
            valid.join(", ")
        )));
    }
    let picked: Vec<Value> = rows
        .into_iter()
        .map(|mut row| {
            Value::Object(
                names
                    .iter()
                    .filter_map(|n| row.remove(n).map(|v| (n.clone(), v)))
                    .collect(),
            )
        })
        .collect();
    Ok(if list {
        Value::Array(picked)
    } else {
        picked.into_iter().next().unwrap_or(Value::Null)
    })
}

fn ids(value: Value) -> Result<Vec<String>> {
    let (rows, _) = rows(value)?;
    rows.into_iter()
        .map(|mut row| match row.remove("id") {
            Some(Value::String(id)) => Ok(id),
            _ => Err(CliError::usage(
                "this command's output has no `id` to print",
            )),
        })
        .collect()
}

impl Output {
    /// Print a command's result.
    ///
    /// * `--json`: `value` as pretty JSON.
    /// * `--quiet`: only `quiet` (one value per line, for scripts).
    /// * otherwise: the human-readable `human` text.
    pub fn emit<T: Serialize>(
        &self,
        value: &T,
        human: impl FnOnce() -> String,
        quiet: impl FnOnce() -> String,
    ) {
        let text = if self.json {
            serde_json::to_string_pretty(value).expect("output always serializes")
        } else if self.quiet {
            quiet()
        } else {
            human()
        };
        print_text(&text);
    }

    /// [`emit`](Self::emit) for a list or a view that `--fields` and `--id-only` may cut
    /// down. A name that is not in the output is a usage error, and nothing is printed.
    ///
    /// * `--fields a,b`: the JSON, with only those keys of each object.
    /// * `--id-only`: each item's `id`, one per line (with `--json`, a JSON array of them).
    pub fn emit_selectable<T: Serialize>(
        &self,
        value: &T,
        human: impl FnOnce() -> String,
        quiet: impl FnOnce() -> String,
    ) -> Result<()> {
        if !self.select.is_requested() {
            self.emit(value, human, quiet);
            return Ok(());
        }
        let value = serde_json::to_value(value).expect("output always serializes");
        let selected = self.select.apply(value)?;
        match (&self.select, &selected) {
            (Selection::IdOnly, Value::Array(ids)) if !self.json => {
                let lines: Vec<&str> = ids.iter().filter_map(Value::as_str).collect();
                print_text(&lines.join("\n"));
            }
            _ => self.emit(&selected, String::new, String::new),
        }
        Ok(())
    }

    /// A progress or confirmation line. Silent with `--quiet` and `--json`.
    pub fn status(&self, message: &str) {
        if !self.quiet && !self.json {
            eprintln!("{message}");
        }
    }
}

fn print_text(text: &str) {
    if !text.is_empty() {
        let mut out = std::io::stdout().lock();
        // A closed pipe (e.g. `| head`) is not an error worth reporting.
        let _ = writeln!(out, "{}", text.trim_end_matches('\n'));
    }
}

/// Report an error on stderr. With `--json`, as `{"error":{"code","message"}}`.
pub fn report_error(json: bool, err: &CliError) {
    if json {
        let body = serde_json::json!({
            "error": { "code": err.code.as_str(), "message": err.message }
        });
        eprintln!("{body}");
    } else {
        eprintln!("error: {}", err.message);
    }
}

/// Render rows as an aligned text table. Columns are separated by two spaces.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| -> String {
        let mut s = String::new();
        for (i, c) in cells.iter().enumerate() {
            if i + 1 == cols {
                s.push_str(c);
            } else {
                s.push_str(c);
                s.push_str(&" ".repeat(widths[i] - c.chars().count() + 2));
            }
        }
        s.trim_end().to_owned()
    };
    let mut out = vec![line(headers.to_vec())];
    for row in rows {
        out.push(line(row.iter().map(String::as_str).collect()));
    }
    out.join("\n")
}
