//! WebAssembly boundary for `linear-core`.
//!
//! Everything crosses the boundary as JSON text: strings in, strings out. That
//! keeps the exported surface to a handful of `fn(&str, ...) -> String`
//! functions, independent of the Rust types behind them, and avoids
//! `serde-wasm-bindgen`. The same shapes are what `--json` and the cache hold.
//!
//! The caller (TypeScript) performs the HTTP request itself: it asks
//! [`build_request`] what to send, sends it, then hands the status, headers and
//! body to [`parse_response`]. The other functions decide: [`audit`] and
//! [`diff`] over data the caller has fetched, [`decide_refresh`] about its
//! cache, [`verify_webhook`] about a delivery. None reads the clock or does I/O.
//!
//! Every function answers `{"ok":true,"data":...}` or
//! `{"ok":false,"error":{"code","message",...}}`. The logic is in [`api`], as
//! ordinary functions; the exports below only forward to it.

pub mod api;
pub mod ops;
#[cfg(not(target_arch = "wasm32"))]
pub mod schema;

use wasm_bindgen::prelude::*;

/// The version of every JSON shape that crosses this boundary.
#[wasm_bindgen]
pub fn schema_version() -> u32 {
    api::schema_version()
}

/// Build the GraphQL request for a named operation (see `ops::OPERATIONS`).
#[wasm_bindgen]
pub fn build_request(operation: &str, params_json: &str) -> String {
    api::build_request(operation, params_json)
}

/// Interpret Linear's response to a request `build_request` made.
#[wasm_bindgen]
pub fn parse_response(operation: &str, meta_json: &str, body: &str, now_ms: f64) -> String {
    api::parse_response(operation, meta_json, body, now_ms)
}

/// Run the audit rules over a snapshot.
#[wasm_bindgen]
pub fn audit(
    snapshot_json: &str,
    config_json: &str,
    now_ms: f64,
    options_json: Option<String>,
) -> String {
    api::audit(snapshot_json, config_json, now_ms, options_json.as_deref())
}

/// The findings in the current report that the previous one did not have.
#[wasm_bindgen]
pub fn diff(previous_json: &str, current_json: &str) -> String {
    api::diff(previous_json, current_json)
}

/// Whether a cached snapshot should be refreshed now.
#[wasm_bindgen]
pub fn decide_refresh(meta_json: &str, event_json: &str, now_ms: f64) -> String {
    api::decide_refresh(meta_json, event_json, now_ms)
}

/// Check a webhook delivery's signature and timestamp.
#[wasm_bindgen]
pub fn verify_webhook(body: &str, signature: &str, secret: &str, now_ms: f64) -> String {
    api::verify_webhook(body, signature, secret, now_ms)
}

// ------------------------------------------------------------------ panics
//
// The wasm build aborts on panic, which JavaScript sees as
// `RuntimeError: unreachable` with no message. A hook keeps the message (and
// the source location) so the caller can fetch it with `last_panic` and
// attach it to the error it throws. This costs about 1.4 KB of raw wasm
// (0.5 KB gzipped); `console_error_panic_hook` would cost 3.4 KB and still
// leave the thrown error empty. Nothing in the audited paths is expected to
// panic: inputs are validated and reported as `usage` errors.

static LAST_PANIC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Runs once when the module is instantiated.
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(info.to_string());
        }
    }));
}

/// The message of the panic that last aborted a call, once. Call it after a
/// `RuntimeError`; `undefined` means the call failed for another reason.
#[wasm_bindgen]
pub fn last_panic() -> Option<String> {
    LAST_PANIC.lock().ok().and_then(|mut last| last.take())
}

#[cfg(feature = "panic-probe")]
#[wasm_bindgen]
pub fn probe_panic() {
    let v: Vec<u8> = Vec::new();
    let i = v.len() + 3;
    let _ = v[i]; // out of bounds
}

#[cfg(feature = "panic-probe")]
#[wasm_bindgen]
pub fn probe_fine() -> u32 {
    42
}
