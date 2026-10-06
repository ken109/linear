//! Workspace configuration (`workspaces.toml`) and workspace resolution.
//!
//! Parsing and resolution are pure; reading files and environment variables is
//! the CLI's job.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// How a workspace authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    /// A personal API key. The default.
    #[default]
    ApiKey,
    /// OAuth 2.0 with PKCE.
    Oauth,
}

impl fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ApiKey => "api-key",
            Self::Oauth => "oauth",
        })
    }
}

/// A validator rule that can be enabled per workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    TemplateSections,
    SourceAttachment,
    LabelGroupsExclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// The workspace's URL key (`linear.app/<url_key>`). Credentials are
    /// checked against it, so a key for the wrong workspace is caught.
    pub url_key: String,
    /// Default team key for commands that need one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_team: Option<String>,
    #[serde(default)]
    pub auth: AuthMethod,
    /// Validator rules to enforce on writes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Allow `linear api --mutation` (ownership rules and validators do not apply to it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_raw_mutation: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The workspace to use when nothing else selects one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub workspaces: BTreeMap<String, WorkspaceConfig>,
}

/// A workspace name becomes a file name, so it is restricted.
pub fn validate_workspace_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !name.starts_with(['-', '_']);
    if ok {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "invalid workspace name {name:?}: use lowercase letters, digits, '-' and '_' (at most 64, not starting with '-' or '_')"
        )))
    }
}

impl Config {
    /// Parse and validate `workspaces.toml`. Unknown keys and unknown rule
    /// names are errors, never silently ignored.
    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Config = toml_edit::de::from_str(text)
            .map_err(|e| Error::Config(format!("workspaces.toml: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        for name in self.workspaces.keys() {
            validate_workspace_name(name)?;
        }
        if let Some(default) = &self.default {
            if !self.workspaces.contains_key(default) {
                return Err(Error::Config(format!(
                    "default workspace {default:?} is not defined under [workspaces]"
                )));
            }
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&WorkspaceConfig> {
        self.workspaces.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.workspaces.keys().map(String::as_str).collect()
    }
}

/// The per-repository `.linear.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub workspace: String,
}

impl RepoConfig {
    pub fn parse(text: &str) -> Result<Self> {
        toml_edit::de::from_str(text).map_err(|e| Error::Config(format!(".linear.toml: {e}")))
    }
}

/// Where the selected workspace came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Flag,
    Env,
    RepoFile,
    ConfigDefault,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Flag => "--workspace",
            Self::Env => "LINEAR_WORKSPACE",
            Self::RepoFile => ".linear.toml",
            Self::ConfigDefault => "default in workspaces.toml",
        }
    }
}

/// A selected workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    pub name: String,
    pub source: Source,
    pub config: &'a WorkspaceConfig,
}

/// Everything that can select a workspace, in precedence order.
#[derive(Debug, Clone, Copy, Default)]
pub struct Selectors<'a> {
    /// `--workspace`
    pub flag: Option<&'a str>,
    /// `LINEAR_WORKSPACE`
    pub env: Option<&'a str>,
    /// The contents of the nearest `.linear.toml`, if any.
    pub repo_file: Option<&'a str>,
}

/// Pick the workspace: `--workspace` > `LINEAR_WORKSPACE` > `.linear.toml` > config default.
/// Empty values count as unset.
pub fn resolve<'a>(sel: Selectors<'_>, config: &'a Config) -> Result<Resolved<'a>> {
    let non_empty = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let (name, source) = if let Some(n) = non_empty(sel.flag) {
        (n, Source::Flag)
    } else if let Some(n) = non_empty(sel.env) {
        (n, Source::Env)
    } else if let Some(text) = sel.repo_file {
        (RepoConfig::parse(text)?.workspace, Source::RepoFile)
    } else if let Some(n) = config.default.clone() {
        (n, Source::ConfigDefault)
    } else {
        return Err(Error::Usage(match config.names().as_slice() {
            [] => "no workspace is configured; run `linear workspace add <name> --url-key <key>`"
                .to_owned(),
            names => format!(
                "no workspace selected; pass --workspace, set LINEAR_WORKSPACE, or add a default (known: {})",
                names.join(", ")
            ),
        }));
    };

    match config.get(&name) {
        Some(ws) => Ok(Resolved {
            name,
            source,
            config: ws,
        }),
        None => Err(Error::Usage(format!(
            "unknown workspace {name:?} (from {}); known workspaces: {}",
            source.describe(),
            if config.workspaces.is_empty() {
                "none".to_owned()
            } else {
                config.names().join(", ")
            }
        ))),
    }
}
