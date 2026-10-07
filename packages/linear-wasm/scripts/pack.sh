#!/bin/sh
# Make the npm tarball of a built package (run scripts/build.sh first).
#
# The tarball is out/linear-wasm-<version>.tgz. A Worker's repo depends
# on it by URL once it is attached to a GitHub Release:
#
#   "@ken109/linear-wasm": "https://github.com/ken109/linear/releases/download/v<version>/linear-wasm-<version>.tgz"
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"

[ -f dist/index.js ] || { echo "dist/ is missing; run scripts/build.sh first" >&2; exit 1; }
rm -rf out
mkdir out
# npm names a scoped package's tarball <scope>-<name>-<version>.tgz; the scope is noise
# on a Release page, so the file is renamed.
version=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' package.json)
packed=$(npm pack --pack-destination out --silent)
mv "out/$packed" "out/linear-wasm-$version.tgz"
ls -l out
