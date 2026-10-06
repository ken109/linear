//! Workspace tagging.
//!
//! Every datum belongs to a workspace; identity is the pair `(workspace, id)`.
//! The tag is added after a response is parsed, never selected from Linear.

use serde::{Deserialize, Serialize};

/// A value tagged with the name of the workspace it came from.
///
/// Serializes flat: the wrapped value's fields plus a `workspace` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InWorkspace<T> {
    pub workspace: String,
    #[serde(flatten)]
    pub value: T,
}

impl<T> InWorkspace<T> {
    pub fn new(workspace: impl Into<String>, value: T) -> Self {
        Self {
            workspace: workspace.into(),
            value,
        }
    }

    /// Tag every item of a list.
    pub fn tag_all(workspace: &str, values: Vec<T>) -> Vec<InWorkspace<T>> {
        values
            .into_iter()
            .map(|v| InWorkspace::new(workspace, v))
            .collect()
    }
}
