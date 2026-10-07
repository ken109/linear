//! `--dry-run`: what a write would send, and nothing sent.
//!
//! A dry run goes through exactly the steps of the real write: the workspace is
//! checked, names are resolved, the ownership rules and the validators run, and
//! every refusal is the same refusal with the same exit code. Only the last step
//! differs: instead of sending a mutation the command records it, and at the end
//! prints the list of mutations (the GraphQL operation name and its variables),
//! what the real run would write (`changed`), and how it would undo a half-done
//! write. Reads that resolving needs are still sent; no mutation ever is.
//!
//! The safety does not rest on the commands alone: a client made for a dry run
//! refuses to send a mutation (and to upload a file), so a write that forgot to
//! record its step fails instead of writing.
//!
//! The shape printed with `--json` (every key is always present):
//!
//! ```json
//! {
//!   "dryRun": true,
//!   "workspace": "main",
//!   "command": "issue update",
//!   "target": { "kind": "issue", "name": "KK-12", "id": "...", "new": false },
//!   "changed": ["state", "labels"],
//!   "mutations": [ { "operation": "IssueUpdate", "variables": { ... } } ],
//!   "rollback": [ { "operation": "IssueUpdate", "variables": { ... } } ],
//!   "notes": [],
//!   "reason": null,
//!   "forced": false,
//!   "overridden": []
//! }
//! ```
//!
//! `mutations` are in the order they would be sent. A value that only exists once an earlier
//! step has run (the id of the issue a create makes) is a placeholder such as
//! [`NEW_ISSUE_ID`]. `rollback` lists what would be sent, newest first, if a later step
//! failed. `notes` names what a plan does besides sending mutations (the bytes of a file
//! that would be uploaded). `reason` says why `mutations` is empty (a run that has nothing to send). With `--force`,
//! `forced` is `true` and `overridden` lists the ownership refusals the real run would override
//! (the same objects it reports), and they are reported on stderr as in the real run.

use super::{Overridden, WriteSession};
use crate::error::Result;
use linear_core::wire::{build_request, Request};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::cell::RefCell;

/// Stands for the id of the issue that an earlier step of the plan creates.
pub const NEW_ISSUE_ID: &str = "<id of the issue created by the first mutation>";
/// Stands for the id of the project that an earlier step of the plan creates.
pub const NEW_PROJECT_ID: &str = "<id of the project created by the first mutation>";

/// One mutation of the plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    /// The GraphQL operation name (`IssueCreate`).
    pub operation: String,
    /// Its variables, exactly as they would be sent.
    pub variables: Value,
    /// The document, for a caller that goes on to send the step (`issue batch`).
    #[serde(skip)]
    pub query: String,
    /// What puts it back, if a later step fails.
    #[serde(skip)]
    pub undo: Option<Box<Step>>,
    /// Cannot be undone, so a caller that sends several steps sends it after the others.
    #[serde(skip)]
    pub deferred: bool,
}

impl Step {
    pub fn of<Q, V: Serialize>(op: &cynic::Operation<Q, V>) -> Self {
        let request = build_request(op);
        Step {
            operation: request.operation_name.unwrap_or_default(),
            variables: request.variables,
            query: request.query,
            undo: None,
            deferred: false,
        }
    }

    /// The request that sends this step, with `substitute` applied to its variables.
    pub fn request(&self, substitute: &dyn Fn(&mut Value)) -> Request {
        let mut variables = self.variables.clone();
        substitute(&mut variables);
        Request {
            query: self.query.clone(),
            variables,
            operation_name: Some(self.operation.clone()),
        }
    }
}

/// What the commands of a dry run have recorded so far.
#[derive(Debug, Default)]
pub struct Recorded {
    mutations: Vec<Step>,
    notes: Vec<String>,
}

impl Recorded {
    /// What would be sent, newest first, if a later step failed: the undo of every step but
    /// the last (nothing follows it, so nothing can fail after it).
    fn rollback(&self) -> Vec<&Step> {
        let n = self.mutations.len().saturating_sub(1);
        self.mutations[..n]
            .iter()
            .rev()
            .filter_map(|s| s.undo.as_deref())
            .collect()
    }
}

pub type Recorder = RefCell<Recorded>;

/// A plan a command finished while a batch was collecting them.
#[derive(Debug)]
pub struct Collected {
    pub plan: Plan,
    pub mutations: Vec<Step>,
    pub notes: Vec<String>,
}

/// What a write is aimed at.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    /// `issue`, `project`, `milestone`, `initiative`, ...
    pub kind: &'static str,
    /// What a person calls it: an identifier, a name or a title.
    pub name: String,
    /// Its id, `null` for a thing the write would create.
    pub id: Option<String>,
    /// `true` when the write would create it.
    pub new: bool,
}

impl Target {
    pub fn existing(kind: &'static str, name: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            id: Some(id.into()),
            new: false,
        }
    }

    /// Several things at once (a reorder): named together, with no single id.
    pub fn several(kind: &'static str, names: &[String]) -> Self {
        Self {
            kind,
            name: names.join(", "),
            id: None,
            new: false,
        }
    }

    pub fn new(kind: &'static str, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            id: None,
            new: true,
        }
    }
}

/// What a command says about its own dry run, besides the mutations it recorded.
#[derive(Debug)]
pub struct Plan {
    pub command: &'static str,
    pub target: Target,
    /// The fields the real run would write (empty for commands that do not update fields).
    pub changed: Vec<&'static str>,
    /// Why there is nothing to send, used when no mutation was recorded.
    pub reason: &'static str,
}

impl Plan {
    pub fn new(command: &'static str, target: Target) -> Self {
        Self {
            command,
            target,
            changed: Vec::new(),
            reason: "nothing to send",
        }
    }

    pub fn changed(mut self, changed: Vec<&'static str>) -> Self {
        self.changed = changed;
        self
    }

    pub fn reason(mut self, reason: &'static str) -> Self {
        self.reason = reason;
        self
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Printed<'a> {
    dry_run: bool,
    workspace: &'a str,
    command: &'a str,
    target: &'a Target,
    changed: &'a [&'static str],
    mutations: &'a [Step],
    rollback: Vec<&'a Step>,
    notes: &'a [String],
    reason: Option<&'a str>,
    /// `--force` overrode an ownership refusal on the way to this plan (the real run would too).
    forced: bool,
    /// The refusals it overrode, as the real run reports them.
    overridden: Vec<Overridden>,
}

impl WriteSession {
    /// Note a mutation the real run would send.
    pub fn record<Q, V: Serialize>(&self, op: &cynic::Operation<Q, V>) {
        self.recorded.borrow_mut().mutations.push(Step::of(op));
    }

    /// Note a mutation that cannot be taken back (a source attached to an issue that already
    /// exists): a caller that sends several steps sends it after the others.
    pub fn record_deferred<Q, V: Serialize>(&self, op: &cynic::Operation<Q, V>) {
        let mut step = Step::of(op);
        step.deferred = true;
        self.recorded.borrow_mut().mutations.push(step);
    }

    /// Note what the real run sends to take back the mutation recorded last, when a later
    /// step fails.
    pub fn record_undo<Q, V: Serialize>(&self, op: &cynic::Operation<Q, V>) {
        if let Some(last) = self.recorded.borrow_mut().mutations.last_mut() {
            last.undo = Some(Box::new(Step::of(op)));
        }
    }

    /// Say about the plan something that is not a mutation (bytes that would be uploaded).
    pub fn plan_note(&self, note: impl Into<String>) {
        self.recorded.borrow_mut().notes.push(note.into());
    }

    /// Send `op` and say whether Linear reported success; in a dry run, record it and say yes.
    pub fn send_checked<Q, V, T>(
        &self,
        op: &cynic::Operation<Q, V>,
        success: impl FnOnce(T) -> bool,
    ) -> Result<bool>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        if self.dry_run {
            self.record(op);
            return Ok(true);
        }
        let data: T = self.client.execute(op)?;
        Ok(success(data))
    }

    /// Start keeping the plans commands finish instead of printing them.
    pub fn collect_plans(&self) {
        *self.collected.borrow_mut() = Some(Vec::new());
    }

    /// The plans kept since `collect_plans`.
    pub fn take_collected(&self) -> Vec<Collected> {
        self.collected.borrow_mut().take().unwrap_or_default()
    }

    /// Forget what a command recorded before it failed.
    pub fn discard_recorded(&self) {
        *self.recorded.borrow_mut() = Recorded::default();
    }

    /// Print the plan and finish: the last thing a dry run of a command does.
    pub fn finish_dry_run(&self, plan: Plan) -> Result<()> {
        if let Some(kept) = self.collected.borrow_mut().as_mut() {
            let recorded = std::mem::take(&mut *self.recorded.borrow_mut());
            kept.push(Collected {
                plan,
                mutations: recorded.mutations,
                notes: recorded.notes,
            });
            return Ok(());
        }
        let recorded = self.recorded.borrow();
        let reason = recorded.mutations.is_empty().then_some(plan.reason);
        let overridden = self.overridden();
        let value = Printed {
            dry_run: true,
            workspace: &self.workspace,
            command: plan.command,
            target: &plan.target,
            changed: &plan.changed,
            mutations: &recorded.mutations,
            rollback: recorded.rollback(),
            notes: &recorded.notes,
            reason,
            forced: !overridden.is_empty(),
            overridden,
        };
        self.out.emit(
            &value,
            || human(&self.workspace, &plan, &recorded, &value.overridden),
            || {
                recorded
                    .mutations
                    .iter()
                    .map(|s| s.operation.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        );
        Ok(())
    }
}

fn human(workspace: &str, plan: &Plan, recorded: &Recorded, overridden: &[Overridden]) -> String {
    let what = if plan.target.new {
        format!("new {} {:?}", plan.target.kind, plan.target.name)
    } else {
        format!("{} {}", plan.target.kind, plan.target.name)
    };
    let mut text = format!(
        "dry run, nothing was sent: {} ({what}, workspace {workspace})",
        plan.command
    );
    for o in overridden {
        text.push_str(&format!(
            "\nforced: --force would override the ownership rules for {} ({})",
            o.target, o.denied.operation
        ));
    }
    if !plan.changed.is_empty() {
        text.push_str(&format!("\nwould change: {}", plan.changed.join(", ")));
    }
    for note in &recorded.notes {
        text.push_str(&format!("\nnote: {note}"));
    }
    if recorded.mutations.is_empty() {
        text.push_str(&format!("\nno mutation: {}", plan.reason));
        return text;
    }
    text.push_str(&format!(
        "\nwould send {} mutation(s), in this order:",
        recorded.mutations.len()
    ));
    for (n, step) in recorded.mutations.iter().enumerate() {
        text.push_str(&format!("\n{}. {}", n + 1, step.operation));
        text.push_str(&indented(&step.variables));
    }
    let rollback = recorded.rollback();
    if !rollback.is_empty() {
        text.push_str("\nif a later mutation failed, it would send (newest first):");
        for step in rollback {
            text.push_str(&format!("\n- {}", step.operation));
            text.push_str(&indented(&step.variables));
        }
    }
    text
}

fn indented(variables: &Value) -> String {
    if variables.is_null() {
        return String::new();
    }
    let json = serde_json::to_string_pretty(variables).expect("variables serialize");
    json.lines().map(|l| format!("\n     {l}")).collect()
}
