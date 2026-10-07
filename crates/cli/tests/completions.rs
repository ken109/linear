//! `linear completions <shell>`: every shell gets a script, with nothing
//! configured. What the script names is checked against the command
//! definition by a unit test in `commands/completions.rs`.

mod common;

use common::*;

fn completions(shell: &str) -> std::process::Output {
    // An empty config directory and no credentials: nothing may be needed.
    Sandbox::new().run(&["completions", shell], None, &[])
}

#[test]
fn every_supported_shell_prints_a_script_that_names_the_binary() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let o = completions(shell);
        assert_eq!(code(&o), 0, "{shell}: {}", stderr(&o));
        let out = stdout(&o);
        assert!(out.contains("linear"), "{shell}: {out}");
        assert!(out.contains("issue"), "{shell}: the commands are in it");
        assert_eq!(stderr(&o), "", "{shell}");
    }
}

#[test]
fn the_scripts_are_what_each_shell_expects() {
    assert!(stdout(&completions("bash")).contains("complete -F _linear"));
    assert!(stdout(&completions("zsh")).starts_with("#compdef linear"));
    assert!(stdout(&completions("fish")).contains("complete -c linear"));
}

#[test]
fn an_unknown_shell_is_a_usage_error_that_lists_the_shells() {
    let o = completions("tcsh");
    assert_eq!(code(&o), 2);
    let err = stderr(&o);
    assert!(err.contains("bash") && err.contains("powershell"), "{err}");

    let o = Sandbox::new().run(&["completions"], None, &[]);
    assert_eq!(code(&o), 2);
}

#[test]
fn it_works_with_no_home_at_all() {
    // `Sandbox::run` sets HOME and LINEAR_CONFIG_DIR; run the binary bare.
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_linear"));
    cmd.args(["completions", "bash"]).env_clear();
    if let Some(root) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", root); // Windows
    }
    let o = cmd.output().expect("spawn");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!o.stdout.is_empty());
}
