//! `linear api` against a real workspace. Ignored by default; see `live.rs`
//! for how to provide the sandbox credentials.

mod common;
use common::*;

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn api_query_and_mutation_refusal_against_the_sandbox_workspace() {
    let key = std::env::var("LINEAR_API_KEY_SANDBOX").expect("LINEAR_API_KEY_SANDBOX");
    let env = [("LINEAR_API_KEY_SANDBOX", key.as_str())];
    let sb = Sandbox::new();
    let o = sb.run(
        &[
            "workspace",
            "add",
            "sandbox",
            "--url-key",
            "ken109-sandbox",
            "--team",
            "SAND",
        ],
        None,
        &[],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    // A query with a variable. Assert non-empty values: an empty-but-successful
    // response must not pass.
    let o = sb.run(
        &[
            "api",
            "query Teams($first: Int!) { organization { urlKey } teams(first: $first) { nodes { key } } }",
            "--var-json",
            "first=5",
            "--json",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["organization"]["urlKey"], "ken109-sandbox");
    let keys: Vec<&str> = v["teams"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["key"].as_str())
        .collect();
    assert!(keys.contains(&"SAND"), "teams: {keys:?}");
    assert!(!stdout(&o).contains(&key) && !stderr(&o).contains(&key));

    // A server-side error surfaces as a failure, not as empty output.
    let o = sb.run(&["api", "{ noSuchField }"], None, &env);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");

    // A mutation is refused locally (exit 4).
    let o = sb.run(
        &[
            "api",
            "mutation { issueDelete(id: \"SAND-1\") { success } }",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 4, "{}", stderr(&o));
}
