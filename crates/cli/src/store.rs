//! Reading and writing configuration and credentials on disk.

use crate::error::{CliError, Result};
use linear_core::config::{validate_workspace_name, Config};
use std::fs;
use std::path::{Path, PathBuf};

/// Where `linear` keeps its configuration.
#[derive(Debug, Clone)]
pub struct Dirs {
    root: PathBuf,
}

impl Dirs {
    /// `$LINEAR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/linear`, else `~/.config/linear`.
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let root = if let Some(d) = var("LINEAR_CONFIG_DIR") {
            d
        } else if let Some(x) = var("XDG_CONFIG_HOME") {
            x.join("linear")
        } else if let Some(h) = var("HOME") {
            h.join(".config").join("linear")
        } else {
            return Err(CliError::general(
                "cannot locate the config directory: set HOME or LINEAR_CONFIG_DIR",
            ));
        };
        Ok(Self { root })
    }

    pub fn workspaces_file(&self) -> PathBuf {
        self.root.join("workspaces.toml")
    }

    pub fn credentials_file(&self, workspace: &str) -> Result<PathBuf> {
        // The name becomes a file name: never let it escape the directory.
        validate_workspace_name(workspace)?;
        Ok(self
            .root
            .join("credentials")
            .join(format!("{workspace}.json")))
    }
}

/// Read `workspaces.toml`. A missing file is an empty config.
pub fn read_config(dirs: &Dirs) -> Result<Config> {
    let path = dirs.workspaces_file();
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Config::parse(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(CliError::general(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

/// The contents of the nearest `.linear.toml`, searching from `start` upwards.
pub fn find_repo_file(start: &Path) -> Option<String> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if let Ok(text) = fs::read_to_string(d.join(".linear.toml")) {
            return Some(text);
        }
        dir = d.parent();
    }
    None
}
