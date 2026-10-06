# Spike: linear-core as wasm, called from a Worker (KK-227)

Throwaway verification, not an application. It answers one question: can the
I/O-free `linear-core` crate be compiled to wasm and called from a Cloudflare
Worker (and from a TanStack Start app on Workers), with `fetch` done on the
TypeScript side?

- `crates/wasm` exports `build_request` and `parse_response`. Everything crosses
  the boundary as JSON text (no `serde-wasm-bindgen`).
- `worker/` is a plain Worker (`wrangler dev`) with routes `/whoami`,
  `/issue/<id>`, `/projects`, `/store` (parse, then put in a local KV).
- `start/` is a minimal TanStack Start app (`@cloudflare/vite-plugin`) whose
  index route loader calls `whoami` through the same wasm.

## Run

Needs the `wasm32-unknown-unknown` target and a `wasm-bindgen-cli` whose version
equals the `wasm-bindgen` crate in `Cargo.lock`:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(cargo metadata --format-version 1 \
  | jq -r '.packages[] | select(.name=="wasm-bindgen") | .version')" --locked
```

Put a Linear API key (use a sandbox workspace) in `.dev.vars` (gitignored):

```sh
printf 'LINEAR_API_KEY=%s\n' "$KEY" > worker/.dev.vars   # and start/.dev.vars
```

```sh
cd worker && npm install && npm run dev      # http://localhost:8787/whoami
cd start  && npm install && npm run dev      # http://localhost:3000/
```

Nothing here is deployed; `wrangler dev` and `vite dev` run locally.

## Sizes (`wasm32-unknown-unknown`, `wasm-bindgen --target web`)

Measured on 2026-10-06 with `crates/wasm` exporting the two functions and four
operations. Workers limits: 3 MB free / 10 MB paid, after compression.

| build | raw | gzip -9 | brotli -q11 |
| --- | ---: | ---: | ---: |
| `wasm-release` profile (opt-level s, lto, codegen-units 1, panic abort, strip) | 333,579 | 105,698 | 86,402 |
| same + `wasm-opt -Oz` | 302,537 | 117,475 | 93,404 |
| default `--release` | 563,170 | 144,804 | 114,857 |
| default `--release` + `wasm-opt -Oz` | 382,091 | 146,230 | 113,837 |

`wrangler deploy --dry-run` (no upload) reports `Total Upload: 333.17 KiB / gzip: 106.39 KiB`
for the Worker. `wasm-opt` shrinks the raw size but not the compressed size, so
it is not needed (`WASM_OPT=1 sh worker/scripts/build-wasm.sh` runs it if you want).

## What came of it

The spike's answer was yes, and the real thing has replaced it:

- `crates/wasm` now exports six functions (`build_request`, `parse_response`, `audit`, `diff`,
  `decide_refresh`, `verify_webhook`) over one table of operations, and the typed TypeScript
  package built from it is in `packages/linear-wasm`. The two apps here follow the new envelope
  (`{ ok, data }` instead of `{ ok, request }`) but are no longer maintained.
- A panic aborts the module and reaches JavaScript as `RuntimeError: unreachable`; a small hook
  keeps the message for `last_panic()` (see `crates/wasm/src/lib.rs`).

Sizes after `audit`, `diff`, `decide_refresh`, `verify_webhook` and 15 operations were added
(`wasm-release`, no `wasm-opt`, measured 2026-10-06 with `packages/linear-wasm/scripts/build-wasm.sh`):

| build | raw | gzip -9 | brotli -q11 |
| --- | ---: | ---: | ---: |
| this spike (2 functions, 4 operations) | 333,579 | 105,698 | 86,402 |
| 6 functions, 15 operations, panic hook | 666,956 | 188,240 | 144,734 |

Where it went, from builds that leave functions out (before the upstream changes that added
issue fields, so the totals are lower than the table): `audit` + `diff` about +160 KB raw / +45 KB
gzip, `decide_refresh` +23 KB / +7 KB, `verify_webhook` (HMAC-SHA256) +19 KB / +6 KB, and the 11
further operations with their types and the panic hook about +103 KB / +13 KB. At 188 KB gzipped
it is 6% of the Workers free-plan limit (3 MB).
