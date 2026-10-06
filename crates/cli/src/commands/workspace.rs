//! `linear workspace list|add|login|whoami`.

use super::Ctx;
use crate::cli::WorkspaceCommand;
use crate::error::Result;
use crate::output::table;
use crate::store;
use linear_core::auth::{api_key_env_var, AuthMethod};
use serde::Serialize;

pub fn run(ctx: &Ctx, cmd: &WorkspaceCommand) -> Result<()> {
    match cmd {
        WorkspaceCommand::List => list(ctx),
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
