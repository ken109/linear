#!/usr/bin/env bash
# Refresh schema/linear.graphql from Linear's published SDL.
#
#   scripts/update-schema.sh            # fetch and overwrite if it changed
#   LINEAR_SCHEMA_URL=<url> scripts/update-schema.sh   # fetch from somewhere else
#
# Exits 0 whether or not the file changed (check `git diff`); non-zero if the
# download fails or does not look like Linear's schema, in which case the
# vendored file is left untouched.
set -euo pipefail

URL="${LINEAR_SCHEMA_URL:-https://raw.githubusercontent.com/linear/linear/master/packages/sdk/src/schema.graphql}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="$ROOT/schema/linear.graphql"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

curl --fail --silent --show-error --location --retry 3 --output "$tmp" "$URL"

# Refuse an error page, a truncated download or the wrong file.
size="$(wc -c <"$tmp" | tr -d ' ')"
if [ "$size" -lt 500000 ]; then
  echo "error: downloaded schema is only $size bytes; refusing to use it" >&2
  exit 1
fi
for root in Query Mutation; do
  if ! grep -q "^type $root " "$tmp"; then
    echo "error: downloaded schema has no \`type $root\`; refusing to use it" >&2
    exit 1
  fi
done

if cmp -s "$tmp" "$TARGET"; then
  echo "schema/linear.graphql is up to date"
else
  cp "$tmp" "$TARGET"
  echo "schema/linear.graphql updated ($size bytes)"
fi
