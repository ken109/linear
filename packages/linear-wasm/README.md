# @ken109/linear-wasm

The pure core of the [`linear`](../../README.md) CLI, compiled to WebAssembly, for a
Cloudflare Worker (or any JavaScript runtime) that reads Linear. It builds requests,
interprets responses, runs the `audit` rules, decides when a cache is stale and checks
webhook signatures. It does no I/O of its own: your code does the `fetch` and the storage,
and passes `now` in.

Everything that crosses the wasm boundary is JSON text. This package turns that into typed
calls, with TypeScript types and [zod](https://zod.dev) schemas generated from the Rust types.

## Install

The package is attached to each GitHub Release as a tarball. A Worker's repo depends on it by URL:

```json
{
  "dependencies": {
    "@ken109/linear-wasm": "https://github.com/ken109/linear/releases/download/v0.1.0/ken109-linear-wasm-0.1.0.tgz",
    "zod": "^4"
  }
}
```

`zod` is only needed if you import `@ken109/linear-wasm/schemas`. The package version is the
release version; `SCHEMA_VERSION` (exported) is the version of every JSON shape in it, and the
wasm module refuses to start under types of another one.

## Use

```ts
import { linear } from "@ken109/linear-wasm/workers"; // Workers: the bundler loads the .wasm

const data = await linear.send("whoami", undefined, { authorization: env.LINEAR_API_KEY });
data.viewer.displayName; // typed

// Or do the fetch yourself:
const request = linear.buildRequest("issue", { id: "KK-12" }); // { url, body }
const res = await fetch(request.url, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: env.LINEAR_API_KEY },
  body: request.body,
});
const { issue } = linear.parseResponse("issue", {
  status: res.status,
  headers: res.headers,
  body: await res.text(),
});
```

Anywhere else, give `createLinear` the module (or its bytes):

```ts
import { createLinear } from "@ken109/linear-wasm";
import { readFileSync } from "node:fs";

const wasmUrl = new URL(import.meta.resolve("@ken109/linear-wasm/linear.wasm"));
const linear = createLinear(readFileSync(wasmUrl));
```

| Call | What it does |
| --- | --- |
| `buildRequest(op, params?)` | What to POST to Linear for an operation. |
| `parseResponse(op, response, now?)` | The operation's result, or a thrown `LinearError`. |
| `send(op, params, { authorization })` | Both, around a `fetch`. |
| `audit(snapshot, config?, { now?, scope? })` | The `audit` rules over data you fetched. |
| `diff(previousReport, currentReport)` | The findings that are new since the last audit. |
| `decideRefresh(meta, event, now?)` | Whether a cached snapshot should be refreshed now. |
| `verifyWebhook(body, signature, secret, now?)` | Checks `Linear-Signature` and the timestamp. |

The operations are the keys of `Operations` (`whoami`, `issue`, `issue_view`, `projects`,
`project_view`, `assigned_started_issues`, ...); each has typed parameters and a typed result in the
shape of Linear's own response. Calls throw `LinearError` (with `code`, and `retryAfterSecs` or
`apiCode` when there is one) for anything the wasm side refuses or Linear rejects. A webhook that
fails verification is not an error: `verifyWebhook` returns `{ status: "invalid", reason }`.

If the wasm module itself panics (a bug, not a bad input) the call throws `LinearPanic`, whose
message is the panic message and location. Treat that instance as suspect and report it.

### Validating at run time

`@ken109/linear-wasm/schemas` has a zod schema for every type, and the values of every enum:

```ts
import { workspaceCacheSchema, ruleIdValues } from "@ken109/linear-wasm/schemas";

const entry = workspaceCacheSchema.parse(JSON.parse(await env.KV.get("cache:ken109") ?? "null"));
```

Enums Linear may extend (project status, health, ...) are plain strings in both the types and
the schemas, with the values known today in `<name>KnownValues`: a new value from Linear does
not fail a parse.

`schema.json` is the JSON Schema of the same types, for readers that are not TypeScript.

## Build

Needs the `wasm32-unknown-unknown` target and a `wasm-bindgen-cli` of the version in `Cargo.lock`
(see `scripts/build-wasm.sh`), and Node 24.

```sh
npm ci
npm run build   # schema.json -> types.ts, schemas.ts -> wasm -> dist/
npm test        # the wasm through the typed wrapper, and the golden files
npm run pack    # out/ken109-linear-wasm-<version>.tgz
```

`schema.json` is committed (it is the reviewable description of the boundary; CI fails if it is
stale). `src/types.ts`, `src/schemas.ts` and the wasm glue are generated and not committed.

When a type that crosses the boundary changes, run `cargo run -q -p linear-wasm --example schema > packages/linear-wasm/schema.json`
and commit it. Bump `linear_core::SCHEMA_VERSION` when the change would make data written
earlier (a cache entry) wrong to read.
