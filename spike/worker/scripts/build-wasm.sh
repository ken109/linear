#!/bin/sh
# Build crates/wasm for wasm32 and generate the JS glue into src/wasm/.
# Requires: rustup target wasm32-unknown-unknown, and wasm-bindgen-cli with the
# same version as the `wasm-bindgen` crate in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <version> --locked
# Optionally runs `wasm-opt -Oz` when it is on PATH (it is not required, and it
# does not reduce the gzip size; see the notes in README.md).
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
out="${1:-$here/src/wasm}" # optional: output directory (spike/start passes its own)

cargo build --manifest-path "$root/Cargo.toml" -p linear-wasm \
  --target wasm32-unknown-unknown --profile wasm-release

rm -rf "$out"
wasm-bindgen --target web --out-dir "$out" \
  "$root/target/wasm32-unknown-unknown/wasm-release/linear_wasm.wasm"

if [ "${WASM_OPT:-0}" = "1" ] && command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals \
    -o "$out/linear_wasm_bg.wasm" "$out/linear_wasm_bg.wasm"
fi

echo "raw:  $(wc -c < "$out/linear_wasm_bg.wasm") bytes"
echo "gzip: $(gzip -9 -c "$out/linear_wasm_bg.wasm" | wc -c) bytes"
