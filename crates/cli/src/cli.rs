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

    /// Give up on a request to Linear after this many seconds (default 30; LINEAR_TIMEOUT).
    /// A read that fails on a timeout, a 5xx or a short rate limit is retried twice; a write never is
    #[arg(long, global = true, value_name = "SECS")]
    pub timeout: Option<u64>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage workspaces and credentials
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// Read and write issues
    #[command(subcommand)]
    Issue(crate::commands::issue::IssueCommand),
    /// Read and write projects
    #[command(subcommand)]
    Project(crate::commands::project::ProjectCommand),
    /// Read and write project milestones
    #[command(subcommand)]
    Milestone(crate::commands::milestone::MilestoneCommand),
    /// Read initiatives, and create one
    #[command(subcommand)]
    Initiative(crate::commands::initiative::InitiativeCommand),
    /// Read issue templates and their sections, and create one
    #[command(subcommand)]
    Template(crate::commands::template::TemplateCommand),
    /// Read labels
    #[command(subcommand)]
    Label(crate::commands::label::LabelCommand),
    /// Read teams
    #[command(subcommand)]
    Team(crate::commands::team::TeamCommand),
    /// Read users
    #[command(subcommand)]
    User(crate::commands::user::UserCommand),
    /// Find the cycle a meeting's commitments go into: the one that contains the day after it
    ///
    /// Read-only. Fails (exit 2, listing the cycles that exist) when no cycle contains that
    /// day, so a missing cycle is never taken for "no cycle". The day is judged at noon
    /// Japan Standard Time. `issue create --held-on` uses the same rule.
    Cycle(crate::commands::cycle::CycleArgs),
    /// Find Linear data that has drifted: stale work, outdated status updates, inconsistent states
    ///
    /// Without --workspace (or LINEAR_WORKSPACE, or a .linear.toml) every configured
    /// workspace is audited. A finding is `actionable` when you own its target (you lead
    /// the project, or the issue is assigned to you) and informational otherwise. The exit
    /// code is 0 whatever is found; `--fail-on actionable` makes actionable findings exit
    /// with code 6.
    Audit(crate::commands::audit::AuditArgs),
    /// Manage the cache that hooks and the statusline read
    #[command(subcommand)]
    Cache(crate::commands::cache::CacheCommand),
    /// One line from the cache for a statusline or hook: what is In Progress and what needs fixing
    ///
    /// Reads the cache only (no credentials, no request) and returns at once. For each
    /// workspace: `main: 3 in progress, 1 actionable`. A snapshot older than --ttl, or none,
    /// prints `main: unknown (snapshot 2h old)` and no numbers: old figures are never shown as
    /// current. Workspaces are joined with ` | `. The exit code is 0 whatever the state; with
    /// --json every workspace has `state` (fresh, expired, missing or unusable). Keeping the
    /// cache fresh is the caller's job: run `linear cache refresh` in the background.
    Status(crate::commands::status::StatusArgs),
    /// Where each unfinished project stands, as markdown: health, latest status update, stale mark
    ///
    /// Asks Linear for the projects that are not finished and shows those that are in progress
    /// or have a status update: the health and the first lines of the latest update, how far
    /// the milestones are, and a mark when there is no update or it is 14 days old or more
    /// (`--stale-days`, or `audit.status_update_days` of the workspace). Newest update first.
    /// --json prints the same facts as data. `--session` is for a SessionStart hook: it prints
    /// nothing in CI, prints nothing and exits 0 on any failure, and gives up after 4 seconds.
    Brief(crate::commands::brief::BriefArgs),
    /// Send a raw GraphQL document and print the response data (queries; mutations only with --mutation)
    Api(ApiArgs),
}

#[derive(Debug, Args)]
pub struct ApiArgs {
    /// The GraphQL document; `-` reads it from standard input
    #[arg(value_name = "QUERY", required_unless_present = "query_file")]
    pub query: Option<String>,
    /// Read the GraphQL document from a file
    #[arg(long, value_name = "FILE", conflicts_with = "query")]
    pub query_file: Option<std::path::PathBuf>,
    /// Set a string variable (repeatable)
    #[arg(long = "var", value_name = "KEY=VALUE")]
    pub var: Vec<String>,
    /// Set a variable from a JSON value, e.g. `--var-json first=5` (repeatable)
    #[arg(long = "var-json", value_name = "KEY=JSON")]
    pub var_json: Vec<String>,
    /// Read variables from a JSON object in a file (`-` for standard input);
    /// --var and --var-json override it
    #[arg(long, value_name = "FILE")]
    pub variables_file: Option<std::path::PathBuf>,
    /// Which operation to run when the document defines several
    #[arg(long, value_name = "NAME")]
    pub operation_name: Option<String>,
    /// Allow a mutation. Needs `allow_raw_mutation = true` for the workspace in
    /// workspaces.toml (exit 4 otherwise). WARNING: the ownership rules and the validators
    /// do NOT apply to a raw mutation; nothing checks whose project or issue it changes.
    /// A warning is printed to stderr first (not with --json or --quiet)
    #[arg(long)]
    pub mutation: bool,
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
    /// How to authenticate (`client-credentials` is for CI: an app's client id and secret)
    #[arg(long, value_enum, default_value_t = AuthArg::ApiKey)]
    pub auth: AuthArg,
    /// The app's client id, with --auth client-credentials (or set LINEAR_CLIENT_ID). The
    /// client secret is never stored: it comes from LINEAR_CLIENT_SECRET
    #[arg(long, value_name = "ID")]
    pub client_id: Option<String>,
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
    #[value(alias = "client_credentials")]
    ClientCredentials,
}
