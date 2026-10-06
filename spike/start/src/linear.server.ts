// Server-only: fetch happens here (TypeScript); building the request and
// interpreting the response happen in linear-core, compiled to wasm.
import wasmModule from "./wasm/linear_wasm_bg.wasm";
import { build_request, initSync, parse_response } from "./wasm/linear_wasm.js";

initSync({ module: wasmModule });

// The wasm boundary returns JSON text; this is its envelope (data is per operation).
export interface LinearResult {
  ok: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data?: any;
  error?: { code: string; message: string };
}

export async function call(op: string, vars: unknown, apiKey: string): Promise<LinearResult> {
  const built = JSON.parse(build_request(op, JSON.stringify(vars ?? {})));
  if (!built.ok) return built;

  const res = await fetch(built.data.url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: apiKey },
    body: built.data.body,
  });
  const body = await res.text();
  const meta = { status: res.status };
  return JSON.parse(parse_response(op, JSON.stringify(meta), body, Date.now()));
}
