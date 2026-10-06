// Spike: fetch happens here (TypeScript); building the request and interpreting
// the response happen in linear-core, compiled to wasm.
import wasmModule from "./wasm/linear_wasm_bg.wasm";
import { build_request, initSync, parse_response } from "./wasm/linear_wasm.js";

interface Env {
  LINEAR_API_KEY: string;
  CACHE: KVNamespace;
}

initSync({ module: wasmModule });

type Wire =
  | { ok: true; request: { url: string; body: string } }
  | { ok: false; error: { code: string; message: string } };

async function call(op: string, vars: unknown, apiKey: string): Promise<unknown> {
  const built = JSON.parse(build_request(op, JSON.stringify(vars ?? {}))) as Wire;
  if (!built.ok) return built;

  const res = await fetch(built.request.url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: apiKey },
    body: built.request.body,
  });
  const body = await res.text();
  const meta = {
    status: res.status,
    retryAfterSecs: numberOrNull(res.headers.get("retry-after")),
    rateLimitResetMs: numberOrNull(res.headers.get("x-ratelimit-requests-reset")),
  };
  return JSON.parse(parse_response(op, JSON.stringify(meta), body, Date.now()));
}

function numberOrNull(v: string | null): number | null {
  if (v === null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    // `?bad=1` sends an invalid key, to exercise the error path of parse_response.
    const key = url.searchParams.has("bad") ? "lin_api_invalid" : env.LINEAR_API_KEY;

    let result: unknown;
    if (url.pathname === "/whoami") {
      result = await call("whoami", {}, key);
    } else if (url.pathname.startsWith("/issue/")) {
      result = await call("issue", { id: url.pathname.slice("/issue/".length) }, key);
    } else if (url.pathname === "/projects") {
      result = await call("projects", { first: 5 }, key);
    } else if (url.pathname === "/store") {
      // Parse via wasm, put the result in KV, read it back.
      const parsed = await call("projects", { first: 5 }, key);
      await env.CACHE.put("projects", JSON.stringify(parsed));
      const back = await env.CACHE.get("projects");
      result = { stored: back !== null, bytes: back?.length ?? 0 };
    } else {
      return new Response("try /whoami, /issue/<id>, /projects (add ?bad=1 for a bad key)\n", {
        status: 404,
      });
    }
    return Response.json(result);
  },
};
