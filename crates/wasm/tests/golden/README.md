# Golden files

Each file is one call across the wasm boundary: an `input` and the `expected` answer
(`{"ok": true, "data": ...}` or `{"ok": false, "error": ...}`), in a directory named for the
function:

| Directory  | Function          | `input`                                                       |
| ---------- | ----------------- | ------------------------------------------------------------- |
| `audit/`   | `audit`           | `snapshot`, `config`, `options` (null for none), `now`        |
| `diff/`    | `diff`            | `previous` (null for none), `current`                         |
| `refresh/` | `decide_refresh`  | `meta` (null for no cache), `event`, `now`                    |
| `webhook/` | `verify_webhook`  | `body`, `signature`, `secret`, `now`                          |
| `parse/`   | `parse_response`  | `operation`, a core `fixture` (or a literal `body`), `status`, optional `retryAfterSecs` / `rateLimitResetMs`, `now` |
| `build/`   | `build_request`   | `operation`, `params` (null for none)                         |

`now` is an RFC 3339 time; the boundary takes epoch milliseconds, which both runners derive from it.
A built request is kept as its URL, variables and the first line of its query (the rest of the
query text changes whenever a fragment gains a field and says nothing about whether the two sides
agree).

## Who checks them

- `crates/wasm/tests/golden.rs` (in `cargo test --workspace`) runs every input natively and compares:
  this is the native core, what the CLI runs. It also runs the `audit` cases through
  `linear_core::audit::audit_scoped` directly, the function `linear audit` calls, and checks that
  every audit rule fires in at least one case.
- `packages/linear-wasm/test/golden.test.ts` (in `npm test`) runs the same inputs through the wasm
  module in Node and requires the identical answer. It also checks that the generated zod schemas
  accept every input the core accepts and every answer, and reject the inputs the core refuses
  (a snapshot without its fields, an unknown config key, an unknown event, ...).
- `crates/cli/tests/golden_audit.rs` (in `cargo test --workspace`) runs the `audit` cases through
  the real `linear` binary: a mock Linear serves the case's snapshot, the case's config becomes a
  `workspaces.toml`, and `linear audit --now <now>` must print the golden findings. The hidden
  `--now` option pins the clock that `linear audit` otherwise reads. The cases that expect an
  error are about the JSON boundary and have no CLI counterpart, so they are skipped.

## Changing them

When a type that crosses the boundary changes shape, or an audit rule changes what it says, these
fail on purpose. Remake the answers from the native core and read the diff:

```sh
UPDATE_GOLDEN=1 cargo test -p linear-wasm --test golden
git diff crates/wasm/tests/golden
```

If the inputs themselves no longer parse (a type gained a required field), update the builders in
`scripts/golden-inputs.py` and follow its header.
