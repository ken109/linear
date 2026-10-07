//! The shared path every write takes.
//!
//! ```text
//! write_session  ->  guard  ->  validate  ->  mutate (with Rollback)
//! ```
//!
//! 1. [`Ctx::write_session`] resolves the workspace, checks the credentials
//!    belong to it, and learns who "me" is (the viewer).
//! 2. [`WriteSession::guard`] asks the ownership rules (as strict as the
//!    workspace's `ownership` setting says) whether the viewer may make this
//!    write (exit code 4 when not). With `--force`, in a workspace that sets
//!    `allow_force`, a refusal is overridden instead, and reported: on stderr
//!    when it happens, and in the `--json` output of the command
//!    ([`WriteSession::emit`]). Only this step is bypassed.
//! 3. [`WriteSession::validate`] runs the workspace's enabled validator rules
//!    (exit code 5 when one fails), fetching whatever they need first. It may
//!    also answer "this already exists" (source idempotence).
//! 4. The command then mutates. A write made of several requests registers
//!    how to undo each step on a [`Rollback`], so a failure part way leaves
//!    nothing half done.
//!
//! Everything before step 4 only reads: a refused write sends no mutation.
//! Resolving names (`resolve`) is part of that: an unknown name is a usage
//! error, never silently dropped.
//!
//! `issue` holds the issue commands built on this path; the project,
//! milestone, initiative and template writes use the same pieces.

pub mod comment;
pub mod document;
pub mod file;
pub mod initiative;
pub mod issue;
pub mod label;
pub mod milestone;
pub mod project;
pub mod resolve;
pub mod template;

use super::listing::Session;
use super::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::Output;
use clap::Args;
use linear_core::config::Ownership;
use linear_core::error::ErrorCode;
use linear_core::guard::{self, Denied, Verdict, Viewer, Write};
use linear_core::queries;
use linear_core::read;
use linear_core::rules::{Draft, Fetched, Needs, Outcome, Rejected, RuleSet};
use serde::Serialize;
use std::cell::RefCell;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

/// `--force`, on every command that asks the ownership rules.
#[derive(Debug, Args, Clone, Copy, Default)]
pub struct ForceArg {
    /// Write even where the ownership rules would refuse (exit 4). Needs `allow_force = true`
    /// for the workspace in workspaces.toml (exit 2 otherwise). Validators and `--yes` still
    /// apply. An override is reported on stderr, and as `forced` in the --json output
    #[arg(long)]
    pub force: bool,
}

/// An ownership refusal that `--force` overrode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Overridden {
    /// What was written: `project "Roadmap"`, `issue KK-12`.
    pub target: String,
    #[serde(flatten)]
    pub denied: Denied,
}

impl From<Denied> for CliError {
    /// A write refused by the ownership rules: exit code 4.
    fn from(d: Denied) -> Self {
        CliError::new(d.code(), d.message)
    }
}

impl From<Rejected> for CliError {
    /// A write refused by a validator: exit code 5. The message lists every violation.
    fn from(r: Rejected) -> Self {
        CliError::new(r.code(), r.to_string())
    }
}

/// A workspace ready to be written to.
pub struct WriteSession {
    pub workspace: String,
    pub client: Client,
    /// The validators this workspace enabled.
    pub rules: RuleSet,
    /// The workspace's default team key, if configured.
    pub default_team: Option<String>,
    /// The title of a new source attachment when `--source-title` is not given.
    pub source_title: String,
    /// "Me" in this workspace.
    pub viewer: Viewer,
    /// How strictly the ownership rules apply in this workspace.
    pub ownership: Ownership,
    /// `--force` was given (and the workspace allows it).
    force: bool,
    /// The ownership refusals `--force` has overridden so far in this run.
    forced: RefCell<Vec<Overridden>>,
    pub out: Output,
}

impl Ctx {
    /// Open a workspace for writing.
    ///
    /// Unlike a read, this first checks that the credentials belong to the
    /// workspace the configuration names (a key for the wrong workspace must
    /// never write), and fetches the viewer the ownership rules compare
    /// against. The viewer is looked up once per run.
    pub fn write_session(&self) -> Result<WriteSession> {
        self.write_session_with(ForceArg::default())
    }

    /// [`Ctx::write_session`] for a command that takes `--force`. `--force` in a workspace
    /// that does not set `allow_force` is a usage error, before anything is sent.
    pub fn write_session_with(&self, force: ForceArg) -> Result<WriteSession> {
        let stored = crate::store::read_config(&self.dirs)?;
        let resolved = self.resolve(&stored)?;
        if force.force && !resolved.config.allow_force {
            return Err(CliError::usage(format!(
                "--force needs `allow_force = true` for workspace {:?} in workspaces.toml; \
                 nothing was sent",
                resolved.name
            )));
        }
        let Session {
            workspace,
            config,
            client,
        } = self.session_for(&resolved.name, resolved.config)?;
        let who = super::verify(&client, &workspace, &config.url_key)?;
        Ok(WriteSession {
            viewer: Viewer::from_user(&workspace, &who.viewer),
            ownership: config.ownership,
            force: force.force,
            forced: RefCell::default(),
            rules: RuleSet::from_workspace(&config),
            default_team: config.default_team.clone(),
            source_title: config.default_source_title().to_owned(),
            workspace,
            client,
            out: self.out,
        })
    }
}

impl WriteSession {
    /// Step 2: may the viewer make this write to `target`? Exit code 4 when not.
    ///
    /// With `--force` a refusal is overridden instead: it is reported on stderr now,
    /// before anything is sent, and kept for [`WriteSession::emit`]. A write the rules
    /// allow is not reported, forced or not.
    pub fn guard(&self, target: &str, write: &Write<'_>, allow_foreign: bool) -> Result<()> {
        let verdict = guard::check_forced(
            &self.viewer,
            self.ownership,
            write,
            allow_foreign,
            self.force,
        )?;
        if let Verdict::Forced(denied) = verdict {
            self.out.status(&format!(
                "warning: --force overrides the ownership rules for {target} ({}): {}{}",
                denied.operation,
                denied.message,
                held_by(&denied)
            ));
            self.forced.borrow_mut().push(Overridden {
                target: target.to_owned(),
                denied,
            });
        }
        Ok(())
    }

    /// The refusals `--force` overrode in this run.
    pub fn overridden(&self) -> Vec<Overridden> {
        self.forced.borrow().clone()
    }

    /// [`Output::emit`], with `forced: true` and the refusals that were overridden added to
    /// the `--json` output when `--force` overrode any. Otherwise the output is unchanged.
    pub fn emit<T: Serialize>(
        &self,
        value: &T,
        human: impl FnOnce() -> String,
        quiet: impl FnOnce() -> String,
    ) {
        let overridden = self.overridden();
        if !self.out.json || overridden.is_empty() {
            return self.out.emit(value, human, quiet);
        }
        let mut json = serde_json::to_value(value).expect("output always serializes");
        if let serde_json::Value::Object(map) = &mut json {
            map.insert("forced".into(), true.into());
            map.insert(
                "overridden".into(),
                serde_json::to_value(&overridden).expect("output always serializes"),
            );
        }
        self.out.emit(&json, human, quiet)
    }

    /// Step 3: run the enabled validators on what is about to be written.
    ///
    /// Reads whatever the rules ask for (templates, issues that already carry
    /// the source URL), then checks everything at once. A violation is exit
    /// code 5 and lists them all; [`Outcome::AlreadyExists`] means the write is
    /// already done and nothing should be sent.
    pub fn validate(&self, draft: &Draft) -> Result<Outcome> {
        let needs = self.rules.needs(draft);
        let fetched = self.fetch(&needs)?;
        Ok(self.rules.check(draft, &fetched)?)
    }

    fn fetch(&self, needs: &Needs) -> Result<Fetched> {
        let mut fetched = Fetched::default();
        if !needs.templates.is_empty() {
            let data: queries::Templates = self.client.execute(&queries::templates())?;
            fetched.templates = data.templates;
        }
        for url in &needs.source_urls {
            let data: read::AttachmentsForUrlQuery =
                self.client.execute(&read::attachments_for_url(url))?;
            if let Some(owner) = data.attachments_for_url.nodes.into_iter().next() {
                fetched.existing_by_source.insert(url.clone(), owner.issue);
            }
        }
        Ok(fetched)
    }

    /// Say something to the person at the terminal (stderr; silent with `--quiet`/`--json`).
    pub fn note(&self, message: &str) {
        self.out.status(&format!("note: {message}"));
    }
}

/// `; held by: lead user-bob, assignee nobody`: who the write would have needed to be you.
fn held_by(denied: &Denied) -> String {
    if denied.held.is_empty() {
        return String::new();
    }
    let holders: Vec<String> = denied
        .held
        .iter()
        .map(|h| format!("{} {}", h.role, h.user.as_deref().unwrap_or("nobody")))
        .collect();
    format!("; held by: {}", holders.join(", "))
}

// ---------------------------------------------------------------- rollback

type Undo<'a> = Box<dyn FnOnce() -> Result<()> + 'a>;

/// What to undo if a later step of a multi-request write fails.
///
/// Register an undo right after each step that succeeded. If a later step
/// fails, hand the error to [`Rollback::fail`]: it runs the undos newest
/// first and returns an error that says what was undone and, if an undo
/// itself failed, what was left behind. When every step succeeds, just drop
/// the `Rollback`.
#[derive(Default)]
pub struct Rollback<'a> {
    steps: Vec<(String, Undo<'a>)>,
}

impl<'a> Rollback<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `what` names the step in the error message (`"delete KK-12"`).
    pub fn on_failure(&mut self, what: impl Into<String>, undo: impl FnOnce() -> Result<()> + 'a) {
        self.steps.push((what.into(), Box::new(undo)));
    }

    /// Undo everything registered, then return `cause` with the outcome appended.
    pub fn fail(self, cause: CliError) -> CliError {
        if self.steps.is_empty() {
            return cause;
        }
        let mut undone = Vec::new();
        let mut stuck = Vec::new();
        for (what, undo) in self.steps.into_iter().rev() {
            match undo() {
                Ok(()) => undone.push(what),
                Err(e) => stuck.push(format!("{what} ({})", e.message)),
            }
        }
        let mut message = cause.message;
        if !undone.is_empty() {
            message.push_str(&format!("; rolled back: {}", undone.join(", ")));
        }
        if !stuck.is_empty() {
            message.push_str(&format!(
                "; COULD NOT roll back, clean up by hand: {}",
                stuck.join(", ")
            ));
        }
        CliError::new(cause.code, message)
    }
}

// ---------------------------------------------------------------- retry

/// Waits before each attempt of a step that fails intermittently.
pub const ATTACH_WAITS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(1000),
    Duration::from_millis(3000),
];

/// Run `step` once per entry of `waits` (sleeping that long first) until it
/// succeeds. Authentication errors are not retried. Returns the last error.
pub fn retry<T>(
    out: Output,
    what: &str,
    waits: &[Duration],
    mut step: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut last = None;
    for (i, wait) in waits.iter().enumerate() {
        if !wait.is_zero() {
            std::thread::sleep(*wait);
        }
        match step() {
            Ok(v) => return Ok(v),
            Err(e) if e.code == ErrorCode::Auth => return Err(e),
            Err(e) => {
                if i + 1 < waits.len() {
                    out.status(&format!(
                        "{what} failed, trying again in {}ms: {}",
                        waits[i + 1].as_millis(),
                        e.message
                    ));
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| CliError::general(format!("{what}: no attempt was made"))))
}

// ---------------------------------------------------------------- input text

/// Read text given as a file path, or from standard input when the path is `-`.
pub fn read_text(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return Ok(text);
    }
    std::fs::read_to_string(path)
        .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))
}
