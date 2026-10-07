//! The OS keyring, behind a trait so everything that uses it can be tested with a fake.
//!
//! A keyring entry holds exactly what the credentials file holds (the JSON of a
//! `Credential`), under the service [`SERVICE`] and the workspace name as the account.
//! Nothing here knows about workspaces or credentials: it stores strings.

use std::fmt;
use std::path::PathBuf;

/// The keyring service every entry is stored under.
pub const SERVICE: &str = "linear-cli";

/// `off` (or `0`, `false`, `no`) never touches the OS keyring: it is treated as unavailable.
/// For machines that share a configuration with a keyring but have none (WSL, CI).
pub const KEYRING_ENV: &str = "LINEAR_KEYRING";
/// Keep the entries as files in this directory instead of in the OS keyring. For tests:
/// they run the real binary and must not touch the keyring of the machine they run on.
pub const KEYRING_DIR_ENV: &str = "LINEAR_KEYRING_DIR";

#[derive(Debug, PartialEq, Eq)]
pub enum KeyringError {
    /// There is no keyring to use here: none in this build, no Secret Service
    /// running, it is locked or access was refused. The caller falls back to the file.
    Unavailable(String),
    /// The keyring answered with an error that a fallback would only hide.
    Failed(String),
}

impl fmt::Display for KeyringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

pub trait Keyring: Send + Sync {
    /// The value stored for `account`; `None` when there is no entry.
    fn get(&self, account: &str) -> Result<Option<String>, KeyringError>;
    fn set(&self, account: &str, value: &str) -> Result<(), KeyringError>;
    /// Remove the entry; whether there was one.
    fn delete(&self, account: &str) -> Result<bool, KeyringError>;
}

/// The keyring to use for this run.
pub fn from_env() -> Box<dyn Keyring> {
    if let Some(dir) = std::env::var_os(KEYRING_DIR_ENV).filter(|v| !v.is_empty()) {
        return Box::new(DirKeyring { dir: dir.into() });
    }
    if std::env::var(KEYRING_ENV).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        )
    }) {
        return Box::new(Unavailable(format!("disabled by {KEYRING_ENV}=off")));
    }
    os()
}

/// A keyring that is not there.
pub struct Unavailable(pub String);

impl Keyring for Unavailable {
    fn get(&self, _: &str) -> Result<Option<String>, KeyringError> {
        Err(KeyringError::Unavailable(self.0.clone()))
    }
    fn set(&self, _: &str, _: &str) -> Result<(), KeyringError> {
        Err(KeyringError::Unavailable(self.0.clone()))
    }
    fn delete(&self, _: &str) -> Result<bool, KeyringError> {
        Err(KeyringError::Unavailable(self.0.clone()))
    }
}

/// Entries as files in a directory (see [`KEYRING_DIR_ENV`]).
struct DirKeyring {
    dir: PathBuf,
}

impl DirKeyring {
    fn path(&self, account: &str) -> PathBuf {
        self.dir.join(account)
    }
}

impl Keyring for DirKeyring {
    fn get(&self, account: &str) -> Result<Option<String>, KeyringError> {
        match std::fs::read_to_string(self.path(account)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeyringError::Failed(e.to_string())),
        }
    }
    fn set(&self, account: &str, value: &str) -> Result<(), KeyringError> {
        let fail = |e: crate::error::CliError| KeyringError::Failed(e.message);
        crate::store::create_private_dir(&self.dir).map_err(fail)?;
        crate::store::write_atomic(&self.path(account), value.as_bytes(), 0o600).map_err(fail)
    }
    fn delete(&self, account: &str) -> Result<bool, KeyringError> {
        match std::fs::remove_file(self.path(account)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(KeyringError::Failed(e.to_string())),
        }
    }
}

// ------------------------------------------------------------------ the OS keyring

/// The keyring of this machine: the Keychain on macOS, libsecret (through `secret-tool`)
/// on Linux, and none elsewhere or in a build without the `keyring` feature.
fn os() -> Box<dyn Keyring> {
    #[cfg(all(feature = "keyring", target_os = "macos"))]
    return Box::new(Keychain);
    #[cfg(all(feature = "keyring", target_os = "linux"))]
    return Box::new(SecretTool::from_env());
    #[cfg(not(all(feature = "keyring", any(target_os = "macos", target_os = "linux"))))]
    return Box::new(Unavailable(
        "this build of linear has no OS keyring support".to_owned(),
    ));
}

#[cfg(all(feature = "keyring", target_os = "macos"))]
struct Keychain;

#[cfg(all(feature = "keyring", target_os = "macos"))]
impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry, KeyringError> {
        keyring::Entry::new(SERVICE, account).map_err(map_error)
    }
}

#[cfg(all(feature = "keyring", target_os = "macos"))]
impl Keyring for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>, KeyringError> {
        match Self::entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }
    fn set(&self, account: &str, value: &str) -> Result<(), KeyringError> {
        Self::entry(account)?.set_password(value).map_err(map_error)
    }
    fn delete(&self, account: &str) -> Result<bool, KeyringError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(map_error(e)),
        }
    }
}

/// A missing or unreachable keyring is a reason to fall back; anything else is reported.
/// The message is the library's own: it does not contain the stored value.
#[cfg(all(feature = "keyring", target_os = "macos"))]
fn map_error(e: keyring::Error) -> KeyringError {
    let message: String = e.to_string().chars().take(200).collect();
    match e {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            KeyringError::Unavailable(message)
        }
        _ => KeyringError::Failed(message),
    }
}

/// libsecret's `secret-tool` (package `libsecret-tools`). The value goes in and out through
/// standard input and output, never the command line, where other users could read it.
/// A machine without it, or without a Secret Service to talk to (WSL, a CI runner, no
/// session bus), is a machine without a keyring.
#[cfg(unix)]
pub struct SecretTool {
    program: std::ffi::OsString,
}

/// Overrides the program run as `secret-tool` (for tests).
#[cfg(unix)]
pub const SECRET_TOOL_ENV: &str = "LINEAR_SECRET_TOOL";

#[cfg(unix)]
impl SecretTool {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os(SECRET_TOOL_ENV)
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "secret-tool".into()),
        )
    }

    pub fn new(program: impl Into<std::ffi::OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Run it with `stdin`; the exit status, standard output and standard error.
    fn run(
        &self,
        args: &[&str],
        stdin: Option<&str>,
    ) -> Result<(Option<i32>, String, String), KeyringError> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                KeyringError::Unavailable(if e.kind() == std::io::ErrorKind::NotFound {
                    "secret-tool is not installed (libsecret-tools)".to_owned()
                } else {
                    format!("cannot run secret-tool: {e}")
                })
            })?;
        let mut pipe = child.stdin.take().expect("stdin is piped");
        if let Some(text) = stdin {
            // A tool that exits early closes the pipe: its status says why.
            let _ = pipe.write_all(text.as_bytes());
        }
        drop(pipe);
        let out = child
            .wait_with_output()
            .map_err(|e| KeyringError::Failed(format!("secret-tool failed: {e}")))?;
        Ok((
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    fn attributes(account: &str) -> [&str; 4] {
        ["service", SERVICE, "account", account]
    }

    fn unavailable(stderr: &str) -> KeyringError {
        let why: String = stderr.trim().chars().take(200).collect();
        KeyringError::Unavailable(if why.is_empty() {
            "secret-tool failed".to_owned()
        } else {
            why
        })
    }
}

#[cfg(unix)]
impl Keyring for SecretTool {
    fn get(&self, account: &str) -> Result<Option<String>, KeyringError> {
        let mut args = vec!["lookup"];
        args.extend(Self::attributes(account));
        match self.run(&args, None)? {
            (Some(0), out, _) if !out.is_empty() => Ok(Some(out)),
            // Not found: it exits 1 and says nothing. Any other noise is a failure to ask.
            (Some(0) | Some(1), _, err) if err.trim().is_empty() => Ok(None),
            (_, _, err) => Err(Self::unavailable(&err)),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), KeyringError> {
        let label = format!("linear ({account})");
        let mut args = vec!["store", "--label", label.as_str()];
        args.extend(Self::attributes(account));
        match self.run(&args, Some(value))? {
            (Some(0), _, _) => Ok(()),
            (_, _, err) => Err(Self::unavailable(&err)),
        }
    }

    fn delete(&self, account: &str) -> Result<bool, KeyringError> {
        let existed = self.get(account)?.is_some();
        let mut args = vec!["clear"];
        args.extend(Self::attributes(account));
        match self.run(&args, None)? {
            (Some(0), _, _) => Ok(existed),
            (_, _, err) => Err(Self::unavailable(&err)),
        }
    }
}

// ------------------------------------------------------------------ a fake for tests

/// An in-memory keyring that can be made to fail.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Memory {
        pub entries: Mutex<BTreeMap<String, String>>,
        /// When set, every call fails with this error.
        pub broken: Mutex<Option<KeyringError>>,
    }

    impl Memory {
        pub fn unavailable() -> Self {
            Self {
                broken: Mutex::new(Some(KeyringError::Unavailable("no secret service".into()))),
                ..Self::default()
            }
        }

        pub fn get_now(&self, account: &str) -> Option<String> {
            self.entries.lock().unwrap().get(account).cloned()
        }

        fn check(&self) -> Result<(), KeyringError> {
            match &*self.broken.lock().unwrap() {
                Some(KeyringError::Unavailable(m)) => Err(KeyringError::Unavailable(m.clone())),
                Some(KeyringError::Failed(m)) => Err(KeyringError::Failed(m.clone())),
                None => Ok(()),
            }
        }
    }

    impl Keyring for Memory {
        fn get(&self, account: &str) -> Result<Option<String>, KeyringError> {
            self.check()?;
            Ok(self.get_now(account))
        }
        fn set(&self, account: &str, value: &str) -> Result<(), KeyringError> {
            self.check()?;
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<bool, KeyringError> {
            self.check()?;
            Ok(self.entries.lock().unwrap().remove(account).is_some())
        }
    }
}

#[cfg(all(test, unix))]
mod secret_tool_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A `secret-tool` that keeps entries as files in `dir`, like the real one's contract:
    /// `store` reads the value from stdin, `lookup` prints it (exit 1 and no output when
    /// there is none), `clear` removes it. A `broken` file in `dir` makes it fail with a message.
    fn fake_tool(dir: &std::path::Path) -> SecretTool {
        let script = dir.join("secret-tool");
        std::fs::write(
            &script,
            r#"#!/bin/sh
dir="$(dirname "$0")"
if [ -f "$dir/broken" ]; then echo "Cannot autolaunch D-Bus without X11 \$DISPLAY" >&2; exit 1; fi
cmd="$1"; shift
[ "$1" = "--label" ] && shift 2
service="$2"; account="$4"
[ "$service" = "linear-cli" ] || { echo "wrong service" >&2; exit 2; }
case "$cmd" in
  store) cat > "$dir/entry-$account" ;;
  lookup) [ -f "$dir/entry-$account" ] || exit 1; cat "$dir/entry-$account" ;;
  clear) rm -f "$dir/entry-$account" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        SecretTool::new(script)
    }

    #[test]
    fn secret_tool_stores_looks_up_and_clears_through_stdin_and_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = fake_tool(tmp.path());
        assert_eq!(tool.get("main").unwrap(), None);
        assert!(!tool.delete("main").unwrap());

        let value = "{\n  \"kind\": \"api-key\",\n  \"api_key\": \"k\"\n}";
        tool.set("main", value).unwrap();
        assert_eq!(tool.get("main").unwrap().as_deref(), Some(value));
        assert!(tool.delete("main").unwrap());
        assert_eq!(tool.get("main").unwrap(), None);
    }

    #[test]
    fn secret_tool_that_cannot_reach_a_secret_service_is_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = fake_tool(tmp.path());
        std::fs::write(tmp.path().join("broken"), "").unwrap();
        for result in [
            tool.get("main").map(|_| ()),
            tool.set("main", "v"),
            tool.delete("main").map(|_| ()),
        ] {
            match result {
                Err(KeyringError::Unavailable(why)) => assert!(why.contains("D-Bus"), "{why}"),
                other => panic!("expected Unavailable, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_missing_secret_tool_is_unavailable() {
        let tool = SecretTool::new("/nonexistent/secret-tool");
        match tool.get("main") {
            Err(KeyringError::Unavailable(why)) => assert!(why.contains("not installed"), "{why}"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }
}
