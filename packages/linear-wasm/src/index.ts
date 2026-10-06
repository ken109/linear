// A typed wrapper around the wasm module's six functions.
//
// The wasm side takes and returns JSON text; this layer does the
// JSON.stringify / JSON.parse, turns `{ ok: false }` answers into thrown
// errors, and types every call with the generated types.
//
// The caller does the I/O: `buildRequest` says what to POST, `parseResponse`
// reads what came back (`send` does both around a `fetch`). Nothing here reads
// the clock except to fill in a `now` the caller left out.

import * as raw from "./wasm/linear_wasm.js";
import { SCHEMA_VERSION } from "./types.ts";
import type {
  AuditConfig,
  AuditReport,
  BuiltRequest,
  ErrorBody,
  ErrorCodeName,
  Finding,
  OperationName,
  Operations,
  RefreshDecision,
  RefreshEvent,
  RefreshMeta,
  Snapshot,
  Verification,
} from "./types.ts";

export * from "./types.ts";

/** A point in time: a `Date`, or epoch milliseconds. */
export type Instant = Date | number;

/** A call the wasm side refused or could not answer. */
export class LinearError extends Error {
  readonly code: ErrorCodeName;
  /** For a rate limit: seconds to wait, if Linear said. */
  readonly retryAfterSecs: number | undefined;
  /** For a GraphQL error: Linear's own `extensions.code`. */
  readonly apiCode: string | undefined;

  constructor(body: ErrorBody) {
    super(body.message);
    this.name = "LinearError";
    this.code = body.code;
    this.retryAfterSecs = body.retryAfterSecs ?? undefined;
    this.apiCode = body.apiCode ?? undefined;
  }
}

/**
 * The wasm module panicked. That is a bug in it, not in the call. The message
 * and location of the panic are in `message`; the original `RuntimeError`
 * (`unreachable`) is the `cause`.
 */
export class LinearPanic extends Error {
  constructor(message: string, cause: unknown) {
    super(message, { cause });
    this.name = "LinearPanic";
  }
}

/** What `parseResponse` needs of an HTTP response. A `fetch` `Response` fits once its body is read. */
export interface HttpResponse {
  status: number;
  /** For `Retry-After` and `X-RateLimit-Requests-Reset`. */
  headers?: { get(name: string): string | null };
  body: string;
}

/** `AuditConfig` with the defaults left out. */
export type AuditConfigInput = Partial<AuditConfig>;

/** What an audit is narrowed to; `since` may be a `Date`. */
export interface AuditScope {
  issues?: string[] | null;
  since?: Date | string | null;
}

export interface SendInit {
  /** The `Authorization` header as Linear wants it: a personal API key as is, or `Bearer <token>`. */
  authorization: string;
  /** Defaults to the global `fetch`. */
  fetch?: typeof fetch;
  signal?: AbortSignal;
}

type ParamsArg<K extends OperationName> = {} extends Operations[K]["params"]
  ? [params?: Operations[K]["params"]]
  : [params: Operations[K]["params"]];

export interface Linear {
  /** The schema version of the wasm module (equal to `SCHEMA_VERSION`). */
  readonly schemaVersion: number;
  /** What to POST to Linear for an operation. */
  buildRequest<K extends OperationName>(operation: K, ...params: ParamsArg<K>): BuiltRequest;
  /** The result of an operation, from Linear's response to the request `buildRequest` made. */
  parseResponse<K extends OperationName>(
    operation: K,
    response: HttpResponse,
    now?: Instant,
  ): Operations[K]["data"];
  /** `buildRequest`, a `fetch`, and `parseResponse`. */
  send<K extends OperationName>(
    operation: K,
    params: Operations[K]["params"] | undefined,
    init: SendInit,
  ): Promise<Operations[K]["data"]>;
  /** Run the audit rules over a snapshot. */
  audit(
    snapshot: Snapshot,
    config?: AuditConfigInput,
    options?: { now?: Instant; scope?: AuditScope },
  ): AuditReport;
  /** The findings in `current` that `previous` (if any) did not have. */
  diff(previous: AuditReport | null | undefined, current: AuditReport): Finding[];
  /** Whether a cached snapshot should be refreshed now. */
  decideRefresh(
    meta: RefreshMeta | null | undefined,
    event: RefreshEvent,
    now?: Instant,
  ): RefreshDecision;
  /** Check a webhook delivery: `body` is the exact text received. */
  verifyWebhook(body: string, signature: string, secret: string, now?: Instant): Verification;
}

const millis = (t: Instant | undefined): number =>
  t === undefined ? Date.now() : t instanceof Date ? t.getTime() : t;

const numberOrNull = (v: string | null | undefined): number | null => {
  if (v === null || v === undefined || v.trim() === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

type Wire<T> = { ok: true; data: T } | { ok: false; error: ErrorBody };

function unwrap<T>(text: string): T {
  const wire = JSON.parse(text) as Wire<T>;
  if (!wire.ok) throw new LinearError(wire.error);
  return wire.data;
}

/** The wasm module's functions, as `wasm-bindgen` exports them. */
export type WasmExports = Pick<
  typeof raw,
  | "audit"
  | "build_request"
  | "decide_refresh"
  | "diff"
  | "last_panic"
  | "parse_response"
  | "schema_version"
  | "verify_webhook"
>;

/**
 * Instantiate the wasm module and return the typed functions.
 *
 * `wasm` is the compiled module (`import wasm from "@ken109/linear-wasm/linear.wasm"`
 * in a Worker, which the bundler turns into a `WebAssembly.Module`) or its bytes.
 * Calling this again with the same module is cheap: the module is
 * instantiated once.
 */
export function createLinear(wasm: WebAssembly.Module | BufferSource): Linear {
  raw.initSync({ module: wasm });
  return linearFromExports(raw);
}

/**
 * The typed functions over already-instantiated exports. `createLinear` is this
 * after `initSync`; it is separate so that the error handling can be tested
 * against exports that fail.
 */
export function linearFromExports(wasm: WasmExports): Linear {
  const version = wasm.schema_version();
  if (version !== SCHEMA_VERSION) {
    throw new Error(
      `the wasm module has schema version ${version} but these types are for ${SCHEMA_VERSION}; ` +
        "install matching versions of the package's files",
    );
  }

  /** Run a wasm call; a trap becomes a `LinearPanic` carrying the panic message. */
  const guarded = <T>(call: () => string): T => {
    let text: string;
    try {
      text = call();
    } catch (e) {
      if (e instanceof WebAssembly.RuntimeError) {
        throw new LinearPanic(wasm.last_panic() ?? e.message, e);
      }
      throw e;
    }
    return unwrap<T>(text);
  };

  const build = (operation: OperationName, params: unknown): BuiltRequest =>
    guarded(() => wasm.build_request(operation, JSON.stringify(params ?? {})));

  const linear: Linear = {
    schemaVersion: version,

    buildRequest(operation, ...params) {
      return build(operation, params[0]);
    },

    parseResponse(operation, response, now) {
      const meta = {
        status: response.status,
        retryAfterSecs: numberOrNull(response.headers?.get("retry-after")),
        rateLimitResetMs: numberOrNull(response.headers?.get("x-ratelimit-requests-reset")),
      };
      return guarded(() =>
        wasm.parse_response(operation, JSON.stringify(meta), response.body, millis(now)),
      );
    },

    async send(operation, params, init) {
      const request = build(operation, params);
      const res = await (init.fetch ?? fetch)(request.url, {
        method: "POST",
        headers: { "content-type": "application/json", authorization: init.authorization },
        body: request.body,
        signal: init.signal,
      });
      return linear.parseResponse(operation, {
        status: res.status,
        headers: res.headers,
        body: await res.text(),
      });
    },

    audit(snapshot, config, options) {
      const scope = options?.scope;
      return guarded(() =>
        wasm.audit(
          JSON.stringify(snapshot),
          JSON.stringify(config ?? {}),
          millis(options?.now),
          scope === undefined ? undefined : JSON.stringify(scope),
        ),
      );
    },

    diff(previous, current) {
      return guarded(() =>
        wasm.diff(previous ? JSON.stringify(previous) : "", JSON.stringify(current)),
      );
    },

    decideRefresh(meta, event, now) {
      return guarded(() =>
        wasm.decide_refresh(meta ? JSON.stringify(meta) : "", JSON.stringify(event), millis(now)),
      );
    },

    verifyWebhook(body, signature, secret, now) {
      return guarded(() => wasm.verify_webhook(body, signature, secret, millis(now)));
    },
  };
  return linear;
}
