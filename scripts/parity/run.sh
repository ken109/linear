#!/bin/bash
# Run the old-tool vs Rust-CLI parity scenarios against the sandbox workspace.
# Usage: scripts/parity/run.sh [--report FILE] [--keep] [--cleanup-only]
#                              [--old-tools-dir DIR] [--linear-bin FILE]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

if [ -z "${LINEAR_API_KEY_SANDBOX:-}" ]; then
  # The sandbox key lives outside the repo; load it without printing it.
  set -a
  . "${LINEAR_SANDBOX_ENV:-$HOME/.config/linear-dev/sandbox.env}"
  set +a
fi

if [ ! -x "$repo/target/debug/linear" ] && [ -z "${PARITY_LINEAR_BIN:-}" ]; then
  (cd "$repo" && cargo build -q)
fi

exec python3 "$here/parity.py" "$@"
