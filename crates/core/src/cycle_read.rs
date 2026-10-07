//! Queries for `linear cycle list` and `linear cycle view`.
//!
//! The fixed [`Cycle`](crate::types::Cycle) fragment stays as small as it is:
//! it is what `cycle <DATE>` and `issue create --held-on` print, and it must not
//! change. A listing needs more (the state of the cycle, how far along it is,
//! its team), so it has a fragment of its own, [`CycleInfo`].

use crate::filters::CycleFilter;
use crate::nodes::paged_container;
use crate::read::IssueBrief;
use crate::schema;
use crate::types::{whole_number, PageVars, Team};
use chrono::{DateTime, Utc};
use cynic::{Operation, QueryBuilder};
use schemars::JsonSchema;
use serde::Serialize;

/// A cycle as the cycle commands show it.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cynic(graphql_type = "Cycle")]
#[serde(rename_all = "camelCase")]
pub struct CycleInfo {
    #[schemars(with = "String")]
    pub id: cynic::Id,
    /// Linear numbers cycles per team; written as an integer (`7`, not `7.0`).
    #[serde(serialize_with = "whole_number")]
    pub number: f64,
    pub name: Option<String>,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Set when the cycle was completed (possibly before `ends_at`).
    pub completed_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub is_future: bool,
    pub is_next: bool,
    pub is_past: bool,
    /// How much of the cycle's scope is done, from 0 to 1.
    pub progress: f64,
    pub team: Team,
}

/// Where a cycle is in time, in the words the commands print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CycleStatus {
    Active,
    /// The next cycle to start.
    Next,
    /// Starts later, but not next.
    Upcoming,
    Past,
}

impl CycleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Next => "next",
            Self::Upcoming => "upcoming",
            Self::Past => "past",
        }
    }
}

impl CycleInfo {
    /// `#12`, with the cycle's name when it has one: `#12 (Sprint)`.
    pub fn label(&self) -> String {
        let number = format!("#{}", self.number);
        match self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(name) => format!("{number} ({name})"),
            None => number,
        }
    }

    /// The cycle's number as a whole number.
    pub fn number_whole(&self) -> u32 {
        self.number as u32
    }

    pub fn status(&self) -> CycleStatus {
        if self.is_active {
            CycleStatus::Active
        } else if self.is_past {
            CycleStatus::Past
        } else if self.is_next {
            CycleStatus::Next
        } else {
            CycleStatus::Upcoming
        }
    }
}

// ---------------------------------------------------------------- list

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct CycleInfoListVars {
    pub first: i32,
    pub after: Option<String>,
    pub filter: Option<CycleFilter>,
}

impl CycleInfoListVars {
    pub fn new(page: PageVars, filter: Option<CycleFilter>) -> Self {
        Self {
            first: page.first,
            after: page.after,
            filter,
        }
    }
}

paged_container!(CycleInfoConnection, "CycleConnection", CycleInfo);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "CycleInfoListVars")]
pub struct CycleInfoList {
    #[arguments(first: $first, after: $after, filter: $filter)]
    pub cycles: CycleInfoConnection,
}

pub const CYCLE_INFO_PAGE_SIZE: i32 = 100;

/// The cycles that match `filter`, with what `cycle list` shows of each.
pub fn cycle_infos(vars: CycleInfoListVars) -> Operation<CycleInfoList, CycleInfoListVars> {
    CycleInfoList::build(vars)
}

// ---------------------------------------------------------------- issues of a cycle

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct CycleIssuesVars {
    pub id: String,
    pub first: i32,
    pub after: Option<String>,
}

impl CycleIssuesVars {
    pub fn new(id: impl Into<String>, page: PageVars) -> Self {
        Self {
            id: id.into(),
            first: page.first,
            after: page.after,
        }
    }
}

paged_container!(CycleIssueConnection, "IssueConnection", IssueBrief);

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Cycle", variables = "CycleIssuesVars")]
pub struct CycleIssues {
    #[arguments(first: $first, after: $after)]
    pub issues: CycleIssueConnection,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Query", variables = "CycleIssuesVars")]
pub struct CycleIssuesQuery {
    #[arguments(id: $id)]
    pub cycle: CycleIssues,
}

pub const CYCLE_ISSUES_PAGE_SIZE: i32 = 50;

/// One page of the issues in the cycle with this id.
pub fn cycle_issues(vars: CycleIssuesVars) -> Operation<CycleIssuesQuery, CycleIssuesVars> {
    CycleIssuesQuery::build(vars)
}
