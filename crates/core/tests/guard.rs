//! The ownership-based write guard.

use cynic::GraphQlResponse;
use linear_core::config::Ownership;
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

// ------------------------------------------------------------------ lenient

fn lenient(w: Write<'_>, allow_foreign: bool) -> Result<(), Denied> {
    check_with(&me(), Ownership::Lenient, &w, allow_foreign)
}

#[test]
fn strict_is_the_default_and_check_is_strict() {
    assert_eq!(Ownership::default(), Ownership::Strict);
    let w = Write::IssueCreate {
        assignee: Some(BOB),
        placement: bobs(),
    };
    assert_eq!(
        check_with(&me(), Ownership::Strict, &w, false),
        check(&me(), &w, false)
    );
    assert!(check(&me(), &w, false).is_err());
}

#[test]
fn lenient_lets_me_create_issues_for_others_anywhere() {
    for assignee in [Some(ME), Some(BOB), None] {
        for placement in [mine(), bobs(), unled(), Placement::NoProject] {
            let w = Write::IssueCreate {
                assignee,
                placement,
            };
            assert_eq!(lenient(w, false), Ok(()), "{w:?}");
        }
    }
}

#[test]
fn lenient_lets_me_change_issues_owned_by_others() {
    for assignee in [Some(BOB), None] {
        for placement in [bobs(), unled(), Placement::NoProject] {
            for moves_to in [None, Some(bobs()), Some(Placement::NoProject)] {
                let w = Write::IssueUpdate {
                    assignee,
                    placement,
                    moves_to,
                };
                assert_eq!(lenient(w, false), Ok(()), "{w:?}");
            }
        }
    }
}

#[test]
fn lenient_still_refuses_projects_that_are_not_mine() {
    for w in [
        Write::ProjectUpdate { lead: Some(BOB) },
        Write::ProjectUpdate { lead: None },
        Write::ProjectCreate { lead: Some(BOB) },
    ] {
        assert_eq!(
            lenient(w, true).unwrap_err().reason,
            DenyReason::ProjectNotLed
        );
    }
    assert_eq!(
        lenient(Write::ProjectUpdate { lead: Some(ME) }, false),
        Ok(())
    );
}

#[test]
fn canceling_needs_ownership_in_both_modes() {
    let cancel = |assignee, placement| Write::IssueCancel {
        assignee,
        placement,
    };
    for ownership in [Ownership::Strict, Ownership::Lenient] {
        let check = |w| check_with(&me(), ownership, &w, false);
        assert_eq!(check(cancel(Some(ME), bobs())), Ok(()));
        assert_eq!(check(cancel(Some(BOB), mine())), Ok(()));
        for placement in [bobs(), unled(), Placement::NoProject] {
            let d = check(cancel(Some(BOB), placement)).unwrap_err();
            assert_eq!(d.reason, DenyReason::IssueNotOwned, "{ownership}");
            assert_eq!(d.operation, Operation::IssueUpdate);
        }
    }
}

#[test]
fn a_comment_is_its_authors_to_change() {
    let update = |author| Write::CommentUpdate { author };
    let delete = |author| Write::CommentDelete { author };

    // Strict: only my own comment, for either write.
    let strict = |w| check_with(&me(), Ownership::Strict, &w, false);
    assert_eq!(strict(update(Some(ME))), Ok(()));
    assert_eq!(strict(delete(Some(ME))), Ok(()));
    for author in [Some(BOB), None] {
        for w in [update(author), delete(author)] {
            let d = strict(w).unwrap_err();
            assert_eq!(d.reason, DenyReason::CommentNotOwned, "{w:?}");
            assert_eq!(d.code(), ErrorCode::WriteDenied);
            assert_eq!(d.operation, Operation::IssueUpdate);
        }
    }

    // Lenient: anyone may edit a comment, as anyone may change an issue; deleting stays the author's.
    let lenient = |w| check_with(&me(), Ownership::Lenient, &w, false);
    assert_eq!(lenient(update(Some(BOB))), Ok(()));
    assert_eq!(lenient(update(None)), Ok(()));
    assert_eq!(lenient(delete(Some(ME))), Ok(()));
    assert_eq!(
        lenient(delete(Some(BOB))).unwrap_err().reason,
        DenyReason::CommentNotOwned
    );
    assert!(lenient(delete(None)).is_err());

    // --allow-foreign opens nothing here.
    assert!(check(&me(), &update(Some(BOB)), true).is_err());
}

// -------------------------------------------------------------------- force

fn forced(
    ownership: Ownership,
    w: Write<'_>,
    allow_foreign: bool,
    force: bool,
) -> Result<Verdict, Denied> {
    check_forced(&me(), ownership, &w, allow_foreign, force)
}

/// One write per denial reason, with who holds what it asks about.
fn refused_writes() -> Vec<(Write<'static>, DenyReason, Vec<Holder>)> {
    let holder = |role, user: Option<&str>| Holder {
        role,
        user: user.map(str::to_owned),
    };
    vec![
        (
            Write::ProjectUpdate { lead: Some(BOB) },
            DenyReason::ProjectNotLed,
            vec![holder(Role::Lead, Some(BOB))],
        ),
        (
            Write::ProjectCreate { lead: None },
            DenyReason::ProjectNotLed,
            vec![holder(Role::Lead, None)],
        ),
        (
            Write::InitiativeUpdate { owner: Some(BOB) },
            DenyReason::InitiativeNotOwned,
            vec![holder(Role::Owner, Some(BOB))],
        ),
        (
            Write::IssueCreate {
                assignee: Some(BOB),
                placement: Placement::NoProject,
            },
            DenyReason::IssueNotOwned,
            vec![holder(Role::Assignee, Some(BOB))],
        ),
        (
            Write::IssueCreate {
                assignee: Some(ME),
                placement: bobs(),
            },
            DenyReason::ForeignProject,
            vec![holder(Role::Lead, Some(BOB))],
        ),
        (
            Write::IssueUpdate {
                assignee: Some(BOB),
                placement: bobs(),
                moves_to: None,
            },
            DenyReason::IssueNotOwned,
            vec![
                holder(Role::Assignee, Some(BOB)),
                holder(Role::Lead, Some(BOB)),
            ],
        ),
        (
            Write::IssueUpdate {
                assignee: Some(ME),
                placement: Placement::NoProject,
                moves_to: Some(bobs()),
            },
            DenyReason::ForeignProject,
            vec![holder(Role::Lead, Some(BOB))],
        ),
        (
            Write::IssueCancel {
                assignee: None,
                placement: Placement::NoProject,
            },
            DenyReason::IssueNotOwned,
            vec![holder(Role::Assignee, None)],
        ),
        (
            Write::CommentUpdate { author: Some(BOB) },
            DenyReason::CommentNotOwned,
            vec![holder(Role::Author, Some(BOB))],
        ),
        (
            Write::CommentDelete { author: None },
            DenyReason::CommentNotOwned,
            vec![holder(Role::Author, None)],
        ),
    ]
}

#[test]
fn force_overrides_every_denial_and_says_what_it_overrode() {
    for (w, reason, held) in refused_writes() {
        // Strict: unforced it is refused, with the holders named; forced it goes through
        // carrying that same refusal.
        let refusal = forced(Ownership::Strict, w, false, false).unwrap_err();
        assert_eq!(refusal.reason, reason, "{w:?}");
        assert_eq!(refusal.held, held, "{w:?}");

        let verdict = forced(Ownership::Strict, w, false, true).unwrap();
        assert_eq!(verdict, Verdict::Forced(refusal.clone()), "{w:?}");
        assert_eq!(verdict.overridden(), Some(&refusal));

        // The same refusal that `check` gives: `check_forced` does not decide anything else.
        assert_eq!(check(&me(), &w, false), Err(refusal), "{w:?}");
    }
}

#[test]
fn force_reports_nothing_when_the_rules_allow_the_write() {
    let permitted = [
        Write::ProjectUpdate { lead: Some(ME) },
        Write::InitiativeUpdate { owner: Some(ME) },
        Write::IssueCreate {
            assignee: Some(BOB),
            placement: mine(),
        },
        Write::IssueUpdate {
            assignee: Some(ME),
            placement: bobs(),
            moves_to: None,
        },
        Write::IssueCancel {
            assignee: None,
            placement: mine(),
        },
        Write::CommentUpdate { author: Some(ME) },
        Write::CommentDelete { author: Some(ME) },
    ];
    for w in permitted {
        for force in [false, true] {
            let verdict = forced(Ownership::Strict, w, false, force).unwrap();
            assert_eq!(verdict, Verdict::Allowed, "{w:?} force={force}");
            assert_eq!(verdict.overridden(), None);
        }
    }
    // `--allow-foreign` already opens this one, so there is nothing to force.
    let create = Write::IssueCreate {
        assignee: Some(ME),
        placement: bobs(),
    };
    assert_eq!(
        forced(Ownership::Strict, create, true, true),
        Ok(Verdict::Allowed)
    );
}

#[test]
fn force_is_reported_in_lenient_only_where_lenient_still_refuses() {
    // Lenient already allows these, so forcing them overrides nothing.
    let relaxed = [
        Write::IssueCreate {
            assignee: Some(BOB),
            placement: bobs(),
        },
        Write::IssueUpdate {
            assignee: Some(BOB),
            placement: bobs(),
            moves_to: Some(bobs()),
        },
        Write::CommentUpdate { author: Some(BOB) },
    ];
    for w in relaxed {
        assert_eq!(
            forced(Ownership::Lenient, w, false, true),
            Ok(Verdict::Allowed),
            "{w:?}"
        );
    }

    // What lenient keeps refused: a project that is not mine, canceling, deleting a comment.
    let kept = [
        Write::ProjectUpdate { lead: Some(BOB) },
        Write::ProjectCreate { lead: Some(BOB) },
        Write::IssueCancel {
            assignee: Some(BOB),
            placement: bobs(),
        },
        Write::CommentDelete { author: Some(BOB) },
    ];
    for w in kept {
        let refusal = forced(Ownership::Lenient, w, false, false).unwrap_err();
        assert_eq!(
            forced(Ownership::Lenient, w, false, true),
            Ok(Verdict::Forced(refusal)),
            "{w:?}"
        );
    }
}

#[test]
fn a_denial_serializes_with_who_held_what() {
    let d = denied(Write::ProjectUpdate { lead: Some(BOB) }, false);
    assert_eq!(
        serde_json::to_value(&d).unwrap(),
        serde_json::json!({
            "operation": "project_update",
            "reason": "project_not_led",
            "message": d.message,
            "held": [{"role": "lead", "user": BOB}],
        })
    );
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
