//! Filter inputs for list queries.
//!
//! Only the filter fields the read commands use are modelled; cynic input
//! structs may list a subset of the schema's fields. Every `Option` field
//! skips serialization when `None`, so an empty filter is `{}` and never an
//! explicit `null` (see [`crate::inputs`]).
//!
//! The builders at the bottom turn what a user types (`me`, an email, a name,
//! a state type) into a filter. They are pure, so they are tested without a
//! server.

use crate::scalars::DateTimeOrDuration;
use crate::schema;
use chrono::{DateTime, Duration, NaiveDate, Utc};

// ---------------------------------------------------------------- comparators

/// A comparator on a timestamp field that is never null.
#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "DateComparator")]
pub struct DateComparator {
    /// At or after this time.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub gte: Option<DateTimeOrDuration>,
}

/// The same comparator on a nullable timestamp (a distinct type in the schema).
#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NullableDateComparator")]
pub struct NullableDateComparator {
    /// At or after this time. A null value never matches.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub gte: Option<DateTimeOrDuration>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "StringComparator", rename_all = "camelCase")]
pub struct StringComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq_ignore_case: Option<String>,
    #[cynic(rename = "in", skip_serializing_if = "Option::is_none")]
    pub in_: Option<Vec<String>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub nin: Option<Vec<String>>,
}

impl StringComparator {
    pub fn eq(value: impl Into<String>) -> Self {
        Self {
            eq: Some(value.into()),
            ..Self::default()
        }
    }

    pub fn eq_ignore_case(value: impl Into<String>) -> Self {
        Self {
            eq_ignore_case: Some(value.into()),
            ..Self::default()
        }
    }

    pub fn any_of<S: Into<String>>(values: impl IntoIterator<Item = S>) -> Self {
        Self {
            in_: Some(values.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    pub fn none_of<S: Into<String>>(values: impl IntoIterator<Item = S>) -> Self {
        Self {
            nin: Some(values.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }
}

/// The same comparator on a nullable field (a distinct type in the schema).
#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NullableStringComparator", rename_all = "camelCase")]
pub struct NullableStringComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq_ignore_case: Option<String>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "IDComparator")]
pub struct IdComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq: Option<cynic::Id>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "EntityIdentifierIDComparator")]
pub struct EntityIdComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq: Option<cynic::Id>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "BooleanComparator")]
pub struct BooleanComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq: Option<bool>,
}

// ---------------------------------------------------------------- users

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NullableUserFilter", rename_all = "camelCase")]
pub struct UserFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_me: Option<BooleanComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub email: Option<StringComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<StringComparator>,
    /// `true` matches "no user" (unassigned, no lead).
    #[cynic(rename = "null", skip_serializing_if = "Option::is_none")]
    pub is_null: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<UserFilter>>,
}

/// Turn what a person types into a user filter.
///
/// * `me` (any case): the authenticated user
/// * `none` / `unassigned` (any case): no user at all
/// * anything with an `@`: an email address
/// * otherwise: a name or display name, ignoring case
pub fn user_filter(spec: &str) -> UserFilter {
    let spec = spec.trim();
    match spec.to_ascii_lowercase().as_str() {
        "me" => UserFilter {
            is_me: Some(BooleanComparator { eq: Some(true) }),
            ..UserFilter::default()
        },
        "none" | "unassigned" => UserFilter {
            is_null: Some(true),
            ..UserFilter::default()
        },
        _ if spec.contains('@') => UserFilter {
            email: Some(StringComparator::eq(spec)),
            ..UserFilter::default()
        },
        _ => UserFilter {
            or: Some(vec![
                UserFilter {
                    name: Some(StringComparator::eq_ignore_case(spec)),
                    ..UserFilter::default()
                },
                UserFilter {
                    display_name: Some(StringComparator::eq_ignore_case(spec)),
                    ..UserFilter::default()
                },
            ]),
            ..UserFilter::default()
        },
    }
}

// ---------------------------------------------------------------- issues

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "WorkflowStateFilter")]
pub struct StateFilter {
    #[cynic(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<StringComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "TeamFilter")]
pub struct TeamFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub key: Option<StringComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NullableProjectFilter", rename_all = "camelCase")]
pub struct IssueProjectFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntityIdComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NullableProjectMilestoneFilter")]
pub struct IssueMilestoneFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<NullableStringComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "IssueLabelFilter")]
pub struct LabelFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<IdComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparator>,
    /// The group the label is in.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent: Option<LabelGroupFilter>,
}

/// A label filter that looks at nothing but the id: the group of a label is a
/// label too, but only its identity matters here.
#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "IssueLabelFilter")]
pub struct LabelGroupFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<IdComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "IssueLabelCollectionFilter")]
pub struct LabelCollectionFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub some: Option<LabelFilter>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "AttachmentFilter")]
pub struct AttachmentFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub url: Option<StringComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "AttachmentCollectionFilter")]
pub struct AttachmentCollectionFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub some: Option<AttachmentFilter>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "IssueFilter", rename_all = "camelCase")]
pub struct IssueFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub state: Option<StateFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project: Option<IssueProjectFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub project_milestone: Option<IssueMilestoneFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<AttachmentCollectionFilter>,
    /// Every entry must match (used for repeated `--label`).
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<IssueFilter>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<IssueFilter>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub labels: Option<LabelCollectionFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<NullableDateComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub canceled_at: Option<NullableDateComparator>,
}

/// State types that mean the work is finished.
pub const CLOSED_STATE_TYPES: [&str; 2] = ["completed", "canceled"];

/// The issues an audit looks at: every open issue, and any issue updated at
/// or after `changed_since`.
///
/// A state change always updates the issue, so the second alternative catches
/// the issues that were completed or canceled recently, whose state changes
/// the staleness of a project's status update is judged against.
pub fn audit_issues(changed_since: DateTime<Utc>) -> IssueFilter {
    IssueFilter {
        or: Some(vec![
            IssueFilter {
                state: Some(StateFilter {
                    type_: Some(StringComparator::none_of(CLOSED_STATE_TYPES)),
                    name: None,
                }),
                ..IssueFilter::default()
            },
            IssueFilter {
                updated_at: Some(DateComparator {
                    gte: Some(DateTimeOrDuration(changed_since)),
                }),
                ..IssueFilter::default()
            },
        ]),
        ..IssueFilter::default()
    }
}

/// What `issue list` can be narrowed by. Unset fields do not narrow.
#[derive(Debug, Clone, Default)]
pub struct IssueQuery {
    /// `me`, `none`, an email or a name (see [`user_filter`]).
    pub assignee: Option<String>,
    /// Workflow state types (`started`, ...). Takes precedence over `open`.
    pub state_types: Vec<String>,
    /// Workflow state names, ignoring case.
    pub state_names: Vec<String>,
    /// Only issues whose state type is not completed or canceled.
    pub open: bool,
    pub team_key: Option<String>,
    /// The id of an already resolved project.
    pub project_id: Option<String>,
    pub milestone: Option<String>,
    /// Every label must be on the issue.
    pub labels: Vec<String>,
    /// An exact attachment URL.
    pub source_url: Option<String>,
    /// Issues that were completed or canceled at or after this time. Next to
    /// `open` it adds them to the open ones (the open issues, plus the ones
    /// closed since); on its own, or with a state, it narrows to them.
    pub closed_since: Option<DateTime<Utc>>,
}

impl IssueQuery {
    /// `None` when nothing narrows the listing.
    pub fn filter(&self) -> Option<IssueFilter> {
        let mut f = IssueFilter {
            assignee: self.assignee.as_deref().map(user_filter),
            ..IssueFilter::default()
        };

        // Open issues plus the ones closed since: one alternative list, so
        // the open clause must not also be ANDed in as a plain state filter.
        let open_or_recent =
            self.open && self.state_types.is_empty() && self.closed_since.is_some();
        let state_type = if !self.state_types.is_empty() {
            Some(StringComparator::any_of(self.state_types.iter().cloned()))
        } else if self.open && !open_or_recent {
            Some(StringComparator::none_of(CLOSED_STATE_TYPES))
        } else {
            None
        };
        f.state = state_type.map(|t| StateFilter {
            type_: Some(t),
            name: None,
        });

        f.team = self.team_key.as_ref().map(|k| TeamFilter {
            key: Some(StringComparator::eq_ignore_case(k.clone())),
        });
        f.project = self.project_id.as_ref().map(|id| IssueProjectFilter {
            id: Some(EntityIdComparator {
                eq: Some(cynic::Id::new(id.clone())),
            }),
        });
        f.project_milestone = self.milestone.as_ref().map(|m| IssueMilestoneFilter {
            name: Some(NullableStringComparator {
                eq_ignore_case: Some(m.clone()),
            }),
        });
        f.attachments = self
            .source_url
            .as_ref()
            .map(|u| AttachmentCollectionFilter {
                some: Some(AttachmentFilter {
                    url: Some(StringComparator::eq(u.clone())),
                }),
            });

        // `labels` can be given once per object, so further labels go into
        // `and`, every entry of which must hold.
        let mut labels = self.labels.iter();
        if let Some(first) = labels.next() {
            f.labels = Some(label_named(first));
        }
        let mut more: Vec<IssueFilter> = labels
            .map(|l| IssueFilter {
                labels: Some(label_named(l)),
                ..IssueFilter::default()
            })
            .collect();

        // Closed since: completed or canceled at or after the time (a state
        // change always sets one of the two timestamps).
        if let Some(since) = self.closed_since {
            let after = || {
                Some(NullableDateComparator {
                    gte: Some(DateTimeOrDuration(since)),
                })
            };
            let mut alternatives = vec![
                IssueFilter {
                    completed_at: after(),
                    ..IssueFilter::default()
                },
                IssueFilter {
                    canceled_at: after(),
                    ..IssueFilter::default()
                },
            ];
            if open_or_recent {
                alternatives.insert(
                    0,
                    IssueFilter {
                        state: Some(StateFilter {
                            type_: Some(StringComparator::none_of(CLOSED_STATE_TYPES)),
                            name: None,
                        }),
                        ..IssueFilter::default()
                    },
                );
            }
            more.push(IssueFilter {
                or: Some(alternatives),
                ..IssueFilter::default()
            });
        }
        if !more.is_empty() {
            f.and = Some(more);
        }

        // Any of several state names: the entries of `or` are alternatives.
        if !self.state_names.is_empty() {
            f.or = Some(
                self.state_names
                    .iter()
                    .map(|n| IssueFilter {
                        state: Some(StateFilter {
                            type_: None,
                            name: Some(StringComparator::eq_ignore_case(n.clone())),
                        }),
                        ..IssueFilter::default()
                    })
                    .collect(),
            );
        }

        (f != IssueFilter::default()).then_some(f)
    }
}

/// The moment `--completed-since` counts from: `Nd` is `N` days (of 24 hours)
/// before `now`; `YYYY-MM-DD` is the start of that day in UTC.
pub fn closed_since(spec: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    const EXPECTED: &str = "expected a number of days such as 14d, or a date such as 2026-10-01";
    let spec = spec.trim();
    if let Some(days) = spec.strip_suffix(['d', 'D']) {
        let days: u32 = days
            .parse()
            .map_err(|_| format!("{spec:?} is not a number of days; {EXPECTED}"))?;
        // About 100 years; also keeps the arithmetic far from overflow.
        if days == 0 || days > 36_500 {
            return Err(format!(
                "{spec:?} is out of range (1d to 36500d); {EXPECTED}"
            ));
        }
        return Ok(now - Duration::days(i64::from(days)));
    }
    let day: NaiveDate = spec
        .parse()
        .map_err(|e| format!("{spec:?} is not a date ({e}); {EXPECTED}"))?;
    Ok(day.and_time(chrono::NaiveTime::MIN).and_utc())
}

fn label_named(name: &str) -> LabelCollectionFilter {
    LabelCollectionFilter {
        some: Some(LabelFilter {
            name: Some(StringComparator::eq_ignore_case(name)),
            ..LabelFilter::default()
        }),
    }
}

/// The issues that carry the label with this id.
pub fn issues_with_label(label_id: &str) -> IssueFilter {
    IssueFilter {
        labels: Some(LabelCollectionFilter {
            some: Some(LabelFilter {
                id: Some(IdComparator {
                    eq: Some(cynic::Id::new(label_id)),
                }),
                ..LabelFilter::default()
            }),
        }),
        ..IssueFilter::default()
    }
}

/// The issues that carry at least one label of the group with this id.
pub fn issues_with_label_of_group(group_id: &str) -> IssueFilter {
    IssueFilter {
        labels: Some(LabelCollectionFilter {
            some: Some(LabelFilter {
                parent: Some(LabelGroupFilter {
                    id: Some(IdComparator {
                        eq: Some(cynic::Id::new(group_id)),
                    }),
                }),
                ..LabelFilter::default()
            }),
        }),
        ..IssueFilter::default()
    }
}

// ---------------------------------------------------------------- projects

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "ProjectStatusFilter")]
pub struct ProjectStatusFilter {
    #[cynic(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<StringComparator>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "InitiativeFilter", rename_all = "camelCase")]
pub struct InitiativeFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub status: Option<StringComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner: Option<UserFilter>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "InitiativeCollectionFilter")]
pub struct InitiativeCollectionFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub some: Option<InitiativeFilter>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "ProjectFilter")]
pub struct ProjectFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProjectStatusFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub lead: Option<UserFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub initiatives: Option<InitiativeCollectionFilter>,
}

/// What `project list` can be narrowed by. Unset fields do not narrow.
#[derive(Debug, Clone, Default)]
pub struct ProjectQuery {
    /// `me`, `none`, an email or a name (see [`user_filter`]).
    pub lead: Option<String>,
    /// Project status types (`started`, ...). Takes precedence over `open`.
    pub status_types: Vec<String>,
    /// Only projects whose status type is not completed or canceled.
    pub open: bool,
    /// An initiative name, ignoring case.
    pub initiative: Option<String>,
}

impl ProjectQuery {
    pub fn filter(&self) -> Option<ProjectFilter> {
        let type_ = if !self.status_types.is_empty() {
            Some(StringComparator::any_of(self.status_types.iter().cloned()))
        } else if self.open {
            Some(StringComparator::none_of(CLOSED_STATE_TYPES))
        } else {
            None
        };
        let f = ProjectFilter {
            status: type_.map(|t| ProjectStatusFilter { type_: Some(t) }),
            lead: self.lead.as_deref().map(user_filter),
            initiatives: self
                .initiative
                .as_ref()
                .map(|n| InitiativeCollectionFilter {
                    some: Some(InitiativeFilter {
                        name: Some(StringComparator::eq_ignore_case(n.clone())),
                        ..InitiativeFilter::default()
                    }),
                }),
        };
        (f != ProjectFilter::default()).then_some(f)
    }
}

// ---------------------------------------------------------------- cycles

/// A comparator on a number (`Float`) field.
#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "NumberComparator")]
pub struct NumberComparator {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub eq: Option<f64>,
}

#[derive(cynic::InputObject, Debug, Clone, Default, PartialEq)]
#[cynic(graphql_type = "CycleFilter", rename_all = "camelCase")]
pub struct CycleFilter {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamFilter>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub number: Option<NumberComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<BooleanComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_future: Option<BooleanComparator>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub is_past: Option<BooleanComparator>,
}

/// The cycles of the team with this key (ignoring case).
pub fn cycles_of_team(key: &str) -> CycleFilter {
    CycleFilter {
        team: Some(TeamFilter {
            key: Some(StringComparator::eq_ignore_case(key)),
        }),
        ..CycleFilter::default()
    }
}

/// Which of a team's cycles `cycle list` shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleState {
    /// The cycle that is running now.
    Active,
    /// Cycles that have not started.
    Upcoming,
    /// Cycles whose end has passed.
    Past,
}

/// The cycles of the team with this key, narrowed to one state when given.
pub fn cycles_in_state(key: &str, state: Option<CycleState>) -> CycleFilter {
    let yes = || Some(BooleanComparator { eq: Some(true) });
    let mut filter = cycles_of_team(key);
    match state {
        Some(CycleState::Active) => filter.is_active = yes(),
        Some(CycleState::Upcoming) => filter.is_future = yes(),
        Some(CycleState::Past) => filter.is_past = yes(),
        None => {}
    }
    filter
}

/// The cycle of the team with this key that has this number.
pub fn cycle_numbered(key: &str, number: u32) -> CycleFilter {
    CycleFilter {
        number: Some(NumberComparator {
            eq: Some(f64::from(number)),
        }),
        ..cycles_of_team(key)
    }
}

/// What `initiative list` can be narrowed by.
#[derive(Debug, Clone, Default)]
pub struct InitiativeQuery {
    /// Initiative statuses in Linear's spelling (`Active`, ...).
    pub statuses: Vec<String>,
    pub owner: Option<String>,
}

impl InitiativeQuery {
    pub fn filter(&self) -> Option<InitiativeFilter> {
        let f = InitiativeFilter {
            name: None,
            status: (!self.statuses.is_empty())
                .then(|| StringComparator::any_of(self.statuses.iter().cloned())),
            owner: self.owner.as_deref().map(user_filter),
        };
        (f != InitiativeFilter::default()).then_some(f)
    }
}
