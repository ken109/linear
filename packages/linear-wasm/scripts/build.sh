#!/bin/sh
# Build the package into dist/: schema -> TypeScript -> wasm -> tsc.
#
# The release workflow runs this and then scripts/pack.sh. It fails if the
# package version differs from the workspace version, so that the tarball and
# the CLI of one release carry the same number.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
cd "$here"

workspace_version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' "$root/Cargo.toml")
package_version=$(node -p "require('./package.json').version")
if [ "$workspace_version" != "$package_version" ]; then
  echo "package.json is version $package_version but the workspace is $workspace_version" >&2
  exit 1
fi

cargo run -q --manifest-path "$root/Cargo.toml" -p linear-wasm --example schema > schema.json
node scripts/generate.mjs
sh scripts/build-wasm.sh

rm -rf dist
npx tsc -p tsconfig.build.json
mkdir -p dist/wasm
cp src/wasm/linear_wasm.js src/wasm/linear_wasm.d.ts src/wasm/linear_wasm_bg.wasm \
  src/wasm/linear_wasm_bg.wasm.d.ts dist/wasm/
echo "built dist/ ($package_version, schema version $(node -p "require('./dist/types.js').SCHEMA_VERSION"))"
