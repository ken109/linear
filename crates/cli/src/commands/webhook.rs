//! `linear webhook list|create|delete|verify`.
//!
//! A webhook belongs to the workspace (or to a team), not to a project or an
//! issue, so no ownership rule applies and neither does a validator; Linear
//! itself decides who may manage webhooks. `verify` is offline: it checks a
//! delivery's signature with the logic the WebAssembly package shares.

use super::format::{date_time, fields, opt_text};
use super::listing::{paginate, warn_truncated, ListArgs};
use super::write::dry_run::{Plan, Target};
use super::write::{read_text, resolve};
use super::Ctx;
use crate::error::{CliError, Result};
use crate::output::{table, Output};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use linear_core::inputs::{self, WebhookCreate, WebhookCreateInput, WebhookDelete};
use linear_core::matching::match_webhook;
use linear_core::read::{self, Webhooks, WEBHOOKS_PAGE_SIZE};
use linear_core::types::Webhook;
use linear_core::webhook::{self as signing, Rejection, Verification};
use linear_core::InWorkspace;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The environment variable `verify` reads the signing secret from.
const SECRET_ENV: &str = "LINEAR_WEBHOOK_SECRET";

#[derive(Debug, Subcommand)]
pub enum WebhookCommand {
    /// Show how to use these commands, briefly, for an AI agent
    Usage,
    /// List the webhooks of the workspace (never their signing secrets)
    List(ListCmd),
    /// Create a webhook; prints its signing secret, which Linear generates
    Create(CreateCmd),
    /// Delete a webhook
    Delete(DeleteCmd),
    /// Check the signature and timestamp of a delivery you received (offline)
    ///
    /// Reads the exact request body from --body-file or standard input, and the
    /// signing secret from --secret-file or LINEAR_WEBHOOK_SECRET (never from an
    /// argument, which other users of the machine can see). Exits 0 for a valid
    /// delivery. A rejected one exits 1 and says why (`--json` prints the same
    /// `{"status","reason"|"event"}` object as the WebAssembly package on standard
    /// output); a missing or empty secret exits 2.
    Verify(VerifyCmd),
}

#[derive(Debug, Args)]
pub struct ListCmd {
    #[command(flatten)]
    pub page: ListArgs,
}

#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("scope").required(true).multiple(false))]
pub struct CreateCmd {
    /// The URL Linear sends deliveries to (HTTP POST)
    #[arg(long, value_name = "URL")]
    pub url: String,
    /// What to deliver (repeatable or comma-separated): Issue, Comment, Project, ...
    #[arg(long, value_name = "TYPE", value_delimiter = ',', required = true)]
    pub resource_types: Vec<String>,
    /// Deliver the events of this team (key, name or id)
    #[arg(long, value_name = "TEAM", group = "scope")]
    pub team: Option<String>,
    /// Deliver the events of every public team, including teams created later
    #[arg(long, group = "scope")]
    pub all_public_teams: bool,
    /// A name for the webhook, shown in Linear's settings
    #[arg(long, value_name = "LABEL")]
    pub label: Option<String>,
}

#[derive(Debug, Args)]
pub struct DeleteCmd {
    /// Webhook id, label or URL
    pub webhook: String,
}

#[derive(Debug, Args)]
pub struct VerifyCmd {
    /// The delivery's `Linear-Signature` header (hex)
    #[arg(long, value_name = "HEX")]
    pub signature: String,
    /// Read the exact request body from a file (default: standard input). Not a
    /// re-serialization: one changed byte is a different signature
    #[arg(long, value_name = "FILE")]
    pub body_file: Option<PathBuf>,
    /// Read the signing secret from a file (default: the LINEAR_WEBHOOK_SECRET
    /// environment variable)
    #[arg(long, value_name = "FILE")]
    pub secret_file: Option<PathBuf>,
    /// Judge the delivery's timestamp against this time (epoch milliseconds)
    /// instead of now, to check a saved delivery
    #[arg(long, value_name = "MS", allow_negative_numbers = true)]
    pub at: Option<i64>,
}

pub fn run(ctx: &Ctx, cmd: &WebhookCommand) -> Result<()> {
    match cmd {
        WebhookCommand::Usage => unreachable!("handled before the context is built"),
        WebhookCommand::List(args) => list(ctx, args),
        WebhookCommand::Create(args) => create(ctx, args),
        WebhookCommand::Delete(args) => delete(ctx, args),
        WebhookCommand::Verify(args) => verify(ctx.out, args),
    }
}

// ---------------------------------------------------------------- list

fn scope(w: &Webhook) -> String {
    match (&w.team, w.all_public_teams) {
        (Some(team), _) => team.key.clone(),
        (None, true) => "all public teams".to_owned(),
        (None, false) => "workspace".to_owned(),
    }
}

fn name_of(w: &Webhook) -> String {
    opt_text(w.label.as_deref().or(w.url.as_deref()))
}

fn list(ctx: &Ctx, args: &ListCmd) -> Result<()> {
    let session = ctx.session()?;
    let listing = paginate(WEBHOOKS_PAGE_SIZE, args.page.limit(), |page| {
        let data: Webhooks = session.client.execute(&read::webhooks(page))?;
        Ok(data.webhooks)
    })?;
    if listing.truncated {
        warn_truncated(listing.items.len());
    }
    let tagged = InWorkspace::tag_all(&session.workspace, listing.items.clone());
    ctx.out.emit_selectable(
        &tagged,
        || {
            if listing.items.is_empty() {
                return "No webhooks found.".to_owned();
            }
            let body: Vec<Vec<String>> = listing
                .items
                .iter()
                .map(|w| {
                    vec![
                        w.id.inner().to_owned(),
                        opt_text(w.label.as_deref()),
                        opt_text(w.url.as_deref()),
                        scope(w),
                        w.resource_types.join(","),
                        if w.enabled { "yes" } else { "no" }.to_owned(),
                    ]
                })
                .collect();
            table(
                &["ID", "LABEL", "URL", "SCOPE", "RESOURCES", "ENABLED"],
                &body,
            )
        },
        || {
            listing
                .items
                .iter()
                .map(|w| w.id.inner())
                .collect::<Vec<_>>()
                .join("\n")
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------- create

/// What `create` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created<'a> {
    workspace: &'a str,
    #[serde(flatten)]
    webhook: &'a Webhook,
    /// What `verify` needs; Linear generates it. Only this command prints it.
    secret: Option<&'a str>,
}

fn create(ctx: &Ctx, cmd: &CreateCmd) -> Result<()> {
    let url = cmd.url.trim();
    if url.is_empty() {
        return Err(CliError::usage("--url must not be empty"));
    }
    let resource_types: Vec<String> = cmd
        .resource_types
        .iter()
        .map(|t| t.trim().to_owned())
        .collect();
    if resource_types.iter().any(String::is_empty) {
        return Err(CliError::usage(
            "--resource-types must not contain an empty name",
        ));
    }
    let label = cmd
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned);

    let ws = ctx.write_session()?;
    // An unknown team is a usage error here, before anything is created.
    let team_id = match cmd.team.as_deref() {
        Some(reference) => Some(resolve::team(&ws, Some(reference))?.id.into_inner()),
        None => None,
    };
    let op = inputs::webhook_create(WebhookCreateInput {
        url: url.to_owned(),
        resource_types,
        label,
        team_id,
        all_public_teams: cmd.all_public_teams.then_some(true),
    });
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new("webhook create", Target::new("webhook", url)));
    }
    let data: WebhookCreate = ws.client.execute(&op)?;
    let payload = data.webhook_create;
    if !payload.success {
        return Err(CliError::general("Linear could not create the webhook"));
    }
    let webhook = &payload.webhook;
    let secret = payload.signing.secret.as_deref();
    ws.out.status(
        "The signing secret is printed only by `webhook create`; keep it to check deliveries \
         with `webhook verify`.",
    );
    ctx.out.emit(
        &Created {
            workspace: &ws.workspace,
            webhook,
            secret,
        },
        || {
            fields(&[
                ("Id", webhook.id.inner().to_owned()),
                ("Label", opt_text(webhook.label.as_deref())),
                ("URL", opt_text(webhook.url.as_deref())),
                ("Scope", scope(webhook)),
                ("Resources", webhook.resource_types.join(", ")),
                ("Created", date_time(&webhook.created_at)),
                ("Secret", opt_text(secret)),
            ])
        },
        || webhook.id.inner().to_owned(),
    );
    Ok(())
}

// ---------------------------------------------------------------- delete

/// What `delete` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Deleted<'a> {
    workspace: &'a str,
    id: &'a str,
    label: Option<&'a str>,
    url: Option<&'a str>,
    deleted: bool,
}

fn delete(ctx: &Ctx, cmd: &DeleteCmd) -> Result<()> {
    let ws = ctx.write_session()?;
    // Resolved against the list, so a typo or an ambiguous URL deletes nothing.
    let all = paginate(WEBHOOKS_PAGE_SIZE, None, |page| {
        let data: Webhooks = ws.client.execute(&read::webhooks(page))?;
        Ok(data.webhooks)
    })?
    .items;
    let found = match_webhook(&all, &cmd.webhook)?;

    let op = inputs::webhook_delete(found.id.inner());
    if ws.dry_run {
        ws.record(&op);
        return ws.finish_dry_run(Plan::new(
            "webhook delete",
            Target::existing("webhook", name_of(found), found.id.inner()),
        ));
    }
    let data: WebhookDelete = ws.client.execute(&op)?;
    if !data.webhook_delete.success {
        return Err(CliError::general("Linear could not delete the webhook"));
    }
    ctx.out.emit(
        &Deleted {
            workspace: &ws.workspace,
            id: found.id.inner(),
            label: found.label.as_deref(),
            url: found.url.as_deref(),
            deleted: true,
        },
        || format!("{}  (deleted)", name_of(found)),
        || found.id.inner().to_owned(),
    );
    Ok(())
}

// ---------------------------------------------------------------- verify

/// Why a delivery was rejected, in words.
fn explain(reason: Rejection) -> &'static str {
    match reason {
        Rejection::MalformedSignature => "the signature is not 64 hex digits",
        Rejection::SignatureMismatch => {
            "the signature does not match this body and secret (a changed body, or another secret)"
        }
        Rejection::MalformedBody => "the signature is right but the body is not a webhook payload",
        Rejection::MissingTimestamp => {
            "the signature is right but the body has no webhookTimestamp"
        }
        Rejection::StaleTimestamp => {
            "the signature is right but the timestamp is more than a minute from now \
             (pass --at to judge a saved delivery against another time)"
        }
    }
}

/// The signing secret: from a file, else from the environment.
fn secret(file: Option<&Path>) -> Result<String> {
    let text = match file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))?,
        None => std::env::var(SECRET_ENV).map_err(|_| {
            CliError::usage(format!(
                "no signing secret: set {SECRET_ENV} or pass --secret-file"
            ))
        })?,
    };
    // A secret file normally ends with a newline that is not part of the secret.
    Ok(text.trim().to_owned())
}

pub fn verify(out: Output, cmd: &VerifyCmd) -> Result<()> {
    let secret = secret(cmd.secret_file.as_deref())?;
    let body = read_text(cmd.body_file.as_deref().unwrap_or(Path::new("-")))?;
    let now: DateTime<Utc> = match cmd.at {
        Some(ms) => DateTime::from_timestamp_millis(ms)
            .ok_or_else(|| CliError::usage("--at is out of range"))?,
        None => Utc::now(),
    };

    let verdict = signing::verify_webhook(&body, &cmd.signature, &secret, now)?;
    out.emit(
        &verdict,
        || match &verdict {
            Verification::Valid { event } => format!(
                "valid  {} {}  (sent {})",
                event.action,
                event.resource_type,
                DateTime::from_timestamp_millis(event.webhook_timestamp)
                    .map_or_else(|| event.webhook_timestamp.to_string(), |t| t.to_rfc3339())
            ),
            Verification::Invalid { reason } => format!("invalid  {}", explain(*reason)),
        },
        || match &verdict {
            Verification::Valid { .. } => "valid".to_owned(),
            Verification::Invalid { .. } => "invalid".to_owned(),
        },
    );
    match verdict {
        Verification::Valid { .. } => Ok(()),
        Verification::Invalid { reason } => Err(CliError::general(format!(
            "the delivery is not valid: {}",
            explain(reason)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn every_rejection_has_an_explanation() {
        for reason in [
            Rejection::MalformedSignature,
            Rejection::SignatureMismatch,
            Rejection::MalformedBody,
            Rejection::MissingTimestamp,
            Rejection::StaleTimestamp,
        ] {
            assert!(!explain(reason).is_empty());
        }
    }

    #[test]
    fn create_needs_exactly_one_scope() {
        #[derive(clap::Parser)]
        struct Wrap {
            #[command(flatten)]
            cmd: CreateCmd,
        }
        Wrap::command().debug_assert();
        let ok = ["x", "--url", "https://e.test", "--resource-types", "Issue"];
        let parse = |extra: &[&str]| {
            Wrap::command().try_get_matches_from(ok.iter().chain(extra.iter()).copied())
        };
        assert!(parse(&[]).is_err(), "no scope");
        assert!(parse(&["--team", "ENG"]).is_ok());
        assert!(parse(&["--all-public-teams"]).is_ok());
        assert!(parse(&["--team", "ENG", "--all-public-teams"]).is_err());
    }
}
