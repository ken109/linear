//! Command-line definition.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "linear",
    version,
    about = "A command-line client for Linear",
    propagate_version = true
)]
pub struct Cli {
    /// Workspace to use (overrides LINEAR_WORKSPACE, .linear.toml and the config default)
    #[arg(short, long, global = true, value_name = "NAME")]
    pub workspace: Option<String>,

    /// Print machine-readable JSON (errors too, as {"error":{"code","message"}} on stderr)
    #[arg(long, global = true)]
    pub json: bool,

    /// Print only the essential value(s), one per line, and no status messages
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage workspaces and credentials
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// List configured workspaces
    List,
}
