//! `linear completions <shell>`: print a shell completion script.
//!
//! The script is generated from the command definition in `cli.rs`, so it can
//! never be out of date with the commands the binary has. It needs no
//! configuration, no credentials and no network.

use crate::cli::Cli;
use crate::error::Result;
use clap::{Args, CommandFactory};
use clap_complete::Shell;
use std::io::Write;

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to print a completion script for
    #[arg(value_enum, value_name = "SHELL")]
    pub shell: Shell,
}

/// The completion script of `shell` for the `linear` command.
pub fn script(shell: Shell) -> Vec<u8> {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, "linear", &mut buf);
    buf
}

pub fn run(args: &CompletionsArgs) -> Result<()> {
    // Rendered in memory first: `generate` panics when its writer fails, and a
    // closed pipe (`linear completions zsh | head`) must not.
    let buf = script(args.shell);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&buf).and_then(|()| out.flush());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    /// Every subcommand path of the command tree, e.g. `["issue", "list"]`.
    fn paths(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let mut path = prefix.to_vec();
            path.push(sub.get_name().to_owned());
            out.push(path.clone());
            paths(sub, &path, out);
        }
    }

    #[test]
    fn the_command_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_shell_gets_a_script_that_names_every_subcommand_and_global_flag() {
        let cmd = Cli::command();
        let mut all = Vec::new();
        paths(&cmd, &[], &mut all);
        assert!(
            all.iter().any(|p| p == &["issue", "list"]),
            "the walk found nested commands: {all:?}"
        );
        assert!(all.iter().any(|p| p == &["completions"]));

        for shell in Shell::value_variants() {
            let text = String::from_utf8(script(*shell)).expect("a script is text");
            assert!(!text.is_empty(), "{shell}: empty script");
            for path in &all {
                let leaf = path.last().unwrap();
                assert!(
                    text.contains(leaf.as_str()),
                    "{shell}: the script does not mention `{}`",
                    path.join(" ")
                );
            }
            for flag in ["workspace", "json", "quiet", "timeout"] {
                assert!(text.contains(flag), "{shell}: no --{flag}");
            }
        }
    }
}
