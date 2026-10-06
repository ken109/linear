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
