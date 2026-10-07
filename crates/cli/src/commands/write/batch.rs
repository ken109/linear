//! `linear issue batch --file FILE|-`: several issue creations and updates, all or nothing.
//!
//! ```text
//! parse + check  ->  plan every item  ->  send in order  ->  (a failure) undo what was sent
//! ```
//!
//! 1. The document is read and checked against what it can say alone
//!    (`linear_core::batch`): the shape, the items, a source used twice. Usage errors (exit 2),
//!    every problem named by its item, before anything is asked of Linear.
//! 2. **Every item is planned before the first is sent.** Each one goes through the code of
//!    `issue create` or `issue update` in a session that records its mutations (the code of
//!    `--dry-run`): names are resolved, the ownership rules and the validators are asked, an
//!    "already exists" is found. A refusal in any item stops the whole batch, and all the
//!    items' refusals are reported together, each with its index; the exit code is the first
//!    one's. Two items that turn out to write the same issue are refused too.
//! 3. With `--dry-run` the plan is printed here and nothing is sent.
//! 4. Otherwise the recorded mutations are sent in order, with one exception: what cannot be
//!    taken back (a source attached to an issue that exists) is sent after everything that can.
//!    A mutation that fails undoes the ones before it, newest first.
//!
//! What a rollback cannot do: take a source attachment off an existing issue or restore the
//! title or metadata of one (an `update` item's `source`; it is sent last so that a failure
//! almost always comes before it), and un-delete anything for good: an issue made by a `create`
//! item is moved to Linear's trash, which keeps it for a while, not erased. An undo that itself
//! fails is named in the error.

use super::dry_run::{Collected, Step, Target, NEW_ISSUE_ID};
use super::issue::{create_in, update_in, CreateCmd, CreateInputs, UpdateCmd, UpdateInputs};
use super::{read_text, retry, ForceArg, Rollback, ATTACH_WAITS};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::Output;
use clap::Args;
use linear_core::batch::{self, Batch, Create, Item, Update};
use linear_core::error::ErrorCode;
use linear_core::inputs::Patch;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct BatchCmd {
    /// The batch as JSON: a file, or `-` for standard input (`--schema` prints its shape)
    #[arg(long, value_name = "FILE", required_unless_present = "schema")]
    pub file: Option<PathBuf>,
    /// Print the JSON Schema of the batch and exit; needs no workspace or credentials
    #[arg(long, conflicts_with = "file")]
    pub schema: bool,
}

/// `issue batch --schema`.
pub fn print_schema(out: Output) {
    let schema = batch::schema();
    // The schema is the output, whatever `--json` and `--quiet` say.
    let _ = out;
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("a schema serializes")
    );
}

/// One item, turned into what `issue create` or `issue update` takes.
enum Work {
    Create(Box<CreateCmd>, CreateInputs),
    Update(Box<UpdateCmd>, UpdateInputs),
}

pub fn run(ctx: &Ctx, cmd: &BatchCmd) -> Result<()> {
    if cmd.schema {
        print_schema(ctx.out);
        return Ok(());
    }
    let path = cmd
        .file
        .as_deref()
        .ok_or_else(|| CliError::usage("pass --file FILE (or `-` for standard input)"))?;
    let text = read_text(path)?;
    let batch = batch::parse(&text)?;
    let problems = batch.problems();
    if !problems.is_empty() {
        return Err(CliError::usage(problems.join("\n")));
    }
    let work = convert(&batch)?;

    // Plan every item: the same path as the real command, recording instead of sending.
    let ws = ctx.write_session_in(ForceArg::default(), true)?;
    ws.collect_plans();
    let mut failures: Vec<(usize, CliError)> = Vec::new();
    for (i, (item, work)) in batch.issues.iter().zip(work).enumerate() {
        ws.discard_recorded();
        let result = match work {
            Work::Create(cmd, inputs) => create_in(&ws, &cmd, inputs),
            Work::Update(cmd, inputs) => update_in(&ws, &cmd, inputs),
        };
        if let Err(e) = result {
            // The credentials are wrong for everything; do not repeat that per item.
            if e.code == ErrorCode::Auth {
                return Err(e);
            }
            ws.discard_recorded();
            failures.push((
                i,
                CliError::new(e.code, format!("{}: {}", label(i, item), e.message)),
            ));
        }
    }
    let plans = ws.take_collected();
    if !failures.is_empty() {
        let code = failures[0].1.code;
        let n = failures.len();
        let mut message = failures
            .into_iter()
            .map(|(_, e)| e.message)
            .collect::<Vec<_>>()
            .join("\n");
        message.push_str(&format!(
            "\nnothing was sent: {n} of {} item(s) cannot be written",
            batch.issues.len()
        ));
        return Err(CliError::new(code, message));
    }
    if plans.len() != batch.issues.len() {
        return Err(CliError::general(format!(
            "internal error: {} item(s) but {} plan(s); nothing was sent",
            batch.issues.len(),
            plans.len()
        )));
    }
    check_distinct(&batch, &plans)?;

    let order = send_order(&plans);
    if ctx.dry_run {
        print_plan(ctx.out, &ws.workspace, &batch, &plans, &order);
        return Ok(());
    }

    let client = ctx.session()?.client;
    let applied = apply(ctx.out, &client, &batch, &plans, &order)?;
    print_result(ctx.out, &ws.workspace, &batch, &plans, &applied);
    Ok(())
}

fn label(i: usize, item: &Item) -> String {
    format!("issues[{i}] ({})", item.describe())
}

// ---------------------------------------------------------------- items to commands

fn convert(batch: &Batch) -> Result<Vec<Work>> {
    batch
        .issues
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let made = match item {
                Item::Create(c) => create_work(c),
                Item::Update(u) => update_work(u),
            };
            made.map_err(|e| CliError::usage(format!("{}: {e}", label(i, item))))
        })
        .collect()
}

fn create_work(c: &Create) -> std::result::Result<Work, String> {
    let metadata = batch::metadata(&c.meta).map_err(|e| e.to_string())?;
    let cmd = CreateCmd {
        title: c.title.clone(),
        project: c.project.clone(),
        body_file: None,
        template: c.template.clone(),
        source: c.source.clone(),
        source_title: c.source_title.clone(),
        meta: Vec::new(),
        milestone: c.milestone.clone(),
        assignee: c.assignee.clone(),
        label: c.labels.clone(),
        held_on: c.held_on,
        priority: c.priority.as_ref().map(|p| p.number()).transpose()?,
        estimate: c
            .estimate
            .map(|e| i32::try_from(e).map_err(|_| format!("estimate {e} is too large")))
            .transpose()?,
        parent: c.parent.clone(),
        cycle: c.cycle,
        team: c.team.clone(),
        allow_foreign: c.allow_foreign,
        force: ForceArg::default(),
    };
    Ok(Work::Create(
        Box::new(cmd),
        CreateInputs {
            body: c.body.clone(),
            metadata,
        },
    ))
}

fn update_work(u: &Update) -> std::result::Result<Work, String> {
    let metadata = batch::metadata(&u.meta).map_err(|e| e.to_string())?;
    let estimate = match &u.estimate {
        None => None,
        Some(value) => Some(match value.value()? {
            Some(n) => {
                Patch::Set(i32::try_from(n).map_err(|_| format!("estimate {n} is too large"))?)
            }
            None => Patch::Clear,
        }),
    };
    let cycle = match &u.cycle {
        None => None,
        Some(value) => Some(match value.value()? {
            Some(n) => Patch::Set(n),
            None => Patch::Clear,
        }),
    };
    let cmd = UpdateCmd {
        issue: u.issue.clone(),
        state: u.state.clone(),
        project: u.project.clone(),
        milestone: u.milestone.clone(),
        due: u.due,
        assignee: u.assignee.clone(),
        body_file: None,
        template: u.template.clone(),
        source: u.source.clone(),
        source_title: u.source_title.clone(),
        meta: Vec::new(),
        labels: u.labels.clone().unwrap_or_default(),
        add_labels: u.add_labels.clone().unwrap_or_default(),
        remove_labels: u.remove_labels.clone().unwrap_or_default(),
        priority: u.priority.as_ref().map(|p| p.number()).transpose()?,
        estimate,
        parent: u.parent.clone(),
        cycle,
        force: ForceArg::default(),
    };
    Ok(Work::Update(
        Box::new(cmd),
        UpdateInputs {
            body: u.body.clone(),
            metadata,
        },
    ))
}

// ---------------------------------------------------------------- planned

/// Two items that write the same issue would each be planned against what it is now, and the
/// second would undo or double the first.
fn check_distinct(batch: &Batch, plans: &[Collected]) -> Result<()> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut problems = Vec::new();
    for (i, collected) in plans.iter().enumerate() {
        let Some(id) = collected.plan.target.id.as_deref() else {
            continue;
        };
        if collected.mutations.is_empty() {
            continue;
        }
        if let Some(first) = seen.insert(id, i) {
            problems.push(format!(
                "{} and {} both write {}",
                label(first, &batch.issues[first]),
                label(i, &batch.issues[i]),
                collected.plan.target.name
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "{}\nnothing was sent: an issue may be written once in a batch",
            problems.join("\n")
        )))
    }
}

/// The mutations in the order they are sent, each with the item it belongs to: everything that
/// can be undone first, in item order, then what cannot.
fn send_order(plans: &[Collected]) -> Vec<(usize, &Step)> {
    let all = plans
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.mutations.iter().map(move |s| (i, s)));
    let (late, early): (Vec<_>, Vec<_>) = all.partition(|(_, s)| s.deferred);
    early.into_iter().chain(late).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemPlan<'a> {
    index: usize,
    op: &'static str,
    command: &'a str,
    target: &'a Target,
    changed: &'a [&'static str],
    mutations: Vec<&'a str>,
    reason: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchPlan<'a> {
    dry_run: bool,
    workspace: &'a str,
    command: &'static str,
    target: Target,
    changed: Vec<&'static str>,
    mutations: Vec<&'a Step>,
    rollback: Vec<&'a Step>,
    notes: Vec<&'a str>,
    reason: Option<&'static str>,
    items: Vec<ItemPlan<'a>>,
}

fn op_of(item: &Item) -> &'static str {
    match item {
        Item::Create(_) => "create",
        Item::Update(_) => "update",
    }
}

fn print_plan(
    out: Output,
    workspace: &str,
    batch: &Batch,
    plans: &[Collected],
    order: &[(usize, &Step)],
) {
    let n = order.len();
    let rollback: Vec<&Step> = order[..n.saturating_sub(1)]
        .iter()
        .rev()
        .filter_map(|(_, s)| s.undo.as_deref())
        .collect();
    let items: Vec<ItemPlan<'_>> = plans
        .iter()
        .enumerate()
        .map(|(i, c)| ItemPlan {
            index: i,
            op: op_of(&batch.issues[i]),
            command: c.plan.command,
            target: &c.plan.target,
            changed: &c.plan.changed,
            mutations: c.mutations.iter().map(|s| s.operation.as_str()).collect(),
            reason: c.mutations.is_empty().then_some(c.plan.reason),
        })
        .collect();
    let value = BatchPlan {
        dry_run: true,
        workspace,
        command: "issue batch",
        target: Target {
            kind: "issues",
            name: format!("{} item(s)", batch.issues.len()),
            id: None,
            new: false,
        },
        changed: Vec::new(),
        mutations: order.iter().map(|(_, s)| *s).collect(),
        rollback,
        notes: plans
            .iter()
            .flat_map(|c| c.notes.iter().map(String::as_str))
            .collect(),
        reason: order.is_empty().then_some("nothing to send"),
        items,
    };
    out.emit(
        &value,
        || {
            let mut text = format!(
                "dry run, nothing was sent: issue batch ({} item(s), workspace {workspace})",
                batch.issues.len()
            );
            for item in &value.items {
                let what = match (item.mutations.is_empty(), item.target.new) {
                    (true, _) => format!("nothing to send: {}", item.reason.unwrap_or("")),
                    (false, true) => format!("would create, sending {}", item.mutations.join(", ")),
                    (false, false) => format!(
                        "would change {}, sending {}",
                        if item.changed.is_empty() {
                            "nothing named".to_owned()
                        } else {
                            item.changed.join(", ")
                        },
                        item.mutations.join(", ")
                    ),
                };
                text.push_str(&format!(
                    "\n  {}. {} {}: {what}",
                    item.index + 1,
                    item.op,
                    item.target.name
                ));
            }
            text.push_str(&format!(
                "\n{} mutation(s) in all; use --json to see them",
                value.mutations.len()
            ));
            text
        },
        || {
            order
                .iter()
                .map(|(_, s)| s.operation.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
}

// ---------------------------------------------------------------- sending

/// What came back for an item that made an issue.
struct Made {
    id: String,
    identifier: String,
    url: String,
}

/// Send `request`; a payload that does not say `success: true` is a failure.
fn send(client: &Client, step: &Step, request: &linear_core::wire::Request) -> Result<Value> {
    let data: Value = client.execute_request(request)?;
    let ok = data
        .as_object()
        .and_then(|fields| fields.values().next())
        .and_then(|payload| payload.get("success"))
        .and_then(Value::as_bool)
        == Some(true);
    if ok {
        Ok(data)
    } else {
        Err(CliError::general(format!(
            "Linear could not do {}",
            step.operation
        )))
    }
}

fn replace(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(s) if s == from => *s = to.to_owned(),
        Value::Array(items) => items.iter_mut().for_each(|v| replace(v, from, to)),
        Value::Object(fields) => fields.values_mut().for_each(|v| replace(v, from, to)),
        _ => {}
    }
}

fn apply(
    out: Output,
    client: &Client,
    batch: &Batch,
    plans: &[Collected],
    order: &[(usize, &Step)],
) -> Result<HashMap<usize, Made>> {
    let mut made: HashMap<usize, Made> = HashMap::new();
    let mut rollback = Rollback::new();
    for (item, step) in order {
        let item = *item;
        let new_id = made.get(&item).map(|m| m.id.clone());
        let fill = |v: &mut Value| {
            if let Some(id) = &new_id {
                replace(v, NEW_ISSUE_ID, id);
            }
        };
        let request = step.request(&fill);
        let sent = if step.operation == "AttachmentCreate" {
            // `attachmentCreate` upserts on the URL, so trying again is safe.
            retry(out, "attaching the source", &ATTACH_WAITS, || {
                send(client, step, &request)
            })
        } else {
            send(client, step, &request)
        };
        let data = match sent {
            Ok(data) => data,
            Err(cause) => {
                let cause = CliError::new(
                    cause.code,
                    format!("{}: {}", label(item, &batch.issues[item]), cause.message),
                );
                return Err(rollback.fail(cause));
            }
        };
        if step.operation == "IssueCreate" {
            let issue = &data["issueCreate"]["issue"];
            let text = |key: &str| issue[key].as_str().map(str::to_owned);
            match (text("id"), text("identifier"), text("url")) {
                (Some(id), Some(identifier), Some(url)) => {
                    made.insert(
                        item,
                        Made {
                            id,
                            identifier,
                            url,
                        },
                    );
                }
                _ => {
                    let cause = CliError::general(format!(
                        "{}: Linear created the issue but returned none",
                        label(item, &batch.issues[item])
                    ));
                    return Err(rollback.fail(cause));
                }
            }
        }
        if let Some(undo) = step.undo.as_deref() {
            // The placeholder may have just been filled in by this very step.
            let new_id = made.get(&item).map(|m| m.id.clone());
            let fill = |v: &mut Value| {
                if let Some(id) = &new_id {
                    replace(v, NEW_ISSUE_ID, id);
                }
            };
            let request = undo.request(&fill);
            let name = match (undo.operation.as_str(), made.get(&item)) {
                ("IssueDelete", Some(m)) => format!("deleted {}", m.identifier),
                _ => format!(
                    "undid {} for {}",
                    step.operation, plans[item].plan.target.name
                ),
            };
            rollback.on_failure(name, move || send(client, undo, &request).map(|_| ()));
        }
    }
    Ok(made)
}

// ---------------------------------------------------------------- result

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemResult<'a> {
    index: usize,
    op: &'static str,
    identifier: &'a str,
    id: Option<&'a str>,
    url: Option<&'a str>,
    /// `true` for a `create` that found the issue already there and made nothing.
    existing: bool,
    changed: &'a [&'static str],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Done<'a> {
    workspace: &'a str,
    results: Vec<ItemResult<'a>>,
    created: usize,
    updated: usize,
}

fn print_result(
    out: Output,
    workspace: &str,
    batch: &Batch,
    plans: &[Collected],
    made: &HashMap<usize, Made>,
) {
    let results: Vec<ItemResult<'_>> = plans
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let new = made.get(&i);
            ItemResult {
                index: i,
                op: op_of(&batch.issues[i]),
                identifier: new.map_or(c.plan.target.name.as_str(), |m| m.identifier.as_str()),
                id: new.map(|m| m.id.as_str()).or(c.plan.target.id.as_deref()),
                url: new.map(|m| m.url.as_str()),
                existing: matches!(batch.issues[i], Item::Create(_)) && new.is_none(),
                changed: &c.plan.changed,
            }
        })
        .collect();
    let value = Done {
        workspace,
        created: made.len(),
        updated: results
            .iter()
            .filter(|r| r.op == "update" && !r.changed.is_empty())
            .count(),
        results,
    };
    out.emit(
        &value,
        || {
            let mut lines: Vec<String> = value
                .results
                .iter()
                .map(|r| {
                    let how = match (r.op, r.existing, r.changed.is_empty()) {
                        ("create", false, _) => "created".to_owned(),
                        ("create", true, true) => "already exists, nothing created".to_owned(),
                        ("create", true, false) => {
                            format!("already exists; changed {}", r.changed.join(", "))
                        }
                        (_, _, true) => "already as asked, nothing changed".to_owned(),
                        (_, _, false) => format!("updated {}", r.changed.join(", ")),
                    };
                    format!("{}. {}  {}  ({how})", r.index + 1, r.op, r.identifier)
                })
                .collect();
            lines.push(format!(
                "{} created, {} updated",
                value.created, value.updated
            ));
            lines.join("\n")
        },
        || {
            value
                .results
                .iter()
                .map(|r| r.identifier)
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
}
