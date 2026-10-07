//! `linear workspace list|add|login|whoami`.

use super::{verify, Ctx};
use crate::cli::{AddArgs, AuthArg, LoginArgs, MigrateArgs, StoreArg, WorkspaceCommand};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::oauth;
use crate::output::table;
use crate::store;
use linear_core::auth::{api_key_env_var, AuthMethod, Credential, Secret};
use linear_core::config::{
    validate_workspace_name, Config, CredentialStore, Ownership, WorkspaceConfig,
};
use serde::Serialize;
use std::io::{IsTerminal, Read};

pub fn run(ctx: &Ctx, cmd: &WorkspaceCommand) -> Result<()> {
    match cmd {
        WorkspaceCommand::List => list(ctx),
        WorkspaceCommand::Add(args) => add(ctx, args),
        WorkspaceCommand::Login(args) => login(ctx, args),
        WorkspaceCommand::Migrate(args) => migrate(ctx, args),
        WorkspaceCommand::Whoami => whoami(ctx),
    }
}

// ------------------------------------------------------------------ list

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRow {
    name: String,
    url_key: String,
    default_team: Option<String>,
    auth: AuthMethod,
    ownership: Ownership,
    default: bool,
    /// `"env"`, `"file"`, `"keyring"` or null. Never the credential itself.
    credentials: Option<&'static str>,
}

fn list(ctx: &Ctx) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let rows: Vec<WorkspaceRow> = config
        .workspaces
        .iter()
        .map(|(name, ws)| WorkspaceRow {
            name: name.clone(),
            url_key: ws.url_key.clone(),
            default_team: ws.default_team.clone(),
            auth: ws.auth,
            ownership: ws.ownership,
            default: config.default.as_deref() == Some(name),
            credentials: credential_kind(ctx, name, ws),
        })
        .collect();

    ctx.out.emit(
        &rows,
        || {
            if rows.is_empty() {
                return "No workspaces configured. Add one with `linear workspace add <name> --url-key <key>`."
                    .to_owned();
            }
            let body: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        format!("{}{}", r.name, if r.default { " *" } else { "" }),
                        r.url_key.clone(),
                        r.default_team.clone().unwrap_or_else(|| "-".into()),
                        r.auth.to_string(),
                        r.ownership.to_string(),
                        r.credentials
                            .map_or(
                                if r.auth == AuthMethod::ClientCredentials {
                                    "no client secret"
                                } else {
                                    "not logged in"
                                },
                                |k| k,
                            )
                            .to_owned(),
                    ]
                })
                .collect();
            table(
                &["NAME", "URL KEY", "TEAM", "AUTH", "OWNERSHIP", "CREDENTIALS"],
                &body,
            )
        },
        || rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join("\n"),
    );
    Ok(())
}

/// Whether credentials exist, and where. The credential itself is not looked at: the
/// keyring is only asked whether an entry exists, and only for a workspace set to use it.
fn credential_kind(ctx: &Ctx, name: &str, ws: &WorkspaceConfig) -> Option<&'static str> {
    // An app has no stored credential: its secret comes from the environment.
    if ws.auth == AuthMethod::ClientCredentials {
        return store::client_secret_is_set(name).then_some("env");
    }
    if std::env::var(api_key_env_var(name)).is_ok_and(|v| !v.trim().is_empty()) {
        return Some("env");
    }
    let in_keyring = store::credential_store(Some(ws)).ok()? == CredentialStore::Keyring
        && matches!(ctx.keyring.get(name), Ok(Some(_)));
    if in_keyring {
        return Some("keyring");
    }
    let path = ctx.dirs.credentials_file(name).ok()?;
    path.is_file().then_some("file")
}

// ------------------------------------------------------------------ add

fn add(ctx: &Ctx, args: &AddArgs) -> Result<()> {
    validate_workspace_name(&args.name)?;
    if args.url_key.trim().is_empty() {
        return Err(CliError::usage("--url-key must not be empty"));
    }
    if let Some(id) = &args.client_id {
        if matches!(args.auth, AuthArg::ApiKey) {
            return Err(CliError::usage(
                "--client-id is the OAuth app's client_id and needs --auth oauth or client-credentials",
            ));
        }
        if id.trim().is_empty() {
            return Err(CliError::usage("--client-id must not be empty"));
        }
    }
    if args.oauth_port.is_some() && !matches!(args.auth, AuthArg::Oauth) {
        return Err(CliError::usage("--oauth-port needs --auth oauth"));
    }

    let path = ctx.dirs.workspaces_file();
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(CliError::general(format!(
                "cannot read {}: {e}",
                path.display()
            )))
        }
    };
    let config = Config::parse(&existing)?;
    if config.get(&args.name).is_some() {
        return Err(CliError::usage(format!(
            "workspace {:?} already exists; edit {} to change it",
            args.name,
            path.display()
        )));
    }

    // Edit the document in place so comments and ordering survive.
    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .map_err(|e| CliError::general(format!("{}: {e}", path.display())))?;
    let make_default = args.default || config.default.is_none();
    if make_default {
        doc["default"] = toml_edit::value(args.name.as_str());
    }
    let workspaces = doc
        .entry("workspaces")
        .or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| CliError::general("`workspaces` in workspaces.toml is not a table"))?;
    let mut ws = toml_edit::Table::new();
    ws["url_key"] = toml_edit::value(args.url_key.trim());
    if let Some(team) = &args.team {
        ws["default_team"] = toml_edit::value(team.as_str());
    }
    ws["auth"] = toml_edit::value(match args.auth {
        AuthArg::ApiKey => "api-key",
        AuthArg::Oauth => "oauth",
        AuthArg::ClientCredentials => "client_credentials",
    });
    if let Some(id) = &args.client_id {
        ws["client_id"] = toml_edit::value(id.trim());
    }
    if let Some(port) = args.oauth_port {
        ws["oauth_port"] = toml_edit::value(i64::from(port));
    }
    workspaces.insert(&args.name, toml_edit::Item::Table(ws));

    let text = doc.to_string();
    Config::parse(&text)?; // never write a file we could not read back
    store::write_atomic(&path, text.as_bytes(), 0o644)?;

    ctx.out.emit(
        &serde_json::json!({ "name": args.name, "urlKey": args.url_key.trim(), "default": make_default }),
        || format!("Added workspace {}", args.name),
        || args.name.clone(),
    );
    Ok(())
}

// ------------------------------------------------------------------ login

fn login(ctx: &Ctx, args: &LoginArgs) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let (name, ws) = match &args.name {
        Some(n) => {
            let ws = config.get(n).ok_or_else(|| {
                CliError::usage(format!(
                    "unknown workspace {n:?}; add it first with `linear workspace add {n} --url-key <key>`"
                ))
            })?;
            (n.clone(), ws.clone())
        }
        None => {
            let r = ctx.resolve(&config)?;
            (r.name, r.config.clone())
        }
    };

    let oauth = args.oauth || ws.auth == AuthMethod::Oauth;
    if args.oauth && ws.auth == AuthMethod::ClientCredentials {
        return Err(CliError::usage(format!(
            "workspace {name:?} authenticates as an app (client_credentials); --oauth logs a person in"
        )));
    }
    if args.keyring && ws.auth == AuthMethod::ClientCredentials {
        return Err(CliError::usage(format!(
            "workspace {name:?} authenticates as an app (client_credentials): nothing is stored, so --keyring has no effect"
        )));
    }
    if !oauth && (args.client_id.is_some() || args.port.is_some() || args.no_browser) {
        return Err(CliError::usage(
            "--client-id, --port and --no-browser are for the OAuth login: add --oauth",
        ));
    }
    if let Some(id) = &args.client_id {
        if id.trim().is_empty() {
            return Err(CliError::usage("--client-id must not be empty"));
        }
    }

    // An app has nothing to log in to: its id and secret are read from the
    // environment on every run. Check that they work, and store nothing.
    if ws.auth == AuthMethod::ClientCredentials {
        let (credential, source) = ctx.credential(&name)?;
        let who = verify(&Client::new(credential), &name, &ws.url_key)?;
        ctx.out.status(&format!(
            "Nothing is stored: the client credentials are read from the {} on every run",
            source.describe()
        ));
        ctx.out.emit(
            &serde_json::json!({
                "workspace": name,
                "urlKey": who.organization.url_key,
                "user": { "id": who.viewer.id, "name": who.viewer.name, "email": who.viewer.email },
            }),
            || {
                format!(
                    "The client credentials work for {} (as {})",
                    who.organization.url_key, who.viewer.name
                )
            },
            || name.clone(),
        );
        return Ok(());
    }

    let mut used_client_id = None;
    let credential = if oauth {
        let client_id = args
            .client_id
            .as_deref()
            .map(|id| id.trim().to_owned())
            .or_else(|| store::client_id(&name, &ws))
            .ok_or_else(|| store::missing_client_id(&name))?;
        let port = oauth::port(args.port, ws.oauth_port)?;
        let credential = oauth::login(&client_id, port, !args.no_browser)?;
        used_client_id = Some(client_id);
        credential
    } else {
        Credential::ApiKey {
            api_key: Secret::new(read_api_key(args.with_token, &name)?),
        }
    };

    // Verify before saving, so a wrong key never lands on disk.
    let who = verify(&Client::new(credential.clone()), &name, &ws.url_key)?;
    let store = if args.keyring {
        CredentialStore::Keyring
    } else {
        store::credential_store(Some(&ws))?
    };
    let saved = store::save_credential(&ctx.dirs, ctx.keyring.as_ref(), &name, &credential, store)?;
    if let Some(why) = &saved.fallback {
        ctx.out.status(&format!(
            "note: the OS keyring is not available ({why}); stored the credentials in a file instead"
        ));
    }
    // What this login chose is remembered, so the next run (and the refresh) finds it.
    let to_keyring = args.keyring && saved.fallback.is_none();
    // The client id is public, and refreshing needs it on every run.
    let new_client_id = used_client_id
        .as_deref()
        .filter(|id| ws.client_id.as_deref() != Some(*id));
    if (args.oauth && ws.auth != AuthMethod::Oauth)
        || new_client_id.is_some()
        || (to_keyring && ws.credential_store != CredentialStore::Keyring)
    {
        update_workspace(ctx, &name, |table| {
            if args.oauth {
                table["auth"] = toml_edit::value("oauth");
            }
            if let Some(id) = new_client_id {
                table["client_id"] = toml_edit::value(id);
            }
            if to_keyring {
                table["credential_store"] = toml_edit::value("keyring");
            }
        })?;
    }

    let var = api_key_env_var(&name);
    if std::env::var(&var).is_ok_and(|v| !v.trim().is_empty()) {
        ctx.out.status(&format!(
            "note: {var} is set and takes precedence over the stored credentials"
        ));
    }
    ctx.out.status(&format!(
        "Stored credentials in {}",
        saved.source.describe()
    ));
    ctx.out.emit(
        &serde_json::json!({
            "workspace": name,
            "urlKey": who.organization.url_key,
            "user": { "id": who.viewer.id, "name": who.viewer.name, "email": who.viewer.email },
            "credentials": saved.source.kind(),
        }),
        || {
            format!(
                "Logged in to {} as {} <{}>",
                who.organization.url_key, who.viewer.name, who.viewer.email
            )
        },
        || name.clone(),
    );
    Ok(())
}

/// Change a workspace's table in `workspaces.toml`, keeping its comments and ordering.
fn update_workspace(
    ctx: &Ctx,
    name: &str,
    change: impl FnOnce(&mut toml_edit::Table),
) -> Result<()> {
    let path = ctx.dirs.workspaces_file();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CliError::general(format!("cannot read {}: {e}", path.display())))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| CliError::general(format!("{}: {e}", path.display())))?;
    let table = doc["workspaces"][name]
        .as_table_mut()
        .ok_or_else(|| CliError::general(format!("workspace {name:?} is not a table")))?;
    change(table);
    let text = doc.to_string();
    Config::parse(&text)?; // never write a file we could not read back
    store::write_atomic(&path, text.as_bytes(), 0o644)
}

/// Write `credential_store` of a workspace into `workspaces.toml`. The default (`file`) is
/// removed rather than written.
fn set_credential_store(ctx: &Ctx, name: &str, store: CredentialStore) -> Result<()> {
    update_workspace(ctx, name, |table| {
        if store.is_file() {
            table.remove("credential_store");
        } else {
            table["credential_store"] = toml_edit::value(store.to_string());
        }
    })
}

// ------------------------------------------------------------------ migrate

fn migrate(ctx: &Ctx, args: &MigrateArgs) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let name = match &args.name {
        Some(n) => {
            config.get(n).ok_or_else(|| {
                CliError::usage(format!(
                    "unknown workspace {n:?}; see `linear workspace list`"
                ))
            })?;
            n.clone()
        }
        None => ctx.resolve(&config)?.name,
    };
    let to = match args.to {
        StoreArg::File => CredentialStore::File,
        StoreArg::Keyring => CredentialStore::Keyring,
    };

    let moved = store::migrate_credential(&ctx.dirs, ctx.keyring.as_ref(), &name, to)?;
    set_credential_store(ctx, &name, to)?;
    if let Some(warning) = &moved.left_behind {
        ctx.out.status(&format!("warning: {warning}"));
    }
    ctx.out.emit(
        &serde_json::json!({
            "workspace": name,
            "from": moved.from.kind(),
            "to": moved.to.kind(),
        }),
        || {
            format!(
                "Moved the credentials of {name} from {} to {}",
                moved.from.describe(),
                moved.to.describe()
            )
        },
        || name.clone(),
    );
    Ok(())
}

fn read_api_key(with_token: bool, workspace: &str) -> Result<String> {
    let raw = if with_token {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("Linear API key for {workspace}: "))?
    } else {
        return Err(CliError::usage(
            "standard input is not a terminal; pass the key with --with-token (e.g. `printf %s \"$KEY\" | linear workspace login --with-token`)",
        ));
    };
    let key = raw.trim().to_owned();
    if key.is_empty() {
        return Err(CliError::usage("the API key is empty"));
    }
    Ok(key)
}

// ------------------------------------------------------------------ whoami

fn whoami(ctx: &Ctx) -> Result<()> {
    let config = store::read_config(&ctx.dirs)?;
    let resolved = ctx.resolve(&config)?;
    let (credential, source) = ctx.credential(&resolved.name)?;
    if credential.method() != resolved.config.auth {
        return Err(CliError::auth(format!(
            "workspace {:?} is configured for {} but the stored credentials are {}",
            resolved.name,
            resolved.config.auth,
            credential.method()
        )));
    }
    let who = verify(
        &Client::new(credential),
        &resolved.name,
        &resolved.config.url_key,
    )?;

    let source_kind = source.kind();
    ctx.out.emit(
        &serde_json::json!({
            "workspace": resolved.name,
            "organization": who.organization,
            "user": { "id": who.viewer.id, "name": who.viewer.name, "displayName": who.viewer.display_name, "email": who.viewer.email },
            "auth": resolved.config.auth,
            "credentials": source_kind,
        }),
        || {
            format!(
                "Workspace:  {} ({})\nUser:       {} <{}>\nAuth:       {} ({})",
                resolved.name,
                who.organization.url_key,
                who.viewer.name,
                who.viewer.email,
                resolved.config.auth,
                source.describe()
            )
        },
        || who.viewer.name.clone(),
    );
    Ok(())
}
