//! Helpers for the read-command tests: a configured workspace, canned
//! responses from the core crate's fixtures, and request inspection.
#![allow(dead_code)]

use crate::common::*;
use std::process::Output;

pub const KEY: &str = "lin_api_READTEST_do_not_leak_0123456789";

/// A response body from `crates/core/tests/fixtures`.
pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../core/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// A sandbox with a workspace called `example` (the fixtures' workspace).
pub fn workspace() -> Sandbox {
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "example",
            "--url-key",
            "example",
            "--team",
            "EX",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    sb
}

/// Run `linear <args>` against the mock with the test key.
pub fn linear(sb: &Sandbox, mock: &Mock, args: &[&str]) -> Output {
    sb.run(args, Some(mock), &[("LINEAR_API_KEY_EXAMPLE", KEY)])
}

/// The JSON body of the `n`th request the mock saw.
pub fn request(mock: &Mock, n: usize) -> serde_json::Value {
    let seen = mock.requests();
    let body = &seen.get(n).unwrap_or_else(|| panic!("no request {n}")).body;
    serde_json::from_str(body).expect("the request body is JSON")
}

pub fn stdout_json(o: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", stdout(o)))
}

/// Nothing a command prints may contain the credential.
pub fn assert_no_leak(o: &Output) {
    assert!(!stdout(o).contains(KEY) && !stderr(o).contains(KEY));
}

/// The `ProjectRefs` response used to resolve `Fixture Project`.
pub const PROJECT_REFS: &str = r#"{"data":{"projects":{"nodes":[
    {"id":"00000000-0000-4000-8000-000000000006","slugId":"aaaaaaaaaaaa","name":"Fixture Project","url":"https://linear.app/example/project/fixture-project-aaaaaaaaaaaa"}
],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
