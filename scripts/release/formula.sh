#!/bin/sh
# Render the Homebrew formula from packaging/linear.rb.in.
#
#   formula.sh <version> <sha256-macos-arm64> <sha256-macos-x86_64> <sha256-linux-x86_64> [<out-file>]
#
# The checksums are those of the archives scripts/release/package.sh made for
# aarch64-apple-darwin, x86_64-apple-darwin and x86_64-unknown-linux-musl.
# Prints to standard output when no out-file is given.
set -eu

if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
  echo "usage: $0 <version> <sha256-macos-arm64> <sha256-macos-x86_64> <sha256-linux-x86_64> [<out-file>]" >&2
  exit 2
fi
version=$1
arm64=$2
x86_64=$3
linux=$4

root=$(cd "$(dirname "$0")/../.." && pwd)
template="$root/packaging/linear.rb.in"

case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "not a version (no leading v): $version" >&2; exit 2 ;;
esac
for sum in "$arm64" "$x86_64" "$linux"; do
  case "$sum" in
    *[!0-9a-f]* | "") echo "not a sha256: $sum" >&2; exit 2 ;;
  esac
  [ "${#sum}" -eq 64 ] || { echo "not a sha256: $sum" >&2; exit 2; }
done

render() {
  sed \
    -e "s/@VERSION@/$version/g" \
    -e "s/@SHA256_MACOS_ARM64@/$arm64/g" \
    -e "s/@SHA256_MACOS_X86_64@/$x86_64/g" \
    -e "s/@SHA256_LINUX_X86_64@/$linux/g" \
    "$template"
}

if [ "$#" -eq 5 ]; then
  render > "$5"
else
  render
fi
