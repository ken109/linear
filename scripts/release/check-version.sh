#!/bin/sh
# Print the release version and check that it is consistent.
#
#   check-version.sh [<tag>]
#
# The version is the Cargo workspace version, which is also what
# `linear --version` prints. packages/linear-wasm/package.json must carry the same
# version (its build fails otherwise). When a tag is given it must be exactly
# v<version>. Nothing is bumped here: a mismatch is an error.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)

workspace_version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' "$root/Cargo.toml")
package_version=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$root/packages/linear-wasm/package.json")

if [ -z "$workspace_version" ] || [ -z "$package_version" ]; then
  echo "could not read the workspace or package version" >&2
  exit 1
fi
if [ "$workspace_version" != "$package_version" ]; then
  echo "workspace version $workspace_version differs from packages/linear-wasm/package.json $package_version" >&2
  exit 1
fi
if [ "$#" -ge 1 ] && [ -n "$1" ] && [ "$1" != "v$workspace_version" ]; then
  echo "tag $1 does not match the version in Cargo.toml and package.json (v$workspace_version)" >&2
  exit 1
fi

echo "$workspace_version"
