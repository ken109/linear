//! Output conventions and exit codes that apply to every command.

mod common;
use common::*;

#[test]
fn usage_errors_exit_2_and_honour_json() {
    let sb = Sandbox::new();
    let o = sb.run(&["workspace", "bogus", "--json"], None, &[]);
    assert_eq!(code(&o), 2);
    assert_eq!(stdout(&o), "");
    let e: serde_json::Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(e["error"]["code"], "usage");
    assert!(e["error"]["message"].is_string());

    let o = sb.run(&["workspace", "bogus"], None, &[]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("bogus"));
}

#[test]
fn help_and_version_exit_0() {
    let sb = Sandbox::new();
    let o = sb.run(&["--help"], None, &[]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).contains("workspace"));
    let o = sb.run(&["--version"], None, &[]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).starts_with("linear "));
}

#[test]
fn errors_go_to_stderr_not_stdout() {
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.config_dir()).unwrap();
    std::fs::write(sb.config_dir().join("workspaces.toml"), "bogus = 1\n").unwrap();

    let o = sb.run(&["workspace", "list"], None, &[]);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");
    assert!(stderr(&o).starts_with("error: "));

    // With --json the error is an object on stderr, and stdout stays empty.
    let o = sb.run(&["workspace", "list", "--json"], None, &[]);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");
    let e: serde_json::Value = serde_json::from_str(stderr(&o).trim()).unwrap();
    assert_eq!(e["error"]["code"], "error");
}
