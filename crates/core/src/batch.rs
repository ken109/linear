//! The input of `linear issue batch`: several issue creations and updates in one JSON document.
//!
//! This is the pure half: the shape of the document (`schema()` is its JSON Schema, published
//! as `schema/issue-batch.schema.json`), reading it, and the checks that need nothing from
//! Linear. What needs Linear (names, ownership, validators) is the CLI's.
//!
//! ```json
//! {
//!   "issues": [
//!     { "op": "create", "title": "Fix the thing", "project": "My Project", "labels": ["bug"] },
//!     { "op": "update", "issue": "KK-12", "state": "In Progress", "priority": "high" }
//!   ]
//! }
//! ```
//!
//! Every field is the flag of the same name of `issue create` or `issue update` (`heldOn` is
//! `--held-on`, `addLabels` is `--add-labels`). A body is given inline, not from a file.

use crate::error::{Error, Result};
use crate::filters::priority_number;
use crate::metadata::{AttachmentMetadata, MetadataError};
use chrono::NaiveDate;
use schemars::generate::SchemaSettings;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

/// The most items one batch takes. A batch is one request per mutation, one after the other,
/// and a failure undoes the ones before it, so a long one is a long exposure to a failure.
pub const MAX_ITEMS: usize = 50;

/// A batch of issue writes, applied all or nothing.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(
    title = "linear issue batch",
    description = "Several issue creations and updates, applied all or nothing by `linear issue batch --file`."
)]
pub struct Batch {
    /// The writes, applied in this order. Each issue may be written once, and a source URL
    /// may appear once.
    pub issues: Vec<Item>,
}

/// One write.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Item {
    /// `issue create`
    Create(Create),
    /// `issue update`
    Update(Update),
}

/// Fields of `issue create`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    /// The issue's title (`--title`).
    pub title: String,
    /// The project to create it in: id, slug id, URL or name (`--project`).
    pub project: String,
    /// The description, as markdown text (`--body-file`, inline).
    pub body: Option<String>,
    /// The Linear template the description must follow (`--template`).
    pub template: Option<String>,
    /// Where the issue came from: an http(s) URL attached to the issue (`--source`). An issue
    /// that already carries it is returned instead of creating another.
    pub source: Option<String>,
    /// Title of the source attachment (`--source-title`); needs `source`.
    pub source_title: Option<String>,
    /// Metadata of the source attachment (`--meta`): strings and numbers; needs `source`.
    pub meta: Option<BTreeMap<String, MetaScalar>>,
    /// Milestone of the project, by name (`--milestone`).
    pub milestone: Option<String>,
    /// `me`, an email or a name (`--assignee`; default: you).
    pub assignee: Option<String>,
    /// Label names (`--label`).
    #[serde(default)]
    pub labels: Vec<String>,
    /// The day of the meeting the issue came out of; it goes into the cycle that contains the
    /// day after it (`--held-on`). Not with `cycle`.
    pub held_on: Option<NaiveDate>,
    /// 0 to 4, or `none`, `urgent`, `high`, `medium`, `low` (`--priority`).
    pub priority: Option<Priority>,
    /// A whole number in the team's scale (`--estimate`).
    pub estimate: Option<u32>,
    /// Make it a sub-issue of this existing issue (`--parent`).
    pub parent: Option<String>,
    /// The number of a cycle of the team (`--cycle`). Not with `heldOn`.
    pub cycle: Option<u32>,
    /// Team key (`--team`; default: the workspace's `default_team`).
    pub team: Option<String>,
    /// Allow creating, in a project somebody else leads, an issue assigned to you
    /// (`--allow-foreign`).
    #[serde(default)]
    pub allow_foreign: bool,
}

/// Fields of `issue update`. At least one besides `issue` is needed.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    /// The issue: identifier such as `KK-12`, or id. It must exist.
    pub issue: String,
    /// Workflow state, by name (`--state`).
    pub state: Option<String>,
    /// Move to this project (`--project`).
    pub project: Option<String>,
    /// Milestone of the (new) project, by name (`--milestone`).
    pub milestone: Option<String>,
    /// Due date, `YYYY-MM-DD` (`--due`).
    pub due: Option<NaiveDate>,
    /// `me`, an email or a name (`--assignee`).
    pub assignee: Option<String>,
    /// The new description, as markdown text (`--body-file`, inline).
    pub body: Option<String>,
    /// The Linear template the new description must follow (`--template`); needs `body`.
    pub template: Option<String>,
    /// Attach this http(s) URL as the issue's source (`--source`).
    pub source: Option<String>,
    /// Title of the source attachment (`--source-title`); needs `source`.
    pub source_title: Option<String>,
    /// Metadata of the source attachment (`--meta`): strings and numbers; needs `source`.
    pub meta: Option<BTreeMap<String, MetaScalar>>,
    /// Set the labels to exactly these (`--labels`). Not with `addLabels` or `removeLabels`.
    pub labels: Option<Vec<String>>,
    /// Add these labels (`--add-labels`).
    pub add_labels: Option<Vec<String>>,
    /// Remove these labels (`--remove-labels`).
    pub remove_labels: Option<Vec<String>>,
    /// 0 to 4, or `none`, `urgent`, `high`, `medium`, `low` (`--priority`).
    pub priority: Option<Priority>,
    /// A whole number in the team's scale, or `"none"` to remove it (`--estimate`).
    pub estimate: Option<NumberOrNone>,
    /// Make it a sub-issue of this issue, or `"none"` to make it top-level (`--parent`).
    pub parent: Option<String>,
    /// The number of a cycle of the issue's team, or `"none"` to take it out (`--cycle`).
    pub cycle: Option<NumberOrNone>,
}

/// A priority: a number `0` to `4`, or its name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Priority {
    Number(i64),
    Name(String),
}

impl Priority {
    /// Linear's priority number, or why it is not one.
    pub fn number(&self) -> std::result::Result<i32, String> {
        match self {
            Priority::Number(n) => priority_number(&n.to_string()),
            Priority::Name(s) => priority_number(s),
        }
    }
}

/// A whole number, or the word `none`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NumberOrNone {
    Number(u32),
    /// Only `"none"`.
    None(String),
}

impl NumberOrNone {
    /// `Some(n)` for a number, `None` for `"none"`; anything else is an error.
    pub fn value(&self) -> std::result::Result<Option<u32>, String> {
        match self {
            NumberOrNone::Number(n) => Ok(Some(*n)),
            NumberOrNone::None(s) if s.trim().eq_ignore_ascii_case("none") => Ok(None),
            NumberOrNone::None(s) => Err(format!("{s:?} is neither a whole number nor \"none\"")),
        }
    }
}

/// A metadata value: a string or a number, nothing nested.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MetaScalar {
    Text(String),
    Number(Number),
}

/// The metadata of an item as the validated object `issue create --meta` builds.
pub fn metadata(
    meta: &Option<BTreeMap<String, MetaScalar>>,
) -> std::result::Result<Option<AttachmentMetadata>, MetadataError> {
    let Some(meta) = meta else { return Ok(None) };
    let object: Map<String, Value> = meta
        .iter()
        .map(|(k, v)| {
            let value = match v {
                MetaScalar::Text(s) => Value::String(s.clone()),
                MetaScalar::Number(n) => Value::Number(n.clone()),
            };
            (k.clone(), value)
        })
        .collect();
    AttachmentMetadata::from_json(&object).map(Some)
}

impl Item {
    /// What the item says about itself for a message: `create "Fix it"` or `update KK-12`.
    pub fn describe(&self) -> String {
        match self {
            Item::Create(c) => format!("create {:?}", c.title),
            Item::Update(u) => format!("update {}", u.issue),
        }
    }

    /// The source URL the item attaches, trimmed.
    pub fn source(&self) -> Option<&str> {
        match self {
            Item::Create(c) => c.source.as_deref(),
            Item::Update(u) => u.source.as_deref(),
        }
        .map(str::trim)
    }
}

/// Read a batch. A document that is not JSON of this shape (an unknown field, a missing
/// `op`, a wrong type) is a usage error that says where.
pub fn parse(text: &str) -> Result<Batch> {
    serde_json::from_str(text).map_err(|e| {
        Error::Usage(format!(
            "the batch is not valid: {e} (the shape is `linear issue batch --schema`)"
        ))
    })
}

impl Batch {
    /// Everything that can be judged from the document alone: every problem, each naming its
    /// item (`issues[2]`). An empty list means the document may go on to the checks that need
    /// Linear.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.issues.is_empty() {
            problems.push("issues is empty: a batch needs at least one item".to_owned());
        }
        if self.issues.len() > MAX_ITEMS {
            problems.push(format!(
                "issues has {} items; a batch takes at most {MAX_ITEMS}",
                self.issues.len()
            ));
        }
        for (i, item) in self.issues.iter().enumerate() {
            for problem in item_problems(item) {
                problems.push(format!("issues[{i}] ({}): {problem}", item.describe()));
            }
        }
        // A source may be attached to one issue only: two items with it would both look
        // fine until the second one is written.
        let mut sources: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, item) in self.issues.iter().enumerate() {
            let Some(source) = item.source().filter(|s| !s.is_empty()) else {
                continue;
            };
            if let Some(first) = sources.insert(source, i) {
                problems.push(format!(
                    "issues[{i}] ({}): the source {source} is already used by issues[{first}]",
                    item.describe()
                ));
            }
        }
        problems
    }
}

fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn item_problems(item: &Item) -> Vec<String> {
    let mut problems = Vec::new();
    match item {
        Item::Create(c) => {
            if blank(&c.title) {
                problems.push("title is empty".to_owned());
            }
            if blank(&c.project) {
                problems.push("project is empty".to_owned());
            }
            if (c.source_title.is_some() || c.meta.is_some()) && c.source.is_none() {
                problems.push("sourceTitle and meta need a source".to_owned());
            }
            if c.held_on.is_some() && c.cycle.is_some() {
                problems.push("heldOn and cycle cannot be given together".to_owned());
            }
            if let Some(p) = &c.priority {
                if let Err(e) = p.number() {
                    problems.push(e);
                }
            }
            if let Err(e) = metadata(&c.meta) {
                problems.push(e.to_string());
            }
            if c.body.as_deref().is_some_and(blank) {
                problems.push("body is empty".to_owned());
            }
        }
        Item::Update(u) => {
            if blank(&u.issue) {
                problems.push("issue is empty".to_owned());
            }
            let sets_labels = u.labels.as_ref().is_some_and(|l| !l.is_empty());
            let edits_labels = u.add_labels.as_ref().is_some_and(|l| !l.is_empty())
                || u.remove_labels.as_ref().is_some_and(|l| !l.is_empty());
            if u.labels.as_ref().is_some_and(Vec::is_empty) {
                problems.push(
                    "labels is empty; to take labels off an issue name them in removeLabels"
                        .to_owned(),
                );
            }
            if sets_labels && edits_labels {
                problems
                    .push("labels cannot be combined with addLabels or removeLabels".to_owned());
            }
            if u.template.is_some() && u.body.is_none() {
                problems.push("template needs a body".to_owned());
            }
            if (u.source_title.is_some() || u.meta.is_some()) && u.source.is_none() {
                problems.push("sourceTitle and meta need a source".to_owned());
            }
            if u.body.as_deref().is_some_and(blank) {
                problems.push("body is empty".to_owned());
            }
            if let Some(p) = &u.priority {
                if let Err(e) = p.number() {
                    problems.push(e);
                }
            }
            for value in [&u.estimate, &u.cycle].into_iter().flatten() {
                if let Err(e) = value.value() {
                    problems.push(e);
                }
            }
            if let Err(e) = metadata(&u.meta) {
                problems.push(e.to_string());
            }
            let changes = u.state.is_some()
                || u.project.is_some()
                || u.milestone.is_some()
                || u.due.is_some()
                || u.assignee.is_some()
                || u.body.is_some()
                || u.source.is_some()
                || sets_labels
                || edits_labels
                || u.priority.is_some()
                || u.estimate.is_some()
                || u.parent.is_some()
                || u.cycle.is_some();
            if !changes {
                problems.push("nothing to change besides issue".to_owned());
            }
        }
    }
    problems
}

/// The JSON Schema of a batch (draft 2020-12), as `schema/issue-batch.schema.json` holds it.
pub fn schema() -> Value {
    let generator = SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator();
    let schema = generator.into_root_schema_for::<Batch>();
    serde_json::to_value(schema).expect("a schema serializes")
}
