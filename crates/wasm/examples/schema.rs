//! Print the JSON Schema of the wasm boundary: `cargo run -p linear-wasm --example schema [PATH]`.
//!
//! Without a path the document goes to stdout.

fn main() {
    let mut text = serde_json::to_string_pretty(&linear_wasm::schema::document())
        .expect("a schema always serializes");
    text.push('\n');
    match std::env::args().nth(1) {
        Some(path) => std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}")),
        None => print!("{text}"),
    }
}
