//! Reading and writing configuration and credentials on disk.

use crate::error::{CliError, Result};
use crate::keystore::{self, Keyring, KeyringError};
use linear_core::auth::{
    api_key_env_var, client_id_env_var, client_secret_env_var, Credential, Secret, CLIENT_ID_ENV,
    CLIENT_SECRET_ENV,
};
use linear_core::config::{validate_workspace_name, Config, CredentialStore, WorkspaceConfig};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where `linear` keeps its configuration.
#[derive(Debug, Clone)]
pub struct Dirs {
    root: PathBuf,
}

impl Dirs {
    /// `$LINEAR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/linear`, else `~/.config/linear`
    /// (on Windows, `%APPDATA%\linear`).
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
        } else if let Some(d) = default_config_dir(var) {
            d
        } else {
            return Err(CliError::general(format!(
                "cannot locate the config directory: set {} or LINEAR_CONFIG_DIR",
                if cfg!(windows) { "APPDATA" } else { "HOME" }
            )));
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

/// Where the configuration lives when no variable says: `~/.config/linear`.
#[cfg(not(windows))]
fn default_config_dir(var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    var("HOME").map(|h| h.join(".config").join("linear"))
}

/// Where the configuration lives when no variable says: `%APPDATA%\linear`,
/// the per-user folder Windows programs keep their settings in.
#[cfg(windows)]
fn default_config_dir(var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    var("APPDATA").map(|d| d.join("linear"))
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
    /// The OS keyring (the account, which is the workspace name).
    Keyring(String),
}

impl CredentialSource {
    pub fn describe(&self) -> String {
        match self {
            Self::Env(name) => format!("environment variable {name}"),
            Self::File(p) => format!("file {}", p.display()),
            Self::Keyring(account) => format!(
                "the OS keyring (service {}, account {account})",
                keystore::SERVICE
            ),
        }
    }

    /// `"env"`, `"file"` or `"keyring"`, as `workspace list` and `whoami` print it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Env(_) => "env",
            Self::File(_) => "file",
            Self::Keyring(_) => "keyring",
        }
    }
}

/// Overrides `credential_store` of every workspace: `file` or `keyring`.
pub const CREDENTIAL_STORE_ENV: &str = "LINEAR_CREDENTIAL_STORE";

/// Where a workspace keeps its credential: `LINEAR_CREDENTIAL_STORE`, else the
/// workspace's `credential_store`, else the file.
pub fn credential_store(config: Option<&WorkspaceConfig>) -> Result<CredentialStore> {
    match std::env::var(CREDENTIAL_STORE_ENV) {
        Ok(v) if !v.trim().is_empty() => match v.trim() {
            "file" => Ok(CredentialStore::File),
            "keyring" => Ok(CredentialStore::Keyring),
            other => Err(CliError::usage(format!(
                "{CREDENTIAL_STORE_ENV} must be \"file\" or \"keyring\", got {other:?}"
            ))),
        },
        _ => Ok(config.map(|c| c.credential_store).unwrap_or_default()),
    }
}

/// Find the credential for a workspace: the environment override first, then the
/// store the workspace is set to use (the OS keyring), then the file. A workspace
/// set to the keyring still reads a credentials file: from before it was moved,
/// or because this machine has no keyring (WSL, CI).
pub fn load_credential(
    dirs: &Dirs,
    keyring: &dyn Keyring,
    workspace: &str,
    store: CredentialStore,
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

    load_stored_credential(dirs, keyring, workspace, store)
}

/// The stored credential only (the keyring, then the file), whatever the environment says.
/// An OAuth workspace uses this: an API key in the environment is not its credential.
pub fn load_stored_credential(
    dirs: &Dirs,
    keyring: &dyn Keyring,
    workspace: &str,
    store: CredentialStore,
) -> Result<Option<(Credential, CredentialSource)>> {
    let mut keyring_unavailable = None;
    if store == CredentialStore::Keyring {
        validate_workspace_name(workspace)?;
        match keyring.get(workspace) {
            Ok(Some(text)) => {
                let cred = parse_credential(&text, "the OS keyring entry", workspace)?;
                return Ok(Some((
                    cred,
                    CredentialSource::Keyring(workspace.to_owned()),
                )));
            }
            Ok(None) => {}
            Err(KeyringError::Unavailable(why)) => keyring_unavailable = Some(why),
            Err(KeyringError::Failed(why)) => {
                return Err(CliError::auth(format!("cannot read the OS keyring: {why}")))
            }
        }
    }

    match read_file_credential(dirs, workspace)? {
        Some((cred, path)) => Ok(Some((cred, CredentialSource::File(path)))),
        None => match keyring_unavailable {
            Some(why) => Err(CliError::auth(format!(
                "the OS keyring is not available ({why}) and there is no credentials file for \
                 workspace {workspace:?}; run `linear workspace login {workspace}`"
            ))),
            None => Ok(None),
        },
    }
}

/// The credentials file of a workspace, if there is one.
fn read_file_credential(dirs: &Dirs, workspace: &str) -> Result<Option<(Credential, PathBuf)>> {
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
    let cred = parse_credential(&text, &path.display().to_string(), workspace)?;
    Ok(Some((cred, path)))
}

/// Do not echo the parser's message: it may quote the contents.
fn parse_credential(text: &str, origin: &str, workspace: &str) -> Result<Credential> {
    serde_json::from_str(text).map_err(|_| {
        CliError::auth(format!(
            "{origin} is not a valid credential; run `linear workspace login {workspace}` again"
        ))
    })
}

/// The first of these environment variables that is set to something.
fn first_env(names: &[String]) -> Option<(String, String)> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .map(|v| (name.clone(), v))
    })
}

/// The OAuth app's client id of a workspace: `LINEAR_CLIENT_ID_<NAME>`,
/// `LINEAR_CLIENT_ID`, else `client_id` in the workspace's config.
pub fn client_id(workspace: &str, config: &WorkspaceConfig) -> Option<String> {
    first_env(&[client_id_env_var(workspace), CLIENT_ID_ENV.to_owned()])
        .map(|(_, v)| v)
        .or_else(|| {
            config
                .client_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
}

/// The error for an OAuth app whose client id is not set anywhere.
pub fn missing_client_id(workspace: &str) -> CliError {
    CliError::auth(format!(
        "no client id for workspace {workspace:?}: set `client_id` under \
         [workspaces.{workspace}] in workspaces.toml, or {CLIENT_ID_ENV}"
    ))
}

/// The client credentials of a workspace with `auth = "client_credentials"`:
/// the secret from `LINEAR_CLIENT_SECRET_<NAME>` or `LINEAR_CLIENT_SECRET`, the
/// id from [`client_id`]. Nothing is read from or written to a file. `None` when
/// there is no secret; an error when there is a secret but no id.
pub fn load_client_credentials(
    workspace: &str,
    config: &WorkspaceConfig,
) -> Result<Option<(Credential, CredentialSource)>> {
    let Some((secret_var, secret)) = first_env(&[
        client_secret_env_var(workspace),
        CLIENT_SECRET_ENV.to_owned(),
    ]) else {
        return Ok(None);
    };
    let id = client_id(workspace, config).ok_or_else(|| missing_client_id(workspace))?;
    Ok(Some((
        Credential::ClientCredentials {
            client_id: id,
            client_secret: Secret::new(secret),
        },
        CredentialSource::Env(secret_var),
    )))
}

/// Whether the client secret of a workspace is set (not its value).
pub fn client_secret_is_set(workspace: &str) -> bool {
    first_env(&[
        client_secret_env_var(workspace),
        CLIENT_SECRET_ENV.to_owned(),
    ])
    .is_some()
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
fn save_file(dirs: &Dirs, workspace: &str, cred: &Credential) -> Result<PathBuf> {
    let path = dirs.credentials_file(workspace)?;
    let dir = path.parent().expect("credentials file has a parent");
    create_private_dir(dir)?;
    write_atomic(&path, credential_json(cred).as_bytes(), 0o600)?;
    Ok(path)
}

fn credential_json(cred: &Credential) -> String {
    serde_json::to_string_pretty(cred).expect("credential serializes")
}

/// Remove the credentials file; whether there was one.
fn remove_file_credential(dirs: &Dirs, workspace: &str) -> Result<bool> {
    let path = dirs.credentials_file(workspace)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(CliError::general(format!(
            "cannot remove {}: {e}",
            path.display()
        ))),
    }
}

/// Where a credential was stored.
#[derive(Debug)]
pub struct Saved {
    pub source: CredentialSource,
    /// Why the OS keyring was not used, when `Keyring` was asked for and the
    /// credential went to the file instead.
    pub fallback: Option<String>,
}

/// Store a credential where the workspace keeps them. With the keyring, the file
/// of an earlier login is removed (nothing stays behind in plain text), and a
/// machine without a keyring gets the file instead (see [`Saved::fallback`]).
pub fn save_credential(
    dirs: &Dirs,
    keyring: &dyn Keyring,
    workspace: &str,
    cred: &Credential,
    store: CredentialStore,
) -> Result<Saved> {
    validate_workspace_name(workspace)?;
    let mut fallback = None;
    if store == CredentialStore::Keyring {
        match keyring.set(workspace, &credential_json(cred)) {
            Ok(()) => {
                remove_file_credential(dirs, workspace)?;
                return Ok(Saved {
                    source: CredentialSource::Keyring(workspace.to_owned()),
                    fallback: None,
                });
            }
            Err(KeyringError::Unavailable(why)) => fallback = Some(why),
            Err(KeyringError::Failed(why)) => {
                return Err(CliError::general(format!(
                    "cannot write to the OS keyring: {why}"
                )))
            }
        }
    }
    let path = save_file(dirs, workspace, cred)?;
    Ok(Saved {
        source: CredentialSource::File(path),
        fallback,
    })
}

/// Write a credential back to where it was loaded from (a refreshed token).
/// An environment variable is never written to.
pub fn save_credential_to(
    dirs: &Dirs,
    keyring: &dyn Keyring,
    workspace: &str,
    cred: &Credential,
    source: &CredentialSource,
) -> Result<()> {
    match source {
        CredentialSource::File(_) => {
            save_file(dirs, workspace, cred)?;
        }
        CredentialSource::Keyring(_) => keyring
            .set(workspace, &credential_json(cred))
            .map_err(|e| CliError::general(format!("cannot write to the OS keyring: {e}")))?,
        CredentialSource::Env(_) => {}
    }
    Ok(())
}

/// What `migrate_credential` did.
#[derive(Debug)]
pub struct Migrated {
    pub from: CredentialSource,
    pub to: CredentialSource,
    /// A copy that could not be removed from the old place.
    pub left_behind: Option<String>,
}

/// Move a workspace's stored credential to `to`: write it there, read it back to
/// check it, and only then remove the old copy.
pub fn migrate_credential(
    dirs: &Dirs,
    keyring: &dyn Keyring,
    workspace: &str,
    to: CredentialStore,
) -> Result<Migrated> {
    validate_workspace_name(workspace)?;
    let keyring_error = |e: KeyringError| match e {
        KeyringError::Unavailable(why) => CliError::general(format!(
            "the OS keyring is not available ({why}); nothing was moved"
        )),
        KeyringError::Failed(why) => CliError::general(format!("the OS keyring failed: {why}")),
    };
    let nothing = |place: &str| {
        CliError::usage(format!(
            "workspace {workspace:?} has no credential in {place} to move"
        ))
    };
    let file_path = dirs.credentials_file(workspace)?;
    let keyring_source = CredentialSource::Keyring(workspace.to_owned());

    match to {
        CredentialStore::Keyring => {
            let (cred, path) = read_file_credential(dirs, workspace)?
                .ok_or_else(|| nothing("the credentials file"))?;
            let json = credential_json(&cred);
            keyring.set(workspace, &json).map_err(keyring_error)?;
            if keyring.get(workspace).map_err(keyring_error)?.as_deref() != Some(json.as_str()) {
                return Err(CliError::general(
                    "the OS keyring did not return what was written; the credentials file is untouched",
                ));
            }
            remove_file_credential(dirs, workspace)?;
            Ok(Migrated {
                from: CredentialSource::File(path),
                to: keyring_source,
                left_behind: None,
            })
        }
        CredentialStore::File => {
            let text = keyring
                .get(workspace)
                .map_err(keyring_error)?
                .ok_or_else(|| nothing("the OS keyring"))?;
            let cred = parse_credential(&text, "the OS keyring entry", workspace)?;
            save_file(dirs, workspace, &cred)?;
            let left_behind = keyring
                .delete(workspace)
                .err()
                .map(|e| format!("could not remove the OS keyring entry: {e}"));
            Ok(Migrated {
                from: keyring_source,
                to: CredentialSource::File(file_path),
                left_behind,
            })
        }
    }
}

#[cfg(unix)]
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::fake::Memory;

    fn key(value: &str) -> Credential {
        Credential::ApiKey {
            api_key: Secret::new(value),
        }
    }

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            root: root.to_owned(),
        }
    }

    // Workspace names are unique per test: `load_credential` reads `LINEAR_API_KEY_<NAME>`.

    #[test]
    fn a_file_workspace_never_touches_the_keyring() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let ring = Memory::default();
        let saved =
            save_credential(&d, &ring, "ks-file", &key("k1"), CredentialStore::File).unwrap();
        assert!(matches!(saved.source, CredentialSource::File(_)));
        assert!(saved.fallback.is_none());
        assert!(ring.entries.lock().unwrap().is_empty());

        // A broken keyring is not even asked.
        let broken = Memory::unavailable();
        let (cred, source) = load_credential(&d, &broken, "ks-file", CredentialStore::File)
            .unwrap()
            .unwrap();
        assert_eq!(cred, key("k1"));
        assert_eq!(source.kind(), "file");
    }

    #[test]
    fn the_keyring_holds_the_same_json_as_the_file_and_removes_an_old_file() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let ring = Memory::default();
        save_credential(&d, &ring, "ks-ring", &key("old"), CredentialStore::File).unwrap();
        let path = d.credentials_file("ks-ring").unwrap();
        assert!(path.is_file());

        let saved =
            save_credential(&d, &ring, "ks-ring", &key("new"), CredentialStore::Keyring).unwrap();
        assert_eq!(saved.source, CredentialSource::Keyring("ks-ring".into()));
        assert!(!path.exists(), "no plain-text copy is left behind");
        assert_eq!(
            ring.get_now("ks-ring").unwrap(),
            serde_json::to_string_pretty(&key("new")).unwrap()
        );

        let (cred, source) = load_credential(&d, &ring, "ks-ring", CredentialStore::Keyring)
            .unwrap()
            .unwrap();
        assert_eq!(cred, key("new"));
        assert_eq!(source.kind(), "keyring");
        assert!(source.describe().contains("account ks-ring"));
    }

    #[test]
    fn without_a_keyring_the_file_is_used_for_writing_and_reading() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let ring = Memory::unavailable();
        let saved =
            save_credential(&d, &ring, "ks-wsl", &key("k"), CredentialStore::Keyring).unwrap();
        assert_eq!(saved.source.kind(), "file");
        assert_eq!(saved.fallback.as_deref(), Some("no secret service"));

        let (cred, source) = load_credential(&d, &ring, "ks-wsl", CredentialStore::Keyring)
            .unwrap()
            .unwrap();
        assert_eq!(cred, key("k"));
        assert_eq!(source.kind(), "file");
    }

    #[test]
    fn a_keyring_workspace_still_reads_a_file_it_has_not_moved_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        save_credential(
            &d,
            &Memory::default(),
            "ks-old",
            &key("k"),
            CredentialStore::File,
        )
        .unwrap();
        let (cred, source) =
            load_credential(&d, &Memory::default(), "ks-old", CredentialStore::Keyring)
                .unwrap()
                .unwrap();
        assert_eq!(cred, key("k"));
        assert_eq!(source.kind(), "file");
    }

    #[test]
    fn no_credential_anywhere_is_none_and_an_unavailable_keyring_is_said() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        assert!(
            load_credential(&d, &Memory::default(), "ks-none", CredentialStore::Keyring)
                .unwrap()
                .is_none()
        );
        let err = load_credential(
            &d,
            &Memory::unavailable(),
            "ks-none",
            CredentialStore::Keyring,
        )
        .unwrap_err();
        assert!(err.message.contains("OS keyring is not available"), "{err}");
        assert!(
            err.message.contains("linear workspace login ks-none"),
            "{err}"
        );
    }

    #[test]
    fn a_failing_keyring_is_an_error_not_a_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let ring = Memory::default();
        *ring.broken.lock().unwrap() = Some(KeyringError::Failed("access denied".into()));
        save_credential(
            &d,
            &Memory::default(),
            "ks-denied",
            &key("k"),
            CredentialStore::File,
        )
        .unwrap();
        let err = load_credential(&d, &ring, "ks-denied", CredentialStore::Keyring).unwrap_err();
        assert!(err.message.contains("access denied"), "{err}");
        let err = save_credential(&d, &ring, "ks-denied", &key("k"), CredentialStore::Keyring)
            .unwrap_err();
        assert!(err.message.contains("access denied"), "{err}");
    }

    #[test]
    fn a_corrupt_keyring_entry_is_reported_without_quoting_it() {
        let tmp = tempfile::tempdir().unwrap();
        let ring = Memory::default();
        ring.set("ks-bad", "{\"api_key\": SECRET-LOOKING").unwrap();
        let err = load_credential(&dirs(tmp.path()), &ring, "ks-bad", CredentialStore::Keyring)
            .unwrap_err();
        assert!(err.message.contains("not a valid credential"), "{err}");
        assert!(!err.message.contains("SECRET-LOOKING"), "{err}");
    }

    #[test]
    fn migrating_moves_the_credential_and_checks_it_arrived() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        let ring = Memory::default();
        let cred = Credential::Oauth {
            access_token: Secret::new("a"),
            refresh_token: Some(Secret::new("r")),
            expires_at: None,
        };
        save_credential(&d, &ring, "ks-mig", &cred, CredentialStore::File).unwrap();
        let path = d.credentials_file("ks-mig").unwrap();

        let m = migrate_credential(&d, &ring, "ks-mig", CredentialStore::Keyring).unwrap();
        assert_eq!((m.from.kind(), m.to.kind()), ("file", "keyring"));
        assert!(!path.exists());
        assert!(ring.get_now("ks-mig").is_some());

        let m = migrate_credential(&d, &ring, "ks-mig", CredentialStore::File).unwrap();
        assert_eq!((m.from.kind(), m.to.kind()), ("keyring", "file"));
        assert!(m.left_behind.is_none());
        assert!(path.is_file());
        assert!(ring.get_now("ks-mig").is_none());
        let (back, _) = load_credential(&d, &ring, "ks-mig", CredentialStore::File)
            .unwrap()
            .unwrap();
        assert_eq!(back, cred);
    }

    #[test]
    fn migrating_without_a_keyring_or_without_a_credential_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dirs(tmp.path());
        save_credential(
            &d,
            &Memory::default(),
            "ks-keep",
            &key("k"),
            CredentialStore::File,
        )
        .unwrap();
        let path = d.credentials_file("ks-keep").unwrap();

        let err = migrate_credential(
            &d,
            &Memory::unavailable(),
            "ks-keep",
            CredentialStore::Keyring,
        )
        .unwrap_err();
        assert!(err.message.contains("nothing was moved"), "{err}");
        assert!(path.is_file());

        let err = migrate_credential(&d, &Memory::default(), "ks-empty", CredentialStore::Keyring)
            .unwrap_err();
        assert!(
            err.message
                .contains("no credential in the credentials file"),
            "{err}"
        );
        let err = migrate_credential(&d, &Memory::default(), "ks-keep", CredentialStore::File)
            .unwrap_err();
        assert!(
            err.message.contains("no credential in the OS keyring"),
            "{err}"
        );
    }
}
