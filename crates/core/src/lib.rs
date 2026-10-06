//! Pure logic for the `linear` CLI.
//!
//! This crate performs no I/O: it never reads the clock, the file system or the
//! network. Callers fetch and persist; this crate builds requests, interprets
//! responses and decides. Anything time-dependent takes `now` as an argument.

pub mod audit;
pub mod auth;
pub mod brief;
pub mod cache;
pub mod config;
pub mod cycle;
pub mod document;
pub mod error;
pub mod filters;
pub mod guard;
pub mod inputs;
pub mod markdown;
pub mod matching;
pub mod metadata;
pub mod project_write;
pub mod queries;
pub mod read;
pub mod refresh;
pub mod reorder;
pub mod retry;
pub mod rules;
pub mod scalars;
pub mod template;
pub mod types;
pub mod webhook;
pub mod wire;
pub mod workspace;

/// The Linear schema, registered by `build.rs` from `schema/linear.graphql`.
#[cynic::schema("linear")]
pub mod schema {}

mod nodes;

pub use error::{Error, ErrorCode, Result};
pub use workspace::InWorkspace;

/// The version of every JSON shape this crate emits or accepts across a
/// boundary (cache files, `--json`, the WebAssembly package). Bump it whenever
/// one of those shapes changes; a reader that finds another version must not
/// trust what it read.
pub const SCHEMA_VERSION: u32 = 2;

/// The default Linear GraphQL endpoint.
pub const API_URL: &str = "https://api.linear.app/graphql";
