//! Label writes: resolving, the checks that run before a request, the inputs,
//! and the `label-groups-exclusive` rule asked about a label that moves.

use cynic::GraphQlResponse;
use linear_core::filters::{issues_with_label, issues_with_label_of_group};
use linear_core::inputs::Patch;
use linear_core::label_write::*;
use linear_core::read::IssueList;
use linear_core::rules::label_groups::{regroup_violations, Regroup};
use linear_core::types::{LabelGroup, LabelGroupType};
use linear_core::wire::build_request;
use serde_json::json;

const AREA: &str = "00000000-0000-4000-8000-000000000009";
const API: &str = "00000000-0000-4000-8000-000000000008";
const BUG: &str = "00000000-0000-4000-8000-000000000010";
const FLOW: &str = "00000000-0000-4000-8000-000000000012";
const TEAM_EX: &str = "00000000-0000-4000-8000-000000000004";

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let r: GraphQlResponse<T> = serde_json::from_str(&text).unwrap();
    r.data.expect("data")
}

fn labels() -> Vec<LabelDetail> {
    let data: LabelDetails = fixture("label_details");
    data.issue_labels.nodes
}

// ------------------------------------------------------------------ resolving

#[test]
fn the_fixture_parses_with_teams_descriptions_and_group_types() {
    let all = labels();
    assert_eq!(all.len(), 7);
    let area = &all[0];
    assert!(area.is_group);
    assert_eq!(area.group_type, Some(LabelGroupType::SingleSelect));
    assert_eq!(area.scope(), "workspace");
    let review = all.iter().find(|l| l.name == "review").unwrap();
    assert_eq!(review.scope(), "EX");
    assert_eq!(review.path(), "flow/review");
    assert_eq!(all[1].description.as_deref(), Some("Public API"));
}

#[test]
fn a_label_is_found_by_name_path_or_id_ignoring_case() {
    let all = labels();
    assert_eq!(resolve(&all, "bug").unwrap().id.inner(), BUG);
    assert_eq!(resolve(&all, "area/api").unwrap().id.inner(), API);
    assert_eq!(resolve(&all, "AREA/API").unwrap().id.inner(), API);
    assert_eq!(resolve(&all, API).unwrap().name, "api");
    let err = resolve(&all, "nope").unwrap_err().to_string();
    assert!(err.contains("no label \"nope\""), "{err}");
}

#[test]
fn a_name_that_two_labels_share_is_ambiguous_and_lists_ids() {
    let mut all = labels();
    let mut twin = all[2].clone();
    twin.id = cynic::Id::new("twin-id");
    all.push(twin);
    let err = resolve(&all, "Bug").unwrap_err().to_string();
    assert!(
        err.contains("ambiguous") && err.contains("twin-id") && err.contains(BUG),
        "{err}"
    );
    // An id is never ambiguous.
    assert_eq!(resolve(&all, "twin-id").unwrap().id.inner(), "twin-id");
}

#[test]
fn only_a_group_can_be_named_as_a_group() {
    let all = labels();
    assert_eq!(resolve_group(&all, "area").unwrap().id.inner(), AREA);
    let err = resolve_group(&all, "Bug").unwrap_err().to_string();
    assert!(err.contains("a label, not a group"), "{err}");
}

// ------------------------------------------------------------------ checks

#[test]
fn a_label_goes_in_a_group_of_its_own_team() {
    let all = labels();
    let area = resolve_group(&all, "area").unwrap(); // workspace group
    let flow = resolve_group(&all, "flow").unwrap(); // team EX group

    assert!(check_group(area, false, None, "the workspace").is_ok());
    assert!(check_group(flow, false, Some(TEAM_EX), "team EX").is_ok());

    let err = check_group(flow, false, None, "the workspace")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("belongs to team EX") && err.contains("the workspace"),
        "{err}"
    );
    let err = check_group(area, false, Some(TEAM_EX), "team EX")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("belongs to the workspace") && err.contains("team EX"),
        "{err}"
    );

    // A group does not go in a group; a plain label is not a group.
    let err = check_group(area, true, None, "the workspace")
        .unwrap_err()
        .to_string();
    assert!(err.contains("one level"), "{err}");
    let bug = &all[2];
    let err = check_group(bug, false, None, "the workspace")
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a group"), "{err}");
}

#[test]
fn names_collide_across_the_workspace_and_a_team_but_not_across_teams() {
    let mut all = labels();
    // Whatever the case, and wherever the label is.
    assert_eq!(find_taken(&all, "BUG", None, None).unwrap().id.inner(), BUG);
    assert!(
        find_taken(&all, "Bug", Some(TEAM_EX), None).is_some(),
        "a team label takes no workspace name"
    );
    // A workspace label takes no team's name either.
    assert!(find_taken(&all, "team only", None, None).is_some());
    // The label being renamed does not collide with itself.
    assert!(find_taken(&all, "Bug", None, Some(BUG)).is_none());
    assert!(find_taken(&all, "brand new", None, None).is_none());

    // Two teams may use the same name (the other team's copy does not collide with
    // the label of team EX), while a workspace name is taken for every team.
    let mut other = all[6].clone();
    other.id = cynic::Id::new("other-team-label");
    other.team.as_mut().unwrap().id = cynic::Id::new("team-2");
    all.push(other);
    assert!(find_taken(&all, "Team only", Some("team-2"), Some("other-team-label")).is_none());
    assert!(find_taken(&all, "Bug", Some("team-2"), None).is_some());
}

#[test]
fn a_color_is_a_hex_color() {
    assert_eq!(check_color(" #4EA7FC ").unwrap(), "#4EA7FC");
    for bad in ["red", "4EA7FC", "#4EA7F", "#GGGGGG", "#4EA7FC0"] {
        assert!(check_color(bad).is_err(), "{bad}");
    }
}

// ------------------------------------------------------------------ inputs

#[test]
fn a_create_sends_only_what_was_given() {
    let op = label_create(LabelCreateInput {
        name: "ship".into(),
        color: None,
        description: None,
        team_id: None,
        parent_id: Some(AREA.into()),
        is_group: None,
        group_type: None,
    });
    let req = build_request(&op);
    assert_eq!(req.operation_name.as_deref(), Some("LabelCreate"));
    assert_eq!(
        req.variables["input"],
        json!({"name": "ship", "parentId": AREA})
    );

    let op = label_create(LabelCreateInput {
        name: "grp".into(),
        color: Some("#4EA7FC".into()),
        description: Some("d".into()),
        team_id: Some(TEAM_EX.into()),
        parent_id: None,
        is_group: Some(true),
        group_type: Some(LabelGroupType::SingleSelect),
    });
    assert_eq!(
        build_request(&op).variables["input"],
        json!({"name": "grp", "color": "#4EA7FC", "description": "d", "teamId": TEAM_EX,
               "isGroup": true, "groupType": "singleSelect"})
    );
}

#[test]
fn an_update_leaves_alone_what_it_does_not_name_and_clears_a_group_with_null() {
    let none = LabelUpdateInput::default();
    assert!(none.is_empty());
    assert_eq!(
        build_request(&label_update(API, none)).variables["input"],
        json!({})
    );

    let rename = LabelUpdateInput {
        name: Some("rest".into()),
        ..LabelUpdateInput::default()
    };
    assert!(!rename.is_empty());
    assert_eq!(
        build_request(&label_update(API, rename)).variables["input"],
        json!({"name": "rest"})
    );

    let out = LabelUpdateInput {
        parent_id: Patch::Clear,
        ..LabelUpdateInput::default()
    };
    assert!(!out.is_empty());
    let req = build_request(&label_update(API, out));
    assert_eq!(req.operation_name.as_deref(), Some("LabelUpdate"));
    assert_eq!(req.variables["input"], json!({"parentId": null}));

    let into = LabelUpdateInput {
        parent_id: Patch::Set(AREA.into()),
        group_type: Some(LabelGroupType::MultiSelect),
        ..LabelUpdateInput::default()
    };
    assert_eq!(
        build_request(&label_update(API, into)).variables["input"],
        json!({"parentId": AREA, "groupType": "multiSelect"})
    );
}

#[test]
fn the_issue_filters_name_a_label_or_a_group_by_id() {
    let req = build_request(&linear_core::read::issue_list(
        linear_core::read::IssueListVars::new(
            linear_core::types::PageVars {
                first: 50,
                after: None,
            },
            Some(issues_with_label(BUG)),
        ),
    ));
    assert_eq!(
        req.variables["filter"],
        json!({"labels": {"some": {"id": {"eq": BUG}}}})
    );
    let req = build_request(&linear_core::read::issue_list(
        linear_core::read::IssueListVars::new(
            linear_core::types::PageVars {
                first: 50,
                after: None,
            },
            Some(issues_with_label_of_group(AREA)),
        ),
    ));
    assert_eq!(
        req.variables["filter"],
        json!({"labels": {"some": {"parent": {"id": {"eq": AREA}}}}})
    );
}

// ------------------------------------------------------------------ the exclusivity rule

fn issues() -> Vec<linear_core::types::Issue> {
    let data: IssueList = fixture("issue_list");
    data.issues.nodes
}

fn group(id: &str, name: &str, kind: Option<LabelGroupType>) -> LabelGroup {
    LabelGroup {
        id: cynic::Id::new(id),
        name: name.into(),
        group_type: kind,
    }
}

#[test]
fn moving_a_label_next_to_a_sibling_on_the_same_issue_is_a_conflict() {
    // The fixture issue (EX-23) has `api` (in `area`) and `Bug` (in no group).
    let area = group(AREA, "area", Some(LabelGroupType::SingleSelect));
    let v = regroup_violations(
        &issues(),
        &Regroup::Move {
            label_id: BUG,
            group: &area,
        },
    );
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].subject.as_deref(), Some("area"));
    assert!(
        v[0].message.contains("EX-23") && v[0].message.contains("api, Bug"),
        "{}",
        v[0]
    );
}

#[test]
fn a_multi_select_group_never_conflicts() {
    let area = group(AREA, "area", Some(LabelGroupType::MultiSelect));
    assert!(regroup_violations(
        &issues(),
        &Regroup::Move {
            label_id: BUG,
            group: &area
        }
    )
    .is_empty());
}

#[test]
fn a_label_that_is_on_no_issue_with_a_sibling_moves_freely() {
    // `api` itself moving to its own group, or an unrelated label moving: no new conflict.
    let area = group(AREA, "area", Some(LabelGroupType::SingleSelect));
    assert!(regroup_violations(
        &issues(),
        &Regroup::Move {
            label_id: API,
            group: &area
        }
    )
    .is_empty());
    assert!(regroup_violations(
        &issues(),
        &Regroup::Move {
            label_id: "some-other-label",
            group: &area
        }
    )
    .is_empty());
}

#[test]
fn making_a_group_single_select_is_a_conflict_for_an_issue_with_two_of_its_labels() {
    // Put `Bug` into `area` too (as it would be in a multi-select group), then retype.
    let mut with_two = issues();
    for l in with_two[0].labels.nodes.iter_mut() {
        l.parent = Some(group(AREA, "area", Some(LabelGroupType::MultiSelect)));
    }
    let single = LabelGroupType::SingleSelect;
    let v = regroup_violations(
        &with_two,
        &Regroup::Retype {
            group_id: AREA,
            group_type: &single,
        },
    );
    assert_eq!(v.len(), 1, "{v:?}");
    assert!(v[0].message.contains("EX-23"));

    // The other direction never conflicts.
    let multi = LabelGroupType::MultiSelect;
    assert!(regroup_violations(
        &with_two,
        &Regroup::Retype {
            group_id: AREA,
            group_type: &multi
        }
    )
    .is_empty());
}

#[test]
fn a_conflict_an_issue_already_had_is_not_reported() {
    let mut already = issues();
    for l in already[0].labels.nodes.iter_mut() {
        l.parent = Some(group(FLOW, "flow", Some(LabelGroupType::SingleSelect)));
    }
    let flow = group(FLOW, "flow", Some(LabelGroupType::SingleSelect));
    assert!(regroup_violations(
        &already,
        &Regroup::Move {
            label_id: BUG,
            group: &flow
        }
    )
    .is_empty());
}
