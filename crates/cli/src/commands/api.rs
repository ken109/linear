//! `linear api`: send a raw GraphQL query and print the response data.
//!
//! This is the escape hatch for anything without a dedicated command. It
//! honours `--workspace` and the stored credentials like every other command,
//! but none of the CLI's own rules apply to what it sends (a free-form
//! document cannot be checked against ownership rules or validators), so it is
//! read-only: mutations are refused. Allowing them is a separate, opt-in
//! feature that does not exist yet.

use super::Ctx;
use crate::cli::ApiArgs;
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::store;
use linear_core::document::{operation_kinds, OperationKind};
use linear_core::wire::Request;
use linear_core::ErrorCode;
use serde_json::{Map, Value};
use std::io::Read;
use std::path::Path;

pub fn run(ctx: &Ctx, args: &ApiArgs) -> Result<()> {
    // Refuse before anything is read or sent.
    if args.mutation {
        return Err(raw_mutation_unavailable());
    }

    let stdin_users = [
        args.query.as_deref() == Some("-"),
        args.variables_file.as_deref() == Some(Path::new("-")),
    ];
    if stdin_users.iter().all(|u| *u) {
        return Err(CliError::usage(
            "the query and --variables-file cannot both be read from standard input",
        ));
    }

    let document = read_document(args)?;
    let kinds = operation_kinds(&document)?;
    if kinds.contains(&OperationKind::Subscription) {
        return Err(CliError::usage(
            "subscriptions are not supported; `linear api` sends queries over plain HTTP",
        ));
    }
    if kinds.contains(&OperationKind::Mutation) {
        return Err(CliError::new(
            ErrorCode::WriteDenied,
            "the document defines a mutation, but `linear api` only runs queries by default; \
             raw mutations are not available yet (they will need `--mutation` and \
             `allow_raw_mutation = true` in the workspace config)",
        ));
    }

    let request = Request {
        query: document,
        variables: read_variables(args)?,
        operation_name: args.operation_name.clone(),
    };

    let config = store::read_config(&ctx.dirs)?;
    let resolved = ctx.resolve(&config)?;
    let (credential, _) = ctx.credential(&resolved.name)?;
    if credential.method() != resolved.config.auth {
        return Err(CliError::auth(format!(
            "workspace {:?} is configured for {} but the stored credentials are {}",
            resolved.name,
            resolved.config.auth,
            credential.method()
        )));
    }

    let data: Value = Client::new(credential).execute_request(&request)?;
    ctx.out.emit(
        &data,
        || pretty(&data),
        || serde_json::to_string(&data).expect("a JSON value serializes"),
    );
    Ok(())
}

fn raw_mutation_unavailable() -> CliError {
    CliError::new(
        ErrorCode::WriteDenied,
        "raw mutations are not available yet; `--mutation` will require \
         `allow_raw_mutation = true` in the workspace config",
    )
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("a JSON value serializes")
}

fn read_document(args: &ApiArgs) -> Result<String> {
    match (&args.query, &args.query_file) {
        (Some(q), _) if q == "-" => read_stdin(),
        (Some(q), _) => Ok(q.clone()),
        (None, Some(path)) => read_file(path),
        // clap requires one of the two.
        (None, None) => Err(CliError::usage("no query given")),
    }
}

fn read_variables(args: &ApiArgs) -> Result<Value> {
    let mut vars = Map::new();

    if let Some(path) = &args.variables_file {
        let text = if path == Path::new("-") {
            read_stdin()?
        } else {
            read_file(path)?
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => vars = map,
            Ok(_) => {
                return Err(CliError::usage(
                    "--variables-file must contain a JSON object",
                ))
            }
            Err(e) => {
                return Err(CliError::usage(format!(
                    "--variables-file is not valid JSON: {e}"
                )))
            }
        }
    }

    // Flags override the file, and later flags override earlier ones.
    for raw in &args.var {
        let (key, value) = split_pair(raw, "--var")?;
        vars.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    for raw in &args.var_json {
        let (key, value) = split_pair(raw, "--var-json")?;
        let parsed = serde_json::from_str(value).map_err(|e| {
            CliError::usage(format!(
                "--var-json {key}: the value is not valid JSON: {e}"
            ))
        })?;
        vars.insert(key.to_owned(), parsed);
    }

    Ok(if vars.is_empty() {
        Value::Null
    } else {
        Value::Object(vars)
    })
}

fn split_pair<'a>(raw: &'a str, flag: &str) -> Result<(&'a str, &'a str)> {
    match raw.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key, value)),
        _ => Err(CliError::usage(format!(
            "{flag} expects KEY=VALUE, got {raw:?}"
        ))),
    }
}

fn read_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))
}
