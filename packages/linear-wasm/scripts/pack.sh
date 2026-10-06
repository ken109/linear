#!/bin/sh
# Make the npm tarball of a built package (run scripts/build.sh first).
#
# The tarball is out/ken109-linear-wasm-<version>.tgz. A Worker's repo depends
# on it by URL once it is attached to a GitHub Release:
#
#   "@ken109/linear-wasm": "https://github.com/ken109/linear/releases/download/v<version>/ken109-linear-wasm-<version>.tgz"
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"

[ -f dist/index.js ] || { echo "dist/ is missing; run scripts/build.sh first" >&2; exit 1; }
rm -rf out
mkdir out
npm pack --pack-destination out --silent
ls -l out
