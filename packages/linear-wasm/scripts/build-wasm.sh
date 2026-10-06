#!/bin/sh
# Compile crates/wasm to wasm32 and generate the JavaScript glue into src/wasm/.
#
# Needs the wasm32-unknown-unknown target and a wasm-bindgen-cli of the same
# version as the `wasm-bindgen` crate in Cargo.lock:
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version "$(cargo metadata --format-version 1 \
#     | jq -r '.packages[] | select(.name=="wasm-bindgen") | .version')" --locked
#
# FEATURES="panic-probe" adds the feature that exports a function which panics
# (only for test/panic.test.js). OUT overrides the output directory.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
out="${OUT:-$here/src/wasm}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

if [ -n "${FEATURES:-}" ]; then
  cargo build --manifest-path "$root/Cargo.toml" -p linear-wasm \
    --target wasm32-unknown-unknown --profile wasm-release --features "$FEATURES"
else
  cargo build --manifest-path "$root/Cargo.toml" -p linear-wasm \
    --target wasm32-unknown-unknown --profile wasm-release
fi

rm -rf "$out"
wasm-bindgen --target web --out-dir "$out" \
  "$target_dir/wasm32-unknown-unknown/wasm-release/linear_wasm.wasm"

wasm="$out/linear_wasm_bg.wasm"
echo "raw:    $(wc -c < "$wasm" | tr -d ' ') bytes"
echo "gzip:   $(gzip -9 -c "$wasm" | wc -c | tr -d ' ') bytes"
if command -v brotli >/dev/null 2>&1; then
  echo "brotli: $(brotli -q 11 -c "$wasm" | wc -c | tr -d ' ') bytes"
fi
