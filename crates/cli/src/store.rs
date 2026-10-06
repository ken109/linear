//! Reading and writing configuration and credentials on disk.

use crate::error::{CliError, Result};
use linear_core::auth::{api_key_env_var, Credential, Secret};
use linear_core::config::{validate_workspace_name, Config};
use std::fs;
use std::io::Write;
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

/// Where a credential came from. Never contains the secret itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// An environment variable (its name).
    Env(String),
    /// A credentials file (its path).
    File(PathBuf),
}

impl CredentialSource {
    pub fn describe(&self) -> String {
        match self {
            Self::Env(name) => format!("environment variable {name}"),
            Self::File(p) => format!("file {}", p.display()),
        }
    }
}

/// Find the credential for a workspace: the environment override first, then the file.
pub fn load_credential(
    dirs: &Dirs,
    workspace: &str,
) -> Result<Option<(Credential, CredentialSource)>> {
    let var = api_key_env_var(workspace);
    if let Some(value) = std::env::var(&var).ok().filter(|v| !v.trim().is_empty()) {
        return Ok(Some((
            Credential::ApiKey {
                api_key: Secret::new(value.trim()),
            },
            CredentialSource::Env(var),
        )));
    }

    let path = dirs.credentials_file(workspace)?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(CliError::general(format!(
                "cannot read {}: {e}",
                path.display()
            )))
        }
    };
    check_private(&path)?;
    // Do not echo the parser's message: it may quote file contents.
    let cred: Credential = serde_json::from_str(&text).map_err(|_| {
        CliError::auth(format!(
            "{} is not a valid credentials file; run `linear workspace login {workspace}` again",
            path.display()
        ))
    })?;
    Ok(Some((cred, CredentialSource::File(path))))
}

/// Refuse a credentials file that other users can read.
#[cfg(unix)]
fn check_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(CliError::auth(format!(
            "{} is readable by other users (mode {:o}); run `chmod 600` on it",
            path.display(),
            mode & 0o777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private(_path: &Path) -> Result<()> {
    Ok(())
}

/// Write a credential atomically with mode 0600 (in a 0700 directory).
pub fn save_credential(dirs: &Dirs, workspace: &str, cred: &Credential) -> Result<PathBuf> {
    let path = dirs.credentials_file(workspace)?;
    let dir = path.parent().expect("credentials file has a parent");
    create_private_dir(dir)?;
    let body = serde_json::to_string_pretty(cred).expect("credential serializes");
    write_atomic(&path, body.as_bytes(), 0o600)?;
    Ok(path)
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    Ok(())
}

/// Write via a temporary file in the same directory, then rename into place.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let dir = path.parent().expect("path has a parent");
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}
