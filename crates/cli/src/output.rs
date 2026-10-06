//! Output formatting: human tables, `--json`, `--quiet`, and error reporting.
//!
//! Results go to stdout; diagnostics and errors go to stderr.

use crate::error::CliError;
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, Default)]
pub struct Output {
    pub json: bool,
    pub quiet: bool,
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
        if !text.is_empty() {
            let mut out = std::io::stdout().lock();
            // A closed pipe (e.g. `| head`) is not an error worth reporting.
            let _ = writeln!(out, "{}", text.trim_end_matches('\n'));
        }
    }

    /// A progress or confirmation line. Silent with `--quiet` and `--json`.
    pub fn status(&self, message: &str) {
        if !self.quiet && !self.json {
            eprintln!("{message}");
        }
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
