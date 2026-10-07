//! Workspace configuration (`workspaces.toml`) and workspace resolution.
//!
//! Parsing and resolution are pure; reading files and environment variables is
//! the CLI's job.

use crate::error::{Error, Result};
use crate::rules::Operation;
use schemars::JsonSchema;
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
    /// An app, by the OAuth client credentials grant: the client id and secret
    /// are exchanged for a token on every run. Meant for CI. Written
    /// `client_credentials` (`client-credentials` is accepted too).
    #[serde(rename = "client_credentials", alias = "client-credentials")]
    ClientCredentials,
}

impl fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ApiKey => "api-key",
            Self::Oauth => "oauth",
            Self::ClientCredentials => "client_credentials",
        })
    }
}

/// Where a workspace's stored credential lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialStore {
    /// `credentials/<workspace>.json`, mode 0600. The default.
    #[default]
    File,
    /// The OS keyring (macOS Keychain, the Secret Service on Linux). Falls back
    /// to the file where there is no keyring (WSL, CI).
    Keyring,
}

impl CredentialStore {
    pub fn is_file(&self) -> bool {
        *self == Self::File
    }
}

impl fmt::Display for CredentialStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::File => "file",
            Self::Keyring => "keyring",
        })
    }
}

/// How strictly the ownership rules (exit 4) apply in a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ownership {
    /// Writes need ownership: you lead the project, or the issue is yours.
    /// The default.
    #[default]
    Strict,
    /// For teams that work on each other's issues: creating an issue for
    /// somebody else and changing an issue owned by someone else are allowed.
    /// Writing projects, and canceling an issue that is not yours, stay refused.
    Lenient,
}

impl Ownership {
    pub fn is_strict(&self) -> bool {
        *self == Self::Strict
    }
}

impl fmt::Display for Ownership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Strict => "strict",
            Self::Lenient => "lenient",
        })
    }
}

/// A validator rule that can be enabled per workspace.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
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
    /// The OAuth app's client id, for `auth = "client_credentials"`. It is public,
    /// so it can live here; `LINEAR_CLIENT_ID` (or `LINEAR_CLIENT_ID_<NAME>`)
    /// overrides it. The client secret is never in this file: it comes from
    /// `LINEAR_CLIENT_SECRET` (or `LINEAR_CLIENT_SECRET_<NAME>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Where `workspace login` keeps the credential: `"file"` (the default) or
    /// `"keyring"` (the OS keyring, with the file as a fallback). Reading the
    /// environment override (`LINEAR_API_KEY_<NAME>`) never depends on it.
    #[serde(default, skip_serializing_if = "CredentialStore::is_file")]
    pub credential_store: CredentialStore,
    /// How strictly the ownership rules apply: `"strict"` (the default) or
    /// `"lenient"` (see [`Ownership`]).
    #[serde(default, skip_serializing_if = "Ownership::is_strict")]
    pub ownership: Ownership,
    /// Validator rules to enforce on writes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Narrow a rule to some of the operations it supports (by default a rule
    /// runs for all of them). Each key must be in `rules`, and each operation
    /// must be one the rule can apply to.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rule_operations: BTreeMap<Rule, Vec<Operation>>,
    /// The values `metadata.kind` of a source attachment may take, for the
    /// `source-attachment` rule. When set, an issue created with `--source`
    /// must also pass `--meta kind=<one of these>`, and `audit` flags issues
    /// whose source attachments have none. Unset: the kind is not checked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_kinds: Vec<String>,
    /// The title of a source attachment when `--source-title` is not given
    /// (`issue create --source`, and `issue update --source` for a URL the issue
    /// does not carry yet). Unset: `Source`. An attachment that already exists
    /// keeps its stored title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    /// Allow `linear api --mutation` (ownership rules and validators do not apply to it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_raw_mutation: bool,
    /// Thresholds for `linear audit`.
    #[serde(default, skip_serializing_if = "AuditSettings::is_default")]
    pub audit: AuditSettings,
}

/// `[workspaces.<name>.audit]`: thresholds for `linear audit`. What is left
/// out keeps the audit's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditSettings {
    /// Days without an update before an In Progress issue is stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_days: Option<u32>,
    /// Days before a project's latest status update counts as outdated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_update_days: Option<u32>,
    /// Days a GitHub pull request linked to an issue may stay open before the
    /// audit flags it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_open_days: Option<u32>,
}

impl AuditSettings {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
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

/// The title of a source attachment when neither `--source-title` nor the
/// workspace's `source_title` says otherwise.
pub const DEFAULT_SOURCE_TITLE: &str = "Source";

impl WorkspaceConfig {
    /// The title a new source attachment gets without `--source-title`.
    pub fn default_source_title(&self) -> &str {
        self.source_title
            .as_deref()
            .map(str::trim)
            .unwrap_or(DEFAULT_SOURCE_TITLE)
    }

    /// A threshold of zero days would flag everything at once.
    fn validate_audit(&self, workspace: &str) -> Result<()> {
        for (key, value) in [
            ("stale_days", self.audit.stale_days),
            ("status_update_days", self.audit.status_update_days),
            ("pr_open_days", self.audit.pr_open_days),
        ] {
            if value == Some(0) {
                return Err(Error::Config(format!(
                    "workspace {workspace:?}: audit.{key} must be at least 1"
                )));
            }
        }
        Ok(())
    }

    /// `client_id` belongs to the client credentials grant, and must not be blank.
    fn validate_client_id(&self, workspace: &str) -> Result<()> {
        match &self.client_id {
            None => Ok(()),
            Some(id) if id.trim().is_empty() => Err(Error::Config(format!(
                "workspace {workspace:?}: client_id must not be empty"
            ))),
            Some(_) if self.auth != AuthMethod::ClientCredentials => Err(Error::Config(format!(
                "workspace {workspace:?}: client_id needs auth = \"client_credentials\""
            ))),
            Some(_) => Ok(()),
        }
    }

    /// A blank default title would attach every source untitled.
    fn validate_source_title(&self, workspace: &str) -> Result<()> {
        match &self.source_title {
            Some(t) if t.trim().is_empty() => Err(Error::Config(format!(
                "workspace {workspace:?}: source_title must not be empty"
            ))),
            _ => Ok(()),
        }
    }

    /// `source_kinds` belongs to `source-attachment`: it needs the rule, and
    /// every entry must be a real kind (an empty list would refuse everything).
    fn validate_source_kinds(&self, workspace: &str) -> Result<()> {
        if self.source_kinds.is_empty() {
            return Ok(());
        }
        if !self.rules.contains(&Rule::SourceAttachment) {
            return Err(Error::Config(format!(
                "workspace {workspace:?}: source_kinds needs the source-attachment rule in rules"
            )));
        }
        if self.source_kinds.iter().any(|k| k.trim().is_empty()) {
            return Err(Error::Config(format!(
                "workspace {workspace:?}: source_kinds must not contain an empty kind"
            )));
        }
        Ok(())
    }

    /// `rule_operations` may only narrow enabled rules to operations they support.
    fn validate_rules(&self, workspace: &str) -> Result<()> {
        for (rule, ops) in &self.rule_operations {
            if !self.rules.contains(rule) {
                return Err(Error::Config(format!(
                    "workspace {workspace:?}: rule_operations names {rule}, which is not in rules"
                )));
            }
            if ops.is_empty() {
                return Err(Error::Config(format!(
                    "workspace {workspace:?}: rule_operations for {rule} is empty; remove the rule from rules instead"
                )));
            }
            for op in ops {
                if !rule.operations().contains(op) {
                    let supported: Vec<&str> =
                        rule.operations().iter().map(|o| o.as_str()).collect();
                    return Err(Error::Config(format!(
                        "workspace {workspace:?}: {rule} cannot apply to {op} (it applies to: {})",
                        supported.join(", ")
                    )));
                }
            }
        }
        Ok(())
    }
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
        for (name, ws) in &self.workspaces {
            validate_workspace_name(name)?;
            ws.validate_rules(name)?;
            ws.validate_audit(name)?;
            ws.validate_source_kinds(name)?;
            ws.validate_source_title(name)?;
            ws.validate_client_id(name)?;
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
