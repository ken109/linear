//! Command implementations.

mod api;
mod format;
pub mod issue;
mod listing;
pub mod project;
mod workspace;

use crate::cli::{Cli, Command};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::Output;
use crate::store::{self, CredentialSource, Dirs};
use linear_core::config::{self, Config, Resolved, Selectors};
use linear_core::queries::{self, Whoami};

/// What every command needs.
pub struct Ctx {
    pub dirs: Dirs,
    pub out: Output,
    pub workspace_flag: Option<String>,
}

pub fn run(cli: &Cli, out: Output) -> Result<()> {
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
