//! The pieces issue writes are built from: inputs, the write-context queries
//! and the reorder plan.

use chrono::{TimeZone, Utc};
use linear_core::inputs::*;
use linear_core::metadata::AttachmentMetadata;
use linear_core::read::{self, AttachmentsForUrlQuery, IssueWriteView};
use linear_core::reorder::{plan, OrderChange, OrderRow};
use linear_core::wire::{build_request, parse_response, ResponseMeta};
use serde_json::json;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let meta = ResponseMeta {
        status: 200,
        ..Default::default()
    };
    let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
    parse_response(&meta, &fixture(name), now).unwrap()
}

// ------------------------------------------------------------------ inputs

#[test]
fn a_milestone_patch_can_keep_clear_or_set() {
    let keep = serde_json::to_value(IssueUpdateInput::default()).unwrap();
    assert_eq!(keep, json!({}), "an untouched field is omitted, never null");

    let clear = serde_json::to_value(IssueUpdateInput {
        project_milestone_id: Patch::Clear,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(clear, json!({ "projectMilestoneId": null }));

    let set = serde_json::to_value(IssueUpdateInput {
        project_milestone_id: Patch::Set("m-1".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(set, json!({ "projectMilestoneId": "m-1" }));
}

#[test]
fn an_update_input_knows_whether_it_changes_anything() {
    assert!(IssueUpdateInput::default().is_empty());
    for input in [
        IssueUpdateInput {
            state_id: Some("s".into()),
            ..Default::default()
        },
        IssueUpdateInput {
            project_milestone_id: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            sort_order: Some(1.5),
            ..Default::default()
        },
    ] {
        assert!(!input.is_empty());
    }
}

#[test]
fn the_fields_an_issue_update_can_clear_say_so_with_an_explicit_null() {
    let keep = serde_json::to_value(IssueUpdateInput::default()).unwrap();
    assert_eq!(keep, json!({}));

    let clear = serde_json::to_value(IssueUpdateInput {
        description: Patch::Clear,
        assignee_id: Patch::Clear,
        project_id: Patch::Clear,
        due_date: Patch::Clear,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        clear,
        json!({ "description": null, "assigneeId": null, "projectId": null, "dueDate": null })
    );

    let set = serde_json::to_value(IssueUpdateInput {
        description: Patch::Set("Body".into()),
        assignee_id: Patch::Set("u".into()),
        project_id: Patch::Set("p".into()),
        due_date: Patch::Set(chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
        label_ids: Some(vec!["l1".into(), "l2".into()]),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        set,
        json!({
            "description": "Body", "assigneeId": "u", "projectId": "p",
            "dueDate": "2026-12-01", "labelIds": ["l1", "l2"],
        })
    );
    // No labels is an empty list, which removes them all; not leaving them alone.
    let none = serde_json::to_value(IssueUpdateInput {
        label_ids: Some(vec![]),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(none, json!({ "labelIds": [] }));
}

#[test]
fn priority_estimate_parent_and_cycle_of_an_update_can_be_set_and_the_last_three_cleared() {
    let set = serde_json::to_value(IssueUpdateInput {
        priority: Some(0),
        estimate: Patch::Set(5),
        parent_id: Patch::Set("i".into()),
        cycle_id: Patch::Set("c".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        set,
        json!({ "priority": 0, "estimate": 5, "parentId": "i", "cycleId": "c" })
    );
    let clear = serde_json::to_value(IssueUpdateInput {
        estimate: Patch::Clear,
        parent_id: Patch::Clear,
        cycle_id: Patch::Clear,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        clear,
        json!({ "estimate": null, "parentId": null, "cycleId": null })
    );
    for input in [
        IssueUpdateInput {
            priority: Some(0),
            ..Default::default()
        },
        IssueUpdateInput {
            estimate: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            parent_id: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            cycle_id: Patch::Clear,
            ..Default::default()
        },
    ] {
        assert!(!input.is_empty());
    }
}

#[test]
fn every_field_of_an_issue_update_counts_towards_it_changing_something() {
    for input in [
        IssueUpdateInput {
            description: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            assignee_id: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            project_id: Patch::Set("p".into()),
            ..Default::default()
        },
        IssueUpdateInput {
            due_date: Patch::Clear,
            ..Default::default()
        },
        IssueUpdateInput {
            label_ids: Some(vec![]),
            ..Default::default()
        },
    ] {
        assert!(!input.is_empty());
    }
}

#[test]
fn the_ordering_values_are_sent_as_numbers() {
    let v = serde_json::to_value(IssueUpdateInput {
        sort_order: Some(-12.5),
        priority_sort_order: Some(3.0),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(v, json!({ "sortOrder": -12.5, "prioritySortOrder": 3.0 }));
}

#[test]
fn a_create_input_omits_what_was_not_given() {
    let only_required = serde_json::to_value(IssueCreateInput {
        team_id: "t".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(only_required, json!({ "teamId": "t" }));

    let full = serde_json::to_value(IssueCreateInput {
        team_id: "t".into(),
        title: Some("Title".into()),
        description: Some("Body".into()),
        assignee_id: Some("u".into()),
        project_id: Some("p".into()),
        project_milestone_id: Some("m".into()),
        label_ids: Some(vec!["l1".into(), "l2".into()]),
        cycle_id: Some("c".into()),
        priority: Some(2),
        estimate: Some(3),
        parent_id: Some("i".into()),
    })
    .unwrap();
    assert_eq!(
        full,
        json!({
            "teamId": "t", "title": "Title", "description": "Body", "assigneeId": "u",
            "projectId": "p", "projectMilestoneId": "m", "labelIds": ["l1", "l2"],
            "cycleId": "c", "priority": 2, "estimate": 3, "parentId": "i",
        })
    );
    // Priority 0 is a value (no priority), not "left out".
    let none = serde_json::to_value(IssueCreateInput {
        team_id: "t".into(),
        priority: Some(0),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(none, json!({ "teamId": "t", "priority": 0 }));
}

#[test]
fn the_mutations_name_the_right_fields() {
    let create = build_request(&issue_create(IssueCreateInput {
        team_id: "t".into(),
        ..Default::default()
    }));
    assert!(create.query.contains("issueCreate"), "{}", create.query);
    assert_eq!(create.variables["input"], json!({ "teamId": "t" }));

    let delete = build_request(&issue_delete("i-1"));
    assert!(delete.query.contains("issueDelete"), "{}", delete.query);
    assert_eq!(delete.variables["id"], "i-1");

    let attach = build_request(&attachment_create(AttachmentCreateInput {
        issue_id: "i-1".into(),
        url: "https://example.com/a".into(),
        title: "Source".into(),
        subtitle: None,
        metadata: None,
    }));
    assert!(attach.query.contains("attachmentCreate"));
    assert_eq!(
        attach.variables["input"],
        json!({ "issueId": "i-1", "url": "https://example.com/a", "title": "Source" })
    );

    let meta =
        AttachmentMetadata::from_pairs(&["kind=slack", "ticket=42", "ratio=0.5", "id=str:42"])
            .unwrap();
    let attach = build_request(&attachment_create(AttachmentCreateInput {
        issue_id: "i-1".into(),
        url: "https://example.com/a".into(),
        title: "Source".into(),
        subtitle: Some("from Slack".into()),
        metadata: Some(meta),
    }));
    assert_eq!(
        attach.variables["input"],
        json!({
            "issueId": "i-1", "url": "https://example.com/a", "title": "Source",
            "subtitle": "from Slack",
            "metadata": { "kind": "slack", "ticket": 42, "ratio": 0.5, "id": "42" },
        })
    );

    let comment = build_request(&comment_create(CommentCreateInput {
        issue_id: "i-1".into(),
        body: "hi".into(),
    }));
    assert!(comment.query.contains("commentCreate"));
    assert_eq!(comment.variables["input"]["body"], "hi");
}

// ------------------------------------------------------------------ queries

#[test]
fn the_write_view_carries_ownership_and_names() {
    let req = build_request(&read::issue_write_view("EX-23"));
    assert!(req.query.contains("write: issue(id: $id)"), "{}", req.query);

    let d: IssueWriteView = parse("issue_write_view");
    assert_eq!(d.issue.identifier, "EX-23");
    assert!(d.issue.sort_order < 0.0 && d.issue.priority_sort_order < 0.0);
    let states: Vec<&str> = d
        .write
        .team
        .states
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(states, ["Backlog", "Todo", "In Progress", "Done"]);
    let project = d.write.project.expect("the issue is in a project");
    assert_eq!(project.name, "Fixture Project");
    assert_eq!(project.project_milestones.len(), 1);
    assert!(project.lead.as_ref().unwrap().is_me);
}

#[test]
fn an_origin_lookup_returns_the_issue_that_carries_the_url() {
    let req = build_request(&read::attachments_for_url("https://example.com/s"));
    assert!(req.query.contains("attachmentsForURL"), "{}", req.query);
    assert_eq!(req.variables["url"], "https://example.com/s");

    let hit: AttachmentsForUrlQuery = parse("attachments_for_url");
    assert_eq!(hit.attachments_for_url.nodes[0].issue.identifier, "EX-23");
    assert_eq!(hit.attachments_for_url.nodes[0].title, "Origin");
    assert!(hit.attachments_for_url.nodes[0].metadata.is_empty());
    let none: AttachmentsForUrlQuery = parse("attachments_for_url_none");
    assert!(none.attachments_for_url.nodes.is_empty());
}

// ------------------------------------------------------------------ reorder

fn row(id: &str, sort: f64, priority: f64) -> OrderRow {
    OrderRow {
        identifier: id.into(),
        sort_order: sort,
        priority_sort_order: priority,
    }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn reordering_hands_the_held_values_out_in_the_requested_order() {
    let rows = [
        row("A", 1.0, 10.0),
        row("B", 2.0, 20.0),
        row("C", 3.0, 30.0),
    ];
    let changes = plan(&rows, &ids(&["C", "A", "B"])).unwrap();
    assert_eq!(
        changes,
        vec![
            OrderChange {
                identifier: "C".into(),
                sort_order: Some(1.0),
                priority_sort_order: Some(10.0)
            },
            OrderChange {
                identifier: "A".into(),
                sort_order: Some(2.0),
                priority_sort_order: Some(20.0)
            },
            OrderChange {
                identifier: "B".into(),
                sort_order: Some(3.0),
                priority_sort_order: Some(30.0)
            },
        ]
    );
}

#[test]
fn issues_in_between_keep_their_place() {
    // X sits between A and C and is not named: the values of A and C are only swapped.
    let rows = [row("A", 1.0, 1.0), row("X", 2.0, 2.0), row("C", 3.0, 3.0)];
    let changes = plan(&rows, &ids(&["C", "A"])).unwrap();
    let sorts: Vec<_> = changes.iter().map(|c| c.sort_order).collect();
    assert_eq!(sorts, [Some(1.0), Some(3.0)]);
    assert!(changes.iter().all(|c| c.identifier != "X"));
}

#[test]
fn both_orders_are_planned_independently() {
    // Already right for sortOrder, wrong for prioritySortOrder: only the latter is written.
    let rows = [row("A", 1.0, 20.0), row("B", 2.0, 10.0)];
    let changes = plan(&rows, &ids(&["A", "B"])).unwrap();
    assert_eq!(
        changes,
        vec![
            OrderChange {
                identifier: "A".into(),
                sort_order: None,
                priority_sort_order: Some(10.0)
            },
            OrderChange {
                identifier: "B".into(),
                sort_order: None,
                priority_sort_order: Some(20.0)
            },
        ]
    );
}

#[test]
fn an_order_that_is_already_right_plans_nothing() {
    let rows = [row("A", 1.0, 1.0), row("B", 2.0, 2.0)];
    assert!(plan(&rows, &ids(&["A", "B"])).unwrap().is_empty());
}

#[test]
fn tied_values_are_spread_evenly() {
    let rows = [row("A", 5.0, 5.0), row("B", 5.0, 5.0), row("C", 5.0, 5.0)];
    // C already holds 5.0, the first value of the spread, so only B and A are written.
    let changes = plan(&rows, &ids(&["C", "B", "A"])).unwrap();
    let written: Vec<_> = changes
        .iter()
        .map(|c| (c.identifier.as_str(), c.sort_order.unwrap()))
        .collect();
    assert_eq!(written, [("B", 5.5), ("A", 6.0)]);
}

#[test]
fn bad_requests_are_usage_errors() {
    let rows = [row("A", 1.0, 1.0), row("B", 2.0, 2.0)];
    for (wanted, what) in [
        (ids(&["A"]), "at least two"),
        (ids(&["A", "A"]), "more than once"),
        (ids(&["A", "Z"]), "no such issue: Z"),
    ] {
        let err = plan(&rows, &wanted).unwrap_err();
        assert_eq!(err.code(), linear_core::ErrorCode::Usage);
        assert!(err.to_string().contains(what), "{err}");
    }
}

// ------------------------------------------------------------------ names

#[test]
fn a_state_is_found_by_name_ignoring_case_or_by_id() {
    use linear_core::matching::match_state;
    let d: IssueWriteView = parse("issue_write_view");
    let states = &d.write.team.states;
    assert_eq!(match_state(states, "done").unwrap().name, "Done");
    assert_eq!(
        match_state(states, "In Progress").unwrap().name,
        "In Progress"
    );
    let by_id = match_state(states, "00000000-0000-4000-8000-000000000021").unwrap();
    assert_eq!(by_id.name, "Todo");

    let err = match_state(states, "Shipped").unwrap_err();
    assert_eq!(err.code(), linear_core::ErrorCode::Usage);
    assert!(
        err.to_string().contains("Backlog, Todo, In Progress, Done"),
        "{err}"
    );
}
