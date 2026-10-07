//! Command implementations.

mod api;
pub mod audit;
pub mod brief;
pub mod cache;
pub mod cached;
pub mod comment;
pub mod completions;
pub mod cycle;
pub mod document;
pub mod file;
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
pub mod webhook;
mod workspace;
pub mod write;

use crate::cli::{Cli, Command};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::keystore::{self, Keyring};
use crate::oauth;
use crate::output::Output;
use crate::store::{self, CredentialSource, Dirs};
use chrono::Utc;
use linear_core::auth::{client_secret_env_var, AuthMethod, Credential, CLIENT_SECRET_ENV};
use linear_core::config::{self, Config, CredentialStore, Resolved, Selectors, WorkspaceConfig};
use linear_core::queries::{self, Whoami};
use linear_core::ErrorCode;

/// What every command needs.
pub struct Ctx {
    pub dirs: Dirs,
    pub out: Output,
    pub workspace_flag: Option<String>,
    /// Where credentials set to the OS keyring are kept.
    pub keyring: Box<dyn Keyring>,
}

pub fn run(cli: &Cli, out: Output) -> Result<()> {
    // These touch neither the configuration nor Linear, so they must work on a
    // machine that has no config directory and no network.
    match &cli.command {
        Command::Completions(args) => return completions::run(args),
        Command::Webhook(webhook::WebhookCommand::Verify(args)) => {
            return webhook::verify(out, args)
        }
        _ => {}
    }
    crate::http::configure(crate::http::Settings::resolve(cli.timeout)?);
    let ctx = Ctx::new(Dirs::from_env()?, out, cli.workspace.clone());
    match &cli.command {
        Command::Workspace(cmd) => workspace::run(&ctx, cmd),
        Command::Api(args) => api::run(&ctx, args),
        Command::Issue(cmd) => issue::run(&ctx, cmd),
        Command::Comment(cmd) => comment::run(&ctx, cmd),
        Command::Project(cmd) => project::run(&ctx, cmd),
        Command::Milestone(cmd) => milestone::run(&ctx, cmd),
        Command::Initiative(cmd) => initiative::run(&ctx, cmd),
        Command::File(cmd) => file::run(&ctx, cmd),
        Command::Template(cmd) => template::run(&ctx, cmd),
        Command::Label(cmd) => label::run(&ctx, cmd),
        Command::Team(cmd) => team::run(&ctx, cmd),
        Command::User(cmd) => user::run(&ctx, cmd),
        Command::Cycle(args) => cycle::run(&ctx, args),
        Command::Document(cmd) => document::run(&ctx, cmd),
        Command::Audit(args) => audit::run(&ctx, args),
        Command::Cache(cmd) => cache::run(&ctx, cmd),
        Command::Status(args) => status::run(&ctx, args),
        Command::Brief(args) => brief::run(&ctx, args),
        Command::Webhook(cmd) => webhook::run(&ctx, cmd),
        Command::Completions(_) => unreachable!("handled before the context is built"),
    }
}

impl Ctx {
    pub fn new(dirs: Dirs, out: Output, workspace_flag: Option<String>) -> Self {
        Self {
            dirs,
            out,
            workspace_flag,
            keyring: keystore::from_env(),
        }
    }

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
    pub fn credential(&self, name: &str) -> Result<(Credential, CredentialSource)> {
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
        let store = store::credential_store(config.get(name))?;
        let load = if config
            .get(name)
            .is_some_and(|w| w.auth == AuthMethod::Oauth)
        {
            store::load_stored_credential
        } else {
            store::load_credential
        };
        let loaded = load(&self.dirs, self.keyring.as_ref(), name, store)?.ok_or_else(|| {
            CliError::auth(format!(
                "no credentials for workspace {name:?}; run `linear workspace login {name}`"
            ))
        })?;
        self.renewed(name, config.get(name), store, loaded)
    }

    /// An OAuth access token that has run out, or is about to, is replaced with the refresh
    /// token and the new one stored where the old one was. Anything else passes through.
    fn renewed(
        &self,
        name: &str,
        ws: Option<&WorkspaceConfig>,
        store: CredentialStore,
        (credential, source): (Credential, CredentialSource),
    ) -> Result<(Credential, CredentialSource)> {
        let now = Utc::now();
        if !credential.needs_refresh(now) {
            return Ok((credential, source));
        }
        let Credential::Oauth {
            refresh_token: Some(_),
            ..
        } = &credential
        else {
            return Err(oauth::expired(name, "there is no refresh token"));
        };
        let client_id = ws
            .and_then(|w| store::client_id(name, w))
            .ok_or_else(|| store::missing_client_id(name))?;
        match oauth::refresh(&client_id, &credential) {
            Ok(new) => {
                // The new refresh token replaces the old one, so losing it would end the login.
                if let Err(e) = store::save_credential_to(
                    &self.dirs,
                    self.keyring.as_ref(),
                    name,
                    &new,
                    &source,
                ) {
                    self.out.status(&format!(
                        "warning: refreshed the OAuth token but could not store it: {}",
                        e.message
                    ));
                }
                Ok((new, source))
            }
            Err(e) if e.code == ErrorCode::Auth => {
                // Another run may have refreshed it first, and the refresh token was used up.
                let other = store::load_credential(&self.dirs, self.keyring.as_ref(), name, store)?;
                match other {
                    Some((c, s)) if c != credential && !c.needs_refresh(now) => Ok((c, s)),
                    _ => Err(oauth::expired(name, &e.message)),
                }
            }
            // Could not ask (offline, a 5xx): the token still in date is used as it is.
            Err(_) if !credential.is_expired(now) => Ok((credential, source)),
            Err(e) => Err(e),
        }
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
