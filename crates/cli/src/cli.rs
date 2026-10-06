//! Command-line definition.

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Add a workspace to the configuration
    Add(AddArgs),
    /// Store credentials for a workspace and verify them
    Login(LoginArgs),
    /// Show who the stored credentials authenticate as
    Whoami,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Name used to refer to the workspace (lowercase letters, digits, '-' and '_')
    pub name: String,
    /// The workspace's URL key (the `<key>` in linear.app/<key>)
    #[arg(long, value_name = "KEY")]
    pub url_key: String,
    /// Default team key
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// How to authenticate
    #[arg(long, value_enum, default_value_t = AuthArg::ApiKey)]
    pub auth: AuthArg,
    /// Make this the default workspace
    #[arg(long)]
    pub default: bool,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Workspace to log in to (defaults to the resolved workspace)
    pub name: Option<String>,
    /// Read the API key from standard input instead of prompting
    #[arg(long)]
    pub with_token: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AuthArg {
    ApiKey,
    Oauth,
}
