//! The cache on disk: one JSON file per workspace.
//!
//! `$LINEAR_CACHE_DIR`, else `$XDG_CACHE_HOME/linear`, else `~/.cache/linear`.
//! Writes go to a temporary file that is renamed into place, so a reader (a
//! statusline) never sees half an entry. What the cache holds and when it is
//! trusted is decided in `linear_core::cache`.

use crate::error::{CliError, Result};
use crate::store::{create_private_dir, write_atomic};
use linear_core::cache::{WorkspaceCache, SCHEMA_VERSION};
use linear_core::config::validate_workspace_name;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// A refresh that has run longer than this is taken to have died.
const LOCK_STALE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct CacheDir {
    root: PathBuf,
}

/// What reading one workspace's entry found.
#[derive(Debug)]
pub enum Loaded {
    /// There is no entry.
    Missing,
    /// There is a file that cannot be used: another schema version, or not
    /// valid. The reason is for people. A refresh replaces it.
    Unusable(String),
    Found(Box<WorkspaceCache>),
}

impl Loaded {
    /// The entry, if there is a usable one.
    pub fn into_entry(self) -> Option<WorkspaceCache> {
        match self {
            Self::Found(c) => Some(*c),
            _ => None,
        }
    }
}

impl CacheDir {
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let root = if let Some(d) = var("LINEAR_CACHE_DIR") {
            d
        } else if let Some(x) = var("XDG_CACHE_HOME") {
            x.join("linear")
        } else if let Some(h) = var("HOME") {
            h.join(".cache").join("linear")
        } else {
            return Err(CliError::general(
                "cannot locate the cache directory: set HOME or LINEAR_CACHE_DIR",
            ));
        };
        Ok(Self { root })
    }

    fn file(&self, workspace: &str) -> Result<PathBuf> {
        // The name becomes a file name: never let it escape the directory.
        validate_workspace_name(workspace)?;
        Ok(self.root.join(format!("{workspace}.json")))
    }

    pub fn load(&self, workspace: &str) -> Result<Loaded> {
        let path = self.file(workspace)?;
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::Missing),
            Err(e) => {
                return Ok(Loaded::Unusable(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return Ok(Loaded::Unusable(format!("not valid JSON: {e}"))),
        };
        // Another version may have written a shape this one cannot decode, so
        // the version is read before the rest.
        match value.get("schemaVersion").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(SCHEMA_VERSION) => {}
            Some(v) => {
                return Ok(Loaded::Unusable(format!(
                    "written with schemaVersion {v}; this build reads {SCHEMA_VERSION}"
                )))
            }
            // Entries written before schema version 3 spelled every key in
            // snake_case. Whatever number they carry, they are an older shape.
            None => match value.get("schema_version").and_then(|v| v.as_u64()) {
                Some(v) => {
                    return Ok(Loaded::Unusable(format!(
                        "written with schema_version {v}; this build reads {SCHEMA_VERSION}"
                    )))
                }
                None => return Ok(Loaded::Unusable("it has no schemaVersion".into())),
            },
        }
        match serde_json::from_value::<WorkspaceCache>(value) {
            Ok(c) => Ok(Loaded::Found(Box::new(c))),
            Err(e) => Ok(Loaded::Unusable(format!("it does not fit the schema: {e}"))),
        }
    }

    /// Write an entry atomically (mode 0600, in a 0700 directory: it holds
    /// issue text).
    pub fn save(&self, entry: &WorkspaceCache) -> Result<PathBuf> {
        let path = self.file(&entry.workspace)?;
        create_private_dir(&self.root)?;
        let body = serde_json::to_string_pretty(entry).expect("a cache entry serializes");
        write_atomic(&path, body.as_bytes(), 0o600)?;
        Ok(path)
    }

    /// Remove one workspace's entry. `false` when there was none.
    pub fn clear(&self, workspace: &str) -> Result<bool> {
        let path = self.file(workspace)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CliError::general(format!(
                "cannot remove {}: {e}",
                path.display()
            ))),
        }
    }

    /// Remove every entry, including those of workspaces no longer
    /// configured. Returns the workspace names that had one.
    pub fn clear_all(&self) -> Result<Vec<String>> {
        let read = match fs::read_dir(&self.root) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(CliError::general(format!(
                    "cannot read {}: {e}",
                    self.root.display()
                )))
            }
        };
        let mut names = Vec::new();
        for entry in read {
            let path = entry?.path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            // Only files this tool could have written.
            if validate_workspace_name(name).is_err() {
                continue;
            }
            if self.clear(name)? {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Take the right to refresh a workspace, so that a statusline that starts
    /// a refresh at every render does not start a dozen at once. `None` when
    /// another refresh holds it. A lock left by a refresh that died expires.
    pub fn lock_refresh(&self, workspace: &str) -> Result<Option<RefreshLock>> {
        validate_workspace_name(workspace)?;
        create_private_dir(&self.root)?;
        let path = self.root.join(format!("{workspace}.lock"));
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Some(RefreshLock { path })),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok());
                    if age.is_some_and(|a| a > LOCK_STALE) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    return Ok(None);
                }
                Err(e) => {
                    return Err(CliError::general(format!(
                        "cannot lock {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        Ok(None)
    }
}

/// Held while a workspace is being refreshed; released on drop.
#[derive(Debug)]
pub struct RefreshLock {
    path: PathBuf,
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
