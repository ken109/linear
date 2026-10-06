//! The JSON Schema of everything that crosses the boundary.
//!
//! Not part of the wasm artifact: it is compiled for the host only, and
//! `cargo run -p linear-wasm --example schema` prints it. The TypeScript
//! package generates `types.ts` and `schemas.ts` from this document.
//!
//! Types are described as they are *serialized* (`Contract::Serialize`): a
//! field that is always written is required, an optional one is `T | null`.
//! Inputs that have defaults (`AuditConfig`, `RefreshMeta`, ...) therefore list
//! all of their fields as required; the TypeScript wrapper makes them partial.

use crate::api::{BuiltRequest, ErrorBody, HttpMeta};
use crate::ops::OPERATIONS;
use linear_core::audit::{AuditConfig, AuditOptions, AuditReport, Finding, Snapshot};
use linear_core::cache::WorkspaceCache;
use linear_core::refresh::{RefreshDecision, RefreshEvent, RefreshMeta};
use linear_core::webhook::Verification;
use linear_core::SCHEMA_VERSION;
use schemars::generate::SchemaSettings;
use serde_json::{json, Map, Value};

/// The document: `schemaVersion`, `operations` (each operation's `params` and
/// `data`, as `$ref`s into `$defs`) and `$defs` (every named type).
pub fn document() -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();

    let mut operations = Map::new();
    for op in OPERATIONS {
        let (params, data) = (op.schemas)(&mut generator);
        operations.insert(
            op.name.to_owned(),
            json!({ "params": params, "data": data }),
        );
    }

    // Types that are exported even though no operation names them.
    macro_rules! export {
        ($($ty:ty),+) => {$( let _ = generator.subschema_for::<$ty>(); )+};
    }
    export!(
        Snapshot,
        WorkspaceCache,
        AuditConfig,
        AuditOptions,
        AuditReport,
        Finding,
        RefreshMeta,
        RefreshEvent,
        RefreshDecision,
        Verification,
        BuiltRequest,
        HttpMeta,
        ErrorBody
    );

    let defs = generator.take_definitions(true);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "linear-wasm",
        "schemaVersion": SCHEMA_VERSION,
        "operations": operations,
        "$defs": defs,
    })
}
