//! Command implementations.

mod api;
pub mod audit;
pub mod brief;
pub mod cache;
pub mod cached;
pub mod completions;
pub mod cycle;
mod format;
pub mod initiative;
pub mod issue;
pub mod label;
mod listing;
pub mod milestone;
pub mod project;
pub mod status;
pub mod team;
pub mod template;
pub mod user;
mod workspace;
pub mod write;

use crate::cli::{Cli, Command};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::Output;
use crate::store::{self, CredentialSource, Dirs};
use linear_core::auth::{client_secret_env_var, AuthMethod, CLIENT_SECRET_ENV};
use linear_core::config::{self, Config, Resolved, Selectors};
use linear_core::queries::{self, Whoami};

/// What every command needs.
pub struct Ctx {
    pub dirs: Dirs,
    pub out: Output,
    pub workspace_flag: Option<String>,
}

pub fn run(cli: &Cli, out: Output) -> Result<()> {
    // These touch neither the configuration nor Linear, so they must work on a
    // machine that has no config directory and no network.
    if let Command::Completions(args) = &cli.command {
        return completions::run(args);
    }
    crate::http::configure(crate::http::Settings::resolve(cli.timeout)?);
    let ctx = Ctx {
        dirs: Dirs::from_env()?,
        out,
        workspace_flag: cli.workspace.clone(),
    };
    match &cli.command {
        Command::Workspace(cmd) => workspace::run(&ctx, cmd),
        Command::Api(args) => api::run(&ctx, args),
        Command::Issue(cmd) => issue::run(&ctx, cmd),
        Command::Project(cmd) => project::run(&ctx, cmd),
        Command::Milestone(cmd) => milestone::run(&ctx, cmd),
        Command::Initiative(cmd) => initiative::run(&ctx, cmd),
        Command::Template(cmd) => template::run(&ctx, cmd),
        Command::Label(cmd) => label::run(&ctx, cmd),
        Command::Team(cmd) => team::run(&ctx, cmd),
        Command::User(cmd) => user::run(&ctx, cmd),
        Command::Cycle(args) => cycle::run(&ctx, args),
        Command::Audit(args) => audit::run(&ctx, args),
        Command::Cache(cmd) => cache::run(&ctx, cmd),
        Command::Status(args) => status::run(&ctx, args),
        Command::Brief(args) => brief::run(&ctx, args),
        Command::Completions(_) => unreachable!("handled before the context is built"),
    }
}

impl Ctx {
    /// Pick the workspace: `--workspace` > `LINEAR_WORKSPACE` > `.linear.toml` > default.
    pub fn resolve<'a>(&self, config: &'a Config) -> Result<Resolved<'a>> {
        let env = std::env::var("LINEAR_WORKSPACE").ok();
        let cwd = std::env::current_dir()?;
        let repo = store::find_repo_file(&cwd);
        Ok(config::resolve(
            Selectors {
                flag: self.workspace_flag.as_deref(),
                env: env.as_deref(),
                repo_file: repo.as_deref(),
            },
            config,
        )?)
    }

    /// Load the credential for a workspace, or fail with an auth error that says what to do.
    pub fn credential(
        &self,
        name: &str,
    ) -> Result<(linear_core::auth::Credential, CredentialSource)> {
        // A workspace that authenticates as an app has no stored credential: its
        // client id and secret come from the environment.
        let config = store::read_config(&self.dirs)?;
        if let Some(ws) = config
            .get(name)
            .filter(|w| w.auth == AuthMethod::ClientCredentials)
        {
            return store::load_client_credentials(name, ws)?.ok_or_else(|| {
                CliError::auth(format!(
                    "no client secret for workspace {name:?}: set {} (or {})",
                    CLIENT_SECRET_ENV,
                    client_secret_env_var(name)
                ))
            });
        }
        store::load_credential(&self.dirs, name)?.ok_or_else(|| {
            CliError::auth(format!(
                "no credentials for workspace {name:?}; run `linear workspace login {name}`"
            ))
        })
    }
}

/// Ask Linear who the credential belongs to and check it is the expected workspace.
pub fn verify(client: &Client, name: &str, expected_url_key: &str) -> Result<Whoami> {
    let who: Whoami = client.execute(&queries::whoami())?;
    if who.organization.url_key != expected_url_key {
        return Err(CliError::auth(format!(
            "these credentials belong to workspace {:?}, but {name:?} is configured for {expected_url_key:?}",
            who.organization.url_key
        )));
    }
    Ok(who)
}
