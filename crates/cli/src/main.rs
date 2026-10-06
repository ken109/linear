//! The `linear` command.

#[cfg(test)]
mod audit_fixes;
mod cache;
mod cli;
mod commands;
mod error;
mod http;
mod output;
mod store;

use clap::{error::ErrorKind, Parser};
use cli::Cli;
use linear_core::ErrorCode;
use output::{report_error, Output};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // Needed before parsing succeeds, so a usage error can still honour --json.
    let wants_json = args
        .iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "--json");

    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    ExitCode::SUCCESS
                }
                _ => {
                    let message = e.render().to_string();
                    let message = message.trim_end().trim_start_matches("error: ");
                    // JSON consumers get the reason only; the usage text is for humans.
                    let message = if wants_json {
                        message.split("\n\n").next().unwrap_or(message)
                    } else {
                        message
                    };
                    let err = error::CliError::usage(message);
                    report_error(wants_json, &err);
                    ExitCode::from(ErrorCode::Usage.exit_code())
                }
            };
        }
    };

    let out = Output {
        json: cli.json,
        quiet: cli.quiet,
    };
    match commands::run(&cli, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(out.json, &err);
            ExitCode::from(err.code.exit_code())
        }
    }
}
