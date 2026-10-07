#!/bin/sh
# Make the release archive of one binary and its checksum.
#
#   package.sh <version> <target> <path-to-linear-binary> <out-dir>
#
# Writes <out-dir>/linear-<version>-<target>.tar.gz and a .sha256 beside it
# (in `sha256sum` format, so `shasum -a 256 -c` works from the directory). The
# archive holds one directory, linear-<version>-<target>/, with the binary,
# LICENSE and README.md; Homebrew unpacks into it automatically.
#
# For a Windows target the binary is linear.exe and the archive is a .zip
# (made with `zip`, or else `7z`, which the Windows runners have), not a .tar.gz.
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
case "$target" in
  *windows*) exe=linear.exe; ext=zip ;;
  *) exe=linear; ext=tar.gz ;;
esac
archive="$name.$ext"

[ -f "$binary" ] || { echo "binary not found: $binary" >&2; exit 1; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

mkdir "$stage/$name"
cp "$binary" "$stage/$name/$exe"
chmod 755 "$stage/$name/$exe"
cp "$root/LICENSE" "$root/README.md" "$stage/$name/"

if [ "$ext" = zip ]; then
  rm -f "$out/$archive"
  if command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -qr "$out/$archive" "$name")
  elif command -v 7z >/dev/null 2>&1; then
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "$out/$archive" "$name")
  else
    echo "neither zip nor 7z is installed" >&2
    exit 1
  fi
else
  tar -C "$stage" -czf "$out/$archive" "$name"
fi

cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$archive" > "$archive.sha256"
else
  shasum -a 256 "$archive" > "$archive.sha256"
fi
echo "$out/$archive"
cat "$archive.sha256"
