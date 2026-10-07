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
pub mod usage;
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
    /// `--dry-run`: a write records its mutations and prints them instead of sending.
    pub dry_run: bool,
    /// Where credentials set to the OS keyring are kept.
    pub keyring: Box<dyn Keyring>,
}

/// The commands whose `--json` output `--fields` and `--id-only` can cut down: the list
/// and view commands that print through `Output::emit_selectable`.
pub const SELECTABLE: &str = "issue list|search|view, project list|view, milestone list|view, \
initiative list|view|status-updates, label list|view, team list|view, user list|view, \
document list|view, template list|view, cycle (by date)|list|view, webhook list";

/// Whether `command` prints through `Output::emit_selectable`.
pub fn is_selectable(command: &Command) -> bool {
    use cycle::CycleCommand;
    use initiative::InitiativeCommand as Init;
    use issue::IssueCommand as Issue;
    use template::TemplateCommand as Tpl;
    match command {
        Command::Issue(c) => matches!(c, Issue::List(_) | Issue::Search(_) | Issue::View(_)),
        Command::Project(c) => matches!(
            c,
            project::ProjectCommand::List(_) | project::ProjectCommand::View(_)
        ),
        Command::Milestone(c) => matches!(
            c,
            milestone::MilestoneCommand::List(_) | milestone::MilestoneCommand::View(_)
        ),
        Command::Initiative(c) => {
            matches!(c, Init::List(_) | Init::View(_) | Init::StatusUpdates(_))
        }
        Command::Label(c) => matches!(
            c,
            label::LabelCommand::List(_) | label::LabelCommand::View(_)
        ),
        Command::Team(c) => matches!(c, team::TeamCommand::List(_) | team::TeamCommand::View(_)),
        Command::User(c) => matches!(c, user::UserCommand::List(_) | user::UserCommand::View(_)),
        Command::Document(c) => matches!(
            c,
            document::DocumentCommand::List(_) | document::DocumentCommand::View(_)
        ),
        Command::Template(c) => matches!(c, Tpl::List(_) | Tpl::View(_)),
        Command::Cycle(args) => matches!(
            args.command,
            None | Some(CycleCommand::List(_)) | Some(CycleCommand::View(_))
        ),
        Command::Webhook(c) => matches!(c, webhook::WebhookCommand::List(_)),
        _ => false,
    }
}

/// Refuse `--fields` and `--id-only` where they would be silently ignored (every write, for
/// one), and the flag combinations that make no sense. Before anything is sent.
fn check_selection(cli: &Cli) -> Result<()> {
    if cli.fields.is_empty() && !cli.id_only {
        return Ok(());
    }
    crate::output::Selection::check_flags(&cli.fields, cli.id_only, cli.json, cli.quiet)?;
    if !is_selectable(&cli.command) {
        let flag = if cli.id_only { "--id-only" } else { "--fields" };
        return Err(CliError::usage(format!(
            "{flag} does not apply to this command; it cuts down the output of: {SELECTABLE}"
        )));
    }
    Ok(())
}

pub fn run(cli: &Cli, out: Output) -> Result<()> {
    check_selection(cli)?;
    if cli.dry_run {
        if let Some(command) = no_dry_run(&cli.command) {
            return Err(CliError::usage(format!(
                "--dry-run is for the commands that write to Linear; `{command}` does not, \
                 or changes only this machine, so it is refused rather than run"
            )));
        }
    }
    // These touch neither the configuration nor Linear, so they must work on a
    // machine that has no config directory and no network.
    if let Some(group) = usage::requested(&cli.command) {
        return usage::run(out, group);
    }
    match &cli.command {
        Command::Completions(args) => return completions::run(args),
        Command::Webhook(webhook::WebhookCommand::Verify(args)) => {
            return webhook::verify(out, args)
        }
        Command::Issue(issue::IssueCommand::Batch(args)) if args.schema => {
            write::batch::print_schema(out);
            return Ok(());
        }
        _ => {}
    }
    crate::http::configure(crate::http::Settings::resolve(cli.timeout)?);
    let mut ctx = Ctx::new(Dirs::from_env()?, out, cli.workspace.clone());
    ctx.dry_run = cli.dry_run;
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
        Command::Completions(_) | Command::Usage => {
            unreachable!("handled before the context is built")
        }
    }
}

/// The command (as typed) when it does not take `--dry-run`: everything that does not write
/// to Linear. Exhaustive on purpose, so a new command has to say which it is.
fn no_dry_run(command: &Command) -> Option<&'static str> {
    use crate::cli::WorkspaceCommand as Workspace;
    use crate::commands::{
        cache::CacheCommand as Cache, comment::CommentCommand as Comment,
        document::DocumentCommand as Doc, file::FileCommand as File,
        initiative::InitiativeCommand as Init, issue::IssueCommand as Issue,
        label::LabelCommand as Label, milestone::MilestoneCommand as Milestone,
        project::ProjectCommand as Project, template::TemplateCommand as Template,
        webhook::WebhookCommand as Webhook,
    };
    match command {
        Command::Usage => Some("usage"),
        Command::Workspace(Workspace::Usage) => Some("workspace usage"),
        Command::Workspace(Workspace::List) => Some("workspace list"),
        Command::Workspace(Workspace::Add(_)) => Some("workspace add"),
        Command::Workspace(Workspace::Login(_)) => Some("workspace login"),
        Command::Workspace(Workspace::Migrate(_)) => Some("workspace migrate"),
        Command::Workspace(Workspace::Whoami) => Some("workspace whoami"),
        Command::Issue(cmd) => match cmd {
            Issue::Usage => Some("issue usage"),
            Issue::List(_) => Some("issue list"),
            Issue::Search(_) => Some("issue search"),
            Issue::View(_) => Some("issue view"),
            Issue::Create(_)
            | Issue::Update(_)
            | Issue::Comment(_)
            | Issue::LinkPr(_)
            | Issue::AttachFile(_)
            | Issue::Unlink(_)
            | Issue::Delete(_)
            | Issue::Archive(_)
            | Issue::Unarchive(_)
            | Issue::Relate(_)
            | Issue::Unrelate(_)
            | Issue::Reorder(_)
            | Issue::Batch(_) => None,
        },
        Command::Comment(cmd) => match cmd {
            Comment::Usage => Some("comment usage"),
            Comment::Update(_) | Comment::Delete(_) => None,
        },
        Command::Project(cmd) => match cmd {
            Project::Usage => Some("project usage"),
            Project::List(_) => Some("project list"),
            Project::View(_) => Some("project view"),
            Project::Create(_)
            | Project::Update(_)
            | Project::Reorder(_)
            | Project::StatusUpdate(_)
            | Project::Delete(_)
            | Project::Unarchive(_) => None,
        },
        Command::Milestone(cmd) => match cmd {
            Milestone::Usage => Some("milestone usage"),
            Milestone::List(_) => Some("milestone list"),
            Milestone::View(_) => Some("milestone view"),
            Milestone::Create(_) | Milestone::Update(_) | Milestone::Delete(_) => None,
        },
        Command::Initiative(cmd) => match cmd {
            Init::Usage => Some("initiative usage"),
            Init::List(_) => Some("initiative list"),
            Init::View(_) => Some("initiative view"),
            Init::StatusUpdates(_) => Some("initiative status-updates"),
            Init::Create(_)
            | Init::Update(_)
            | Init::AddProject(_)
            | Init::RemoveProject(_)
            | Init::StatusUpdate(_)
            | Init::Archive(_)
            | Init::Unarchive(_)
            | Init::Delete(_) => None,
        },
        Command::File(cmd) => match cmd {
            File::Usage => Some("file usage"),
            File::Upload(_) => None,
            File::Download(_) => Some("file download"),
        },
        Command::Template(cmd) => match cmd {
            Template::Usage => Some("template usage"),
            Template::List(_) => Some("template list"),
            Template::View(_) => Some("template view"),
            Template::Skeleton(_) => Some("template skeleton"),
            Template::Create(_) => None,
        },
        Command::Label(cmd) => match cmd {
            Label::Usage => Some("label usage"),
            Label::List(_) => Some("label list"),
            Label::View(_) => Some("label view"),
            Label::Create(_) | Label::Update(_) => None,
        },
        Command::Team(_) => Some("team"),
        Command::User(_) => Some("user"),
        Command::Cycle(_) => Some("cycle"),
        Command::Document(cmd) => match cmd {
            Doc::Usage => Some("document usage"),
            Doc::List(_) => Some("document list"),
            Doc::View(_) => Some("document view"),
            Doc::Create(_) | Doc::Update(_) => None,
        },
        Command::Audit(_) => Some("audit"),
        Command::Cache(cmd) => match cmd {
            Cache::Usage => Some("cache usage"),
            Cache::Refresh => Some("cache refresh"),
            Cache::Show(_) => Some("cache show"),
            Cache::Clear => Some("cache clear"),
        },
        Command::Status(_) => Some("status"),
        Command::Brief(_) => Some("brief"),
        Command::Webhook(cmd) => match cmd {
            Webhook::Usage => Some("webhook usage"),
            Webhook::List(_) => Some("webhook list"),
            Webhook::Create(_) | Webhook::Delete(_) => None,
            Webhook::Verify(_) => Some("webhook verify"),
        },
        Command::Completions(_) => Some("completions"),
        // A query is a read; only `--mutation` writes.
        Command::Api(args) => (!args.mutation).then_some("api (without --mutation)"),
    }
}

impl Ctx {
    pub fn new(dirs: Dirs, out: Output, workspace_flag: Option<String>) -> Self {
        Self {
            dirs,
            out,
            workspace_flag,
            dry_run: false,
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
