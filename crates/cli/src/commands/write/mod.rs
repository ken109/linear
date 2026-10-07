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
//!    write (exit code 4 when not).
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

pub mod file;
pub mod initiative;
pub mod issue;
pub mod milestone;
pub mod project;
pub mod resolve;
pub mod template;

use super::listing::Session;
use super::Ctx;
use crate::error::{CliError, Result};
use crate::http::Client;
use crate::output::Output;
use linear_core::config::Ownership;
use linear_core::error::ErrorCode;
use linear_core::guard::{self, Denied, Viewer, Write};
use linear_core::queries;
use linear_core::read;
use linear_core::rules::{Draft, Fetched, Needs, Outcome, Rejected, RuleSet};
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

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
        let Session {
            workspace,
            config,
            client,
        } = self.session()?;
        let who = super::verify(&client, &workspace, &config.url_key)?;
        Ok(WriteSession {
            viewer: Viewer::from_user(&workspace, &who.viewer),
            ownership: config.ownership,
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
    /// Step 2: may the viewer make this write? Exit code 4 when not.
    pub fn guard(&self, write: &Write<'_>, allow_foreign: bool) -> Result<()> {
        Ok(guard::check_with(
            &self.viewer,
            self.ownership,
            write,
            allow_foreign,
        )?)
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
