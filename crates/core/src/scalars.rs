//! Registration of Linear's custom scalars.
//!
//! Only the scalars that the fragments and inputs actually use are registered.

use crate::schema;
use chrono::{DateTime, NaiveDate, Utc};

cynic::impl_scalar!(DateTime<Utc>, schema::DateTime);
cynic::impl_scalar!(NaiveDate, schema::TimelessDate);
cynic::impl_scalar!(serde_json::Value, schema::JSON);
