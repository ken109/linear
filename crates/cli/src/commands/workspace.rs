//! `linear workspace list|add|login|whoami`.

use super::{verify, Ctx};
use crate::cli::{AddArgs, AuthArg, LoginArgs, WorkspaceCommand};
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::table;
use crate::store::{self, CredentialSource};
use linear_core::auth::{api_key_env_var, AuthMethod, Credential, Secret};
use linear_core::config::{validate_workspace_name, Config};
use serde::Serialize;
use std::io::{IsTerminal, Read};

pub fn run(ctx: &Ctx, cmd: &WorkspaceCommand) -> Result<()> {
    match cmd {
        WorkspaceCommand::List => list(ctx),
        WorkspaceCommand::Add(args) => add(ctx, args),
        WorkspaceCommand::Login(args) => login(ctx, args),
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
    default: bool,
    /// `"env"`, `"file"` or null. Never the credential itself.
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
            default: config.default.as_deref() == Some(name),
            credentials: credential_kind(ctx, name),
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
                        r.credentials.map_or("not logged in", |k| k).to_owned(),
                    ]
                })
                .collect();
            table(&["NAME", "URL KEY", "TEAM", "AUTH", "CREDENTIALS"], &body)
        },
        || rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join("\n"),
    );
    Ok(())
}

/// Whether credentials exist, and where, without reading them.
fn credential_kind(ctx: &Ctx, name: &str) -> Option<&'static str> {
    if std::env::var(api_key_env_var(name)).is_ok_and(|v| !v.trim().is_empty()) {
        return Some("env");
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
    });
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

    if ws.auth == AuthMethod::Oauth {
        return Err(CliError::general(
            "OAuth login is not implemented yet; configure the workspace with --auth api-key",
        ));
    }

    let key = read_api_key(args.with_token, &name)?;
    let credential = Credential::ApiKey {
        api_key: Secret::new(key),
    };

    // Verify before saving, so a wrong key never lands on disk.
    let who = verify(&Client::new(credential.clone()), &name, &ws.url_key)?;
    let path = store::save_credential(&ctx.dirs, &name, &credential)?;

    let var = api_key_env_var(&name);
    if std::env::var(&var).is_ok_and(|v| !v.trim().is_empty()) {
        ctx.out.status(&format!(
            "note: {var} is set and takes precedence over the stored credentials"
        ));
    }
    ctx.out
        .status(&format!("Stored credentials in {}", path.display()));
    ctx.out.emit(
        &serde_json::json!({
            "workspace": name,
            "urlKey": who.organization.url_key,
            "user": { "id": who.viewer.id, "name": who.viewer.name, "email": who.viewer.email },
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

    let source_kind = match &source {
        CredentialSource::Env(_) => "env",
        CredentialSource::File(_) => "file",
    };
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
