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

    // A mutation is refused locally (exit 4), with or without `--mutation`, while
    // the workspace has not allowed it.
    for extra in [&[][..], &["--mutation"][..]] {
        let mut args = vec![
            "api",
            "mutation { issueDelete(id: \"SAND-1\") { success } }",
        ];
        args.extend_from_slice(extra);
        let o = sb.run(&args, None, &env);
        assert_eq!(code(&o), 4, "{extra:?}: {}", stderr(&o));
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn a_raw_mutation_runs_once_the_workspace_allows_it() {
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
    let path = sb.config_dir().join("workspaces.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\nallow_raw_mutation = true\n");
    std::fs::write(&path, text).unwrap();

    // Create a template and delete it again, both through raw mutations.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let input = serde_json::json!({
        "type": "issue",
        "name": format!("live raw mutation {stamp}"),
        "templateData": { "title": "", "descriptionData": { "type": "doc", "content": [] } },
    })
    .to_string();
    let o = sb.run(
        &[
            "api",
            "mutation Create($input: TemplateCreateInput!) { templateCreate(input: $input) { success template { id name } } }",
            "--mutation",
            "--var-json",
            &format!("input={input}"),
            "--json",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stderr(&o), "", "--json prints no warning");
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["templateCreate"]["success"], true);
    let id = v["templateCreate"]["template"]["id"].as_str().unwrap();
    assert!(!id.is_empty());

    // Without --json the warning is printed first.
    let o = sb.run(
        &[
            "api",
            "mutation Delete($id: String!) { templateDelete(id: $id) { success } }",
            "--mutation",
            "--var",
            &format!("id={id}"),
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).contains("warning:"), "{}", stderr(&o));
    assert!(stdout(&o).contains("\"success\": true"), "{}", stdout(&o));
    assert!(!stdout(&o).contains(&key) && !stderr(&o).contains(&key));
}
