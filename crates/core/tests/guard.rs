//! The ownership-based write guard.

use cynic::GraphQlResponse;
use linear_core::guard::*;
use linear_core::queries::{IssueById, Projects, Whoami};
use linear_core::rules::Operation;
use linear_core::ErrorCode;

const ME: &str = "user-me";
const BOB: &str = "user-bob";

fn me() -> Viewer {
    Viewer::new("main", ME)
}

fn mine() -> Placement<'static> {
    Placement::Project { lead: Some(ME) }
}

fn bobs() -> Placement<'static> {
    Placement::Project { lead: Some(BOB) }
}

fn unled() -> Placement<'static> {
    Placement::Project { lead: None }
}

fn allowed(w: Write<'_>, allow_foreign: bool) {
    assert_eq!(check(&me(), &w, allow_foreign), Ok(()), "{w:?}");
}

fn denied(w: Write<'_>, allow_foreign: bool) -> Denied {
    check(&me(), &w, allow_foreign).expect_err(&format!("{w:?} should be denied"))
}

// ------------------------------------------------------------------ projects

#[test]
fn projects_are_writable_only_when_i_lead_them() {
    allowed(Write::ProjectUpdate { lead: Some(ME) }, false);
    allowed(Write::ProjectCreate { lead: Some(ME) }, false);

    for w in [
        Write::ProjectUpdate { lead: Some(BOB) },
        Write::ProjectUpdate { lead: None },
        Write::ProjectCreate { lead: Some(BOB) },
        Write::ProjectCreate { lead: None },
    ] {
        assert_eq!(denied(w, false).reason, DenyReason::ProjectNotLed, "{w:?}");
    }
}

#[test]
fn allow_foreign_never_opens_a_project() {
    let d = denied(Write::ProjectUpdate { lead: Some(BOB) }, true);
    assert_eq!(d.reason, DenyReason::ProjectNotLed);
    denied(Write::ProjectCreate { lead: Some(BOB) }, true);
}

// -------------------------------------------------------------- issue update

#[test]
fn an_issue_is_writable_when_assigned_to_me_or_in_my_project() {
    // Assigned to me, in someone else's project, or in none.
    for placement in [bobs(), unled(), Placement::NoProject, mine()] {
        allowed(
            Write::IssueUpdate {
                assignee: Some(ME),
                placement,
                moves_to: None,
            },
            false,
        );
    }
    // Assigned to someone else or nobody, in my project.
    for assignee in [Some(BOB), None] {
        allowed(
            Write::IssueUpdate {
                assignee,
                placement: mine(),
                moves_to: None,
            },
            false,
        );
    }
}

#[test]
fn someone_elses_issue_in_someone_elses_project_is_refused() {
    for assignee in [Some(BOB), None] {
        for placement in [bobs(), unled(), Placement::NoProject] {
            let d = denied(
                Write::IssueUpdate {
                    assignee,
                    placement,
                    moves_to: None,
                },
                false,
            );
            assert_eq!(d.reason, DenyReason::IssueNotOwned);
            assert_eq!(d.operation, Operation::IssueUpdate);
        }
    }
}

#[test]
fn allow_foreign_does_not_open_an_issue_update() {
    let w = Write::IssueUpdate {
        assignee: Some(BOB),
        placement: bobs(),
        moves_to: None,
    };
    assert_eq!(denied(w, true).reason, DenyReason::IssueNotOwned);
}

#[test]
fn moving_an_issue_needs_a_destination_i_lead_or_no_project() {
    let moving = |assignee, placement, moves_to| Write::IssueUpdate {
        assignee,
        placement,
        moves_to: Some(moves_to),
    };
    allowed(moving(Some(ME), bobs(), mine()), false);
    allowed(moving(Some(ME), mine(), Placement::NoProject), false);

    // Into someone else's project, even for my own issue and even with the flag.
    for flag in [false, true] {
        for dest in [bobs(), unled()] {
            let d = denied(moving(Some(ME), mine(), dest), flag);
            assert_eq!(d.reason, DenyReason::ForeignProject);
        }
    }
    // The current owner check still comes first.
    let d = denied(moving(Some(BOB), bobs(), mine()), false);
    assert_eq!(d.reason, DenyReason::IssueNotOwned);
}

// -------------------------------------------------------------- issue create

#[test]
fn creating_in_my_project_is_allowed_for_any_assignee() {
    for assignee in [Some(ME), Some(BOB), None] {
        allowed(
            Write::IssueCreate {
                assignee,
                placement: mine(),
            },
            false,
        );
    }
}

#[test]
fn creating_without_a_project_needs_me_as_assignee() {
    let create = |assignee| Write::IssueCreate {
        assignee,
        placement: Placement::NoProject,
    };
    allowed(create(Some(ME)), false);
    for assignee in [Some(BOB), None] {
        assert_eq!(
            denied(create(assignee), true).reason,
            DenyReason::IssueNotOwned
        );
    }
}

#[test]
fn creating_in_a_foreign_project_needs_the_flag_and_me_as_assignee() {
    let create = |assignee, placement| Write::IssueCreate {
        assignee,
        placement,
    };
    for placement in [bobs(), unled()] {
        // Assigned to me, with the flag: the one allowed case.
        allowed(create(Some(ME), placement), true);

        // Without the flag.
        let d = denied(create(Some(ME), placement), false);
        assert_eq!(d.reason, DenyReason::ForeignProject);
        assert!(d.message.contains("--allow-foreign"), "{}", d.message);

        // The flag does not cover issues for other people or for nobody.
        for assignee in [Some(BOB), None] {
            for flag in [false, true] {
                let d = denied(create(assignee, placement), flag);
                assert_eq!(d.reason, DenyReason::ForeignProject);
            }
        }
    }
    // Messages tell a project led by someone from one nobody leads.
    assert!(denied(create(Some(ME), bobs()), false)
        .message
        .contains("led by someone else"));
    assert!(denied(create(Some(ME), unled()), false)
        .message
        .contains("no lead"));
}

#[test]
fn the_flag_is_harmless_where_it_is_not_needed() {
    allowed(
        Write::IssueCreate {
            assignee: Some(BOB),
            placement: mine(),
        },
        true,
    );
    allowed(Write::ProjectUpdate { lead: Some(ME) }, true);
}

// ------------------------------------------------------------------- errors

#[test]
fn a_denial_maps_to_exit_code_4() {
    let d = denied(Write::ProjectUpdate { lead: Some(BOB) }, false);
    assert_eq!(d.code(), ErrorCode::WriteDenied);
    assert_eq!(d.code().exit_code(), 4);
    assert_eq!(d.operation, Operation::ProjectUpdate);
    assert!(d.to_string().contains("not the lead"), "{d}");
}

#[test]
fn the_write_names_its_operation() {
    assert_eq!(
        Write::ProjectCreate { lead: None }.operation(),
        Operation::ProjectCreate
    );
    assert_eq!(
        Write::IssueCreate {
            assignee: None,
            placement: Placement::NoProject
        }
        .operation(),
        Operation::IssueCreate
    );
}

// ------------------------------------------------- viewer and real responses

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let r: GraphQlResponse<T> = serde_json::from_str(&text).unwrap();
    r.data.expect("data")
}

#[test]
fn the_viewer_is_per_workspace_and_round_trips_for_caching() {
    let who: Whoami = fixture("whoami");
    let v = Viewer::from_user("main", &who.viewer);
    assert_eq!(v.workspace, "main");
    assert_eq!(v.id, who.viewer.id.inner());

    let text = serde_json::to_string(&v).unwrap();
    assert_eq!(serde_json::from_str::<Viewer>(&text).unwrap(), v);

    // The same user id in another workspace is a different viewer.
    assert_ne!(Viewer::from_user("other", &who.viewer), v);
}

#[test]
fn decisions_over_real_responses() {
    let who: Whoami = fixture("whoami");
    let alice = Viewer::from_user("main", &who.viewer);
    let someone_else = Viewer::new("main", "somebody-else");

    let projects: Projects = fixture("projects");
    let project = &projects.projects.nodes[0];
    assert_eq!(project.lead.as_ref().unwrap().id.inner(), alice.id);

    // Alice leads the project: she may write it and its issues.
    let write = Write::update_project(project);
    assert_eq!(check(&alice, &write, false), Ok(()));
    let denied = check(&someone_else, &write, false).unwrap_err();
    assert_eq!(denied.reason, DenyReason::ProjectNotLed);

    let issue: IssueById = fixture("issue");
    let issue = issue.issue;
    let placement = Placement::of_project(project);
    let write = Write::update_issue(&issue, placement);
    assert_eq!(check(&alice, &write, false), Ok(()));
    // Somebody else is neither the assignee nor the lead.
    let denied = check(&someone_else, &write, false).unwrap_err();
    assert_eq!(denied.reason, DenyReason::IssueNotOwned);

    // Creating in Alice's project as somebody else needs the flag and
    // self-assignment.
    let create = Write::IssueCreate {
        assignee: Some("somebody-else"),
        placement,
    };
    assert_eq!(
        check(&someone_else, &create, false).unwrap_err().reason,
        DenyReason::ForeignProject
    );
    assert_eq!(check(&someone_else, &create, true), Ok(()));
}
