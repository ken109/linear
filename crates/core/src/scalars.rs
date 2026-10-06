//! Registration of Linear's custom scalars.
//!
//! Only the scalars that the fragments and inputs actually use are registered.

use crate::schema;
use chrono::{DateTime, NaiveDate, Utc};

cynic::impl_scalar!(DateTime<Utc>, schema::DateTime);

/// A comparison value for a timestamp field. Linear also accepts an ISO 8601
/// duration here; only timestamps are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DateTimeOrDuration(pub DateTime<Utc>);

cynic::impl_scalar!(DateTimeOrDuration, schema::DateTimeOrDuration);
cynic::impl_scalar!(NaiveDate, schema::TimelessDate);
cynic::impl_scalar!(serde_json::Value, schema::JSON);
