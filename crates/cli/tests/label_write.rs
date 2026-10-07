//! `linear label create` and `linear label update` against a mock Linear that
//! answers by operation name: the checks that come before a request, the
//! idempotent return, the `label-groups-exclusive` rule on `update`, and the
//! mutations themselves, seen from outside.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::{json, Value};
use write_support::*;

const AREA: &str = "00000000-0000-4000-8000-000000000009";
const API: &str = "00000000-0000-4000-8000-000000000008";
const BUG: &str = "00000000-0000-4000-8000-000000000010";
const FLOW: &str = "00000000-0000-4000-8000-000000000012";
const TEAM_EX: &str = "00000000-0000-4000-8000-000000000004";
const NEW_LABEL: &str = "00000000-0000-4000-8000-0000000000b1";

const MUTATIONS: [&str; 2] = ["LabelCreate", "LabelUpdate"];

fn assert_nothing_written(mock: &Routed) {
    let ops = mock.ops();
    assert!(
        ops.iter().all(|o| !MUTATIONS.contains(&o.as_str())),
        "a mutation was sent: {ops:?}"
    );
}

fn details() -> Value {
    serde_json::from_str::<Value>(&fixture("label_details")).unwrap()["data"].clone()
}

fn label_details() -> Reply {
    data(details())
}

/// The label of the fixture with this id, with `changes` merged in.
fn label_like(id: &str, changes: Value) -> Value {
    let mut label = details()["issueLabels"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap_or_else(|| panic!("no fixture label {id}"))
        .clone();
    for (k, v) in changes.as_object().unwrap() {
        label[k] = v.clone();
    }
    label
}

fn new_label(changes: Value) -> Value {
    let mut label = json!({
        "id": NEW_LABEL, "name": "x", "color": "#95A2B3", "description": null, "isGroup": false,
        "groupType": null, "parent": null, "team": null
    });
    for (k, v) in changes.as_object().unwrap() {
        label[k] = v.clone();
    }
    label
}

fn payload(field: &str, label: Value) -> Reply {
    data(json!({ field: { "success": true, "issueLabel": label } }))
}

fn routes(extra: Vec<(&'static str, Vec<Reply>)>) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes: Vec<(&'static str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("LabelDetails", vec![label_details()]),
        ("Teams", vec![teams()]),
    ];
    routes.extend(extra);
    routes
}

fn issue_list() -> Reply {
    ok(&fixture("issue_list"))
}

fn group_ref(id: &str, name: &str, kind: &str) -> Value {
    json!({"id": id, "name": name, "groupType": kind})
}

// ------------------------------------------------------------------ label create

#[test]
fn a_workspace_label_is_created_with_a_color_and_a_description() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![payload(
            "issueLabelCreate",
            new_label(json!({"name": "ship", "color": "#112233", "description": "Ready"})),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "create",
            "--name",
            "ship",
            "--color",
            "#112233",
            "--description",
            "Ready",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["existing"], false);
    assert_eq!(v["name"], "ship");
    assert_eq!(v["id"], NEW_LABEL);

    // Reads, then one mutation; a workspace label has no team in the request.
    assert_eq!(mock.ops(), ["Whoami", "LabelDetails", "LabelCreate"]);
    assert_eq!(
        mock.of("LabelCreate")[0]["input"],
        json!({"name": "ship", "color": "#112233", "description": "Ready"})
    );
}

#[test]
fn a_team_label_sends_the_teams_id() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![payload(
            "issueLabelCreate",
            new_label(
                json!({"name": "ship", "team": {"id": TEAM_EX, "key": "EX", "name": "Example"}}),
            ),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &["label", "create", "--name", " ship ", "--team", "ex"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("ship")
            && out.contains("label")
            && out.contains("EX")
            && out.contains("(created)"),
        "{out}"
    );
    assert_eq!(
        mock.of("LabelCreate")[0]["input"],
        json!({"name": "ship", "teamId": TEAM_EX})
    );
}

#[test]
fn a_label_goes_in_a_group_and_takes_the_groups_team() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![payload(
            "issueLabelCreate",
            new_label(json!({"name": "ship", "parent": group_ref(AREA, "area", "singleSelect")})),
        )],
    )]));
    // A workspace group: no team.
    let o = run(
        &sb,
        &mock,
        &[
            "label", "create", "--name", "ship", "--group", "area", "--quiet",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "area/ship");
    assert_eq!(
        mock.of("LabelCreate")[0]["input"],
        json!({"name": "ship", "parentId": AREA})
    );

    // A team's group: the label is the team's too, without --team.
    let o = run(
        &sb,
        &mock,
        &["label", "create", "--name", "ship", "--group", "flow"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("LabelCreate")[1]["input"],
        json!({"name": "ship", "teamId": TEAM_EX, "parentId": FLOW})
    );
    assert!(
        !mock.ops().contains(&"Teams".to_owned()),
        "no --team, no team lookup"
    );
}

#[test]
fn a_group_is_created_with_its_selection_mode() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![payload(
            "issueLabelCreate",
            new_label(json!({"name": "kind", "isGroup": true, "groupType": "singleSelect"})),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "create",
            "--name",
            "kind",
            "--is-group",
            "--group-type",
            "single-select",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["isGroup"], true);
    assert_eq!(
        mock.of("LabelCreate")[0]["input"],
        json!({"name": "kind", "isGroup": true, "groupType": "singleSelect"})
    );
}

#[test]
fn a_label_that_is_already_there_is_returned_and_nothing_is_created() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    // Ignoring case, in the same place, outside any group.
    let o = run(&sb, &mock, &["label", "create", "--name", "bug", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["existing"], true);
    assert_eq!(v["id"], BUG);
    assert_nothing_written(&mock);

    // Also inside its group.
    let o = run(
        &sb,
        &mock,
        &[
            "label", "create", "--name", "api", "--group", "area", "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["id"], API);
    assert_nothing_written(&mock);

    let o = run(&sb, &mock, &["label", "create", "--name", "Bug"]);
    assert!(
        stdout(&o).contains("already exists, nothing created"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn a_name_that_is_used_somewhere_else_is_refused_before_any_request() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    // `api` exists inside `area`; asking for it outside any group is not the same label.
    let o = run(&sb, &mock, &["label", "create", "--name", "api"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("area/api") && stderr(&o).contains("unique"),
        "{}",
        stderr(&o)
    );
    // A team label may not take a workspace label's name.
    let o = run(
        &sb,
        &mock,
        &["label", "create", "--name", "Bug", "--team", "EX"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert_nothing_written(&mock);
}

#[test]
fn a_group_of_another_team_a_label_or_a_missing_group_is_refused() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    let cases: [(&[&str], &str); 4] = [
        (
            &["--group", "area", "--team", "EX"],
            "belongs to the workspace",
        ),
        (&["--group", "Bug"], "a label, not a group"),
        (&["--group", "nope"], "no label \"nope\""),
        (&["--team", "nope"], "nope"),
    ];
    for (extra, want) in cases {
        let mut args = vec!["label", "create", "--name", "ship"];
        args.extend(extra);
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{extra:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(want), "{extra:?}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
}

#[test]
fn bad_input_is_refused_before_anything_is_sent() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![]));
    for args in [
        vec!["label", "create", "--name", "  "],
        vec!["label", "create", "--name", "x", "--color", "red"],
        // clap: a group cannot be in a group; a mode is for groups.
        vec![
            "label",
            "create",
            "--name",
            "x",
            "--is-group",
            "--group",
            "area",
        ],
        vec![
            "label",
            "create",
            "--name",
            "x",
            "--group-type",
            "single-select",
        ],
        vec![
            "label",
            "create",
            "--name",
            "x",
            "--is-group",
            "--group-type",
            "some",
        ],
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
    // The checks that need no lookup happen before the first request.
    let first = Routed::start(vec![]);
    let o = run(
        &sb,
        &first,
        &["label", "create", "--name", "x", "--color", "red"],
    );
    assert_eq!(code(&o), 2);
    assert!(first.ops().is_empty());
}

#[test]
fn an_error_from_linear_is_passed_on() {
    let sb = workspace();
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![graphql_error(
            "Multi-select label groups are not enabled for this workspace.",
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "create",
            "--name",
            "kind",
            "--is-group",
            "--group-type",
            "multi-select",
        ],
    );
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("not enabled for this workspace"),
        "{}",
        stderr(&o)
    );
}

// ------------------------------------------------------------------ label update

fn update_routes(label: Value) -> Vec<(&'static str, Vec<Reply>)> {
    routes(vec![(
        "LabelUpdate",
        vec![payload("issueLabelUpdate", label)],
    )])
}

#[test]
fn a_label_is_renamed_and_only_what_differs_is_sent() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(BUG, json!({"name": "Defect"}))));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "update",
            "bug",
            "--new-name",
            "Defect",
            // The color is the one it has, in another case: not sent.
            "--color",
            "#eb5757",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = stdout_json(&o);
    assert_eq!(v["changed"], true);
    assert_eq!(v["name"], "Defect");
    assert_eq!(mock.ops(), ["Whoami", "LabelDetails", "LabelUpdate"]);
    let call = &mock.of("LabelUpdate")[0];
    assert_eq!(call["id"], BUG);
    assert_eq!(call["input"], json!({"name": "Defect"}));
}

#[test]
fn an_update_that_changes_nothing_sends_nothing() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(BUG, json!({}))));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "update",
            "Bug",
            "--new-name",
            "Bug",
            "--color",
            "#EB5757",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout_json(&o)["changed"], false);
    assert_nothing_written(&mock);

    let o = run(&sb, &mock, &["label", "update", "Bug"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("nothing to change"), "{}", stderr(&o));
}

#[test]
fn a_description_is_set_and_cleared_with_an_empty_value() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(API, json!({"description": ""}))));
    let o = run(
        &sb,
        &mock,
        &["label", "update", "area/api", "--description", ""],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("LabelUpdate")[0]["input"],
        json!({"description": ""})
    );
}

#[test]
fn a_new_name_another_label_has_is_refused() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(BUG, json!({}))));
    let o = run(
        &sb,
        &mock,
        &["label", "update", "Bug", "--new-name", "feature"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("already exists"), "{}", stderr(&o));
    assert_nothing_written(&mock);
}

#[test]
fn a_label_is_moved_into_a_group_and_out_of_it() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(
        BUG,
        json!({"parent": group_ref(AREA, "area", "singleSelect")}),
    )));
    let o = run(&sb, &mock, &["label", "update", "Bug", "--group", "area"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("LabelUpdate")[0]["input"],
        json!({"parentId": AREA})
    );
    // The rule is not enabled here, so no issue is read.
    assert!(
        !mock.ops().contains(&"IssueList".to_owned()),
        "{:?}",
        mock.ops()
    );

    let mock = Routed::start(update_routes(label_like(API, json!({"parent": null}))));
    let o = run(&sb, &mock, &["label", "update", "area/api", "--no-group"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("LabelUpdate")[0]["input"],
        json!({"parentId": null})
    );

    // Already in the group, and already out of any: nothing to send.
    let mock = Routed::start(update_routes(label_like(API, json!({}))));
    let o = run(
        &sb,
        &mock,
        &["label", "update", "area/api", "--group", "area", "--json"],
    );
    assert_eq!(stdout_json(&o)["changed"], false);
    let o = run(
        &sb,
        &mock,
        &["label", "update", "Bug", "--no-group", "--json"],
    );
    assert_eq!(stdout_json(&o)["changed"], false);
    assert_nothing_written(&mock);
}

#[test]
fn a_move_to_the_wrong_kind_of_group_is_refused() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(BUG, json!({}))));
    for (args, want) in [
        (
            vec!["label", "update", "Bug", "--group", "flow"],
            "belongs to team EX",
        ),
        (
            vec!["label", "update", "Bug", "--group", "Feature"],
            "a label, not a group",
        ),
        (
            vec!["label", "update", "area", "--group", "flow"],
            "one level",
        ),
        (
            vec!["label", "update", "Bug", "--group-type", "single-select"],
            "is for groups",
        ),
        (
            vec!["label", "update", "Bug", "--group", "area", "--no-group"],
            "",
        ),
    ] {
        let o = run(&sb, &mock, &args);
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(want), "{args:?}: {}", stderr(&o));
    }
    assert_nothing_written(&mock);
}

#[test]
fn a_group_changes_its_selection_mode() {
    let sb = workspace();
    let mock = Routed::start(update_routes(label_like(
        AREA,
        json!({"groupType": "multiSelect"}),
    )));
    let o = run(
        &sb,
        &mock,
        &[
            "label",
            "update",
            "area",
            "--group-type",
            "multi-select",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("LabelUpdate")[0]["input"],
        json!({"groupType": "multiSelect"})
    );
    assert_eq!(stdout_json(&o)["groupType"], "multiSelect");
}

#[test]
fn a_name_two_labels_share_must_be_given_as_an_id() {
    let sb = workspace();
    let mut twice = details();
    let mut twin = twice["issueLabels"]["nodes"][2].clone();
    twin["id"] = json!("00000000-0000-4000-8000-0000000000c2");
    twice["issueLabels"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(twin);
    let mock = Routed::start(routes(vec![
        ("LabelDetails", vec![data(twice)]),
        (
            "LabelUpdate",
            vec![payload("issueLabelUpdate", label_like(BUG, json!({})))],
        ),
    ]));
    let o = run(
        &sb,
        &mock,
        &["label", "update", "Bug", "--color", "#000000"],
    );
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("ambiguous") && stderr(&o).contains("0000000000c2"),
        "{}",
        stderr(&o)
    );
    assert_nothing_written(&mock);
}

// ------------------------------------------------------------------ the validator rule

fn moved_bug() -> Vec<(&'static str, Vec<Reply>)> {
    let mut r = update_routes(label_like(
        BUG,
        json!({"parent": group_ref(AREA, "area", "singleSelect")}),
    ));
    r.push(("IssueList", vec![issue_list()]));
    r
}

#[test]
fn label_groups_exclusive_refuses_a_move_that_puts_two_of_a_group_on_an_issue() {
    // The fixture issue EX-23 has `api` (in area) and `Bug`; Bug into area makes two.
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mock = Routed::start(moved_bug());
    let o = run(&sb, &mock, &["label", "update", "Bug", "--group", "area"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    let err = stderr(&o);
    assert!(
        err.contains("label-groups-exclusive") && err.contains("EX-23") && err.contains("api, Bug"),
        "{err}"
    );
    assert_nothing_written(&mock);
    // The issues asked about are the ones that carry the label.
    assert_eq!(
        mock.of("IssueList")[0]["filter"],
        json!({"labels": {"some": {"id": {"eq": BUG}}}})
    );
}

#[test]
fn label_groups_exclusive_lets_a_harmless_move_through() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    // `Feature` is on no issue of the fixture list together with another label of `area`.
    let mut r = update_routes(label_like(
        "00000000-0000-4000-8000-000000000011",
        json!({"parent": group_ref(AREA, "area", "singleSelect")}),
    ));
    r.push((
        "IssueList",
        vec![data(
            json!({"issues": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}}),
        )],
    ));
    let mock = Routed::start(r);
    let o = run(
        &sb,
        &mock,
        &["label", "update", "Feature", "--group", "area"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.ops(),
        ["Whoami", "LabelDetails", "IssueList", "LabelUpdate"]
    );
}

#[test]
fn label_groups_exclusive_leaves_a_multi_select_group_alone() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mut all = details();
    all["issueLabels"]["nodes"][0]["groupType"] = json!("multiSelect");
    let mut r = moved_bug();
    r.push(("LabelDetails", vec![data(all)]));
    let mock = Routed::start(r);
    let o = run(&sb, &mock, &["label", "update", "Bug", "--group", "area"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        !mock.ops().contains(&"IssueList".to_owned()),
        "{:?}",
        mock.ops()
    );
}

#[test]
fn label_groups_exclusive_refuses_making_a_group_single_select_under_two_labels_of_it() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    // `area` is multi-select here and EX-23 carries two of its labels.
    let mut all = details();
    all["issueLabels"]["nodes"][0]["groupType"] = json!("multiSelect");
    let mut issues: Value =
        serde_json::from_str::<Value>(&fixture("issue_list")).unwrap()["data"].clone();
    for l in issues["issues"]["nodes"][0]["labels"]["nodes"]
        .as_array_mut()
        .unwrap()
    {
        l["parent"] = group_ref(AREA, "area", "multiSelect");
    }
    let mock = Routed::start(vec![
        ("Whoami", vec![whoami()]),
        ("LabelDetails", vec![data(all)]),
        ("IssueList", vec![data(issues)]),
        (
            "LabelUpdate",
            vec![payload("issueLabelUpdate", label_like(AREA, json!({})))],
        ),
    ]);
    let o = run(
        &sb,
        &mock,
        &["label", "update", "area", "--group-type", "single-select"],
    );
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("EX-23"), "{}", stderr(&o));
    assert_nothing_written(&mock);
    assert_eq!(
        mock.of("IssueList")[0]["filter"],
        json!({"labels": {"some": {"parent": {"id": {"eq": AREA}}}}})
    );
}

#[test]
fn a_creation_has_no_issue_to_hold_against_the_rule() {
    let sb = workspace_with_rules(&["label-groups-exclusive"]);
    let mock = Routed::start(routes(vec![(
        "LabelCreate",
        vec![payload(
            "issueLabelCreate",
            new_label(json!({"name": "ship", "parent": group_ref(AREA, "area", "singleSelect")})),
        )],
    )]));
    let o = run(
        &sb,
        &mock,
        &["label", "create", "--name", "ship", "--group", "area"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!mock.ops().contains(&"IssueList".to_owned()));
}
