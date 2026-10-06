#!/bin/sh
# Make the release archive of one binary and its checksum.
#
#   package.sh <version> <target> <path-to-linear-binary> <out-dir>
#
# Writes <out-dir>/linear-<version>-<target>.tar.gz and a .sha256 beside it
# (in `sha256sum` format, so `shasum -a 256 -c` works from the directory). The
# archive holds one directory, linear-<version>-<target>/, with the binary,
# LICENSE and README.md; Homebrew unpacks into it automatically.
set -eu

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <version> <target> <binary> <out-dir>" >&2
  exit 2
fi
version=$1
target=$2
binary=$3
out=$4

root=$(cd "$(dirname "$0")/../.." && pwd)
name="linear-$version-$target"

[ -f "$binary" ] || { echo "binary not found: $binary" >&2; exit 1; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

mkdir "$stage/$name"
cp "$binary" "$stage/$name/linear"
chmod 755 "$stage/$name/linear"
cp "$root/LICENSE" "$root/README.md" "$stage/$name/"

tar -C "$stage" -czf "$out/$name.tar.gz" "$name"

cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
else
  shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
fi
echo "$out/$name.tar.gz"
cat "$name.tar.gz.sha256"
