// The golden files in crates/wasm/tests/golden: one input and the answer the
// native core gives (what the CLI runs). Here the same inputs go through the
// wasm module, and the answers must be identical. The generated zod schemas
// must accept every input that the core accepts and every answer, and reject
// the inputs the core refuses.
//
// To change a golden: `UPDATE_GOLDEN=1 cargo test -p linear-wasm --test golden`.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import * as raw from "../src/wasm/linear_wasm.js";
import {
  auditConfigSchema,
  auditOptionsSchema,
  auditReportSchema,
  builtRequestSchema,
  errorBodySchema,
  findingSchema,
  idParamsSchema,
  noParamsSchema,
  operationSchemas,
  refreshDecisionSchema,
  refreshEventSchema,
  refreshMetaSchema,
  snapshotSchema,
  verificationSchema,
} from "../src/schemas.ts";
import { fixture } from "./helpers.ts";
import "./helpers.ts"; // instantiates the module
import type { z } from "zod";

const GOLDEN = new URL("../../../crates/wasm/tests/golden/", import.meta.url);
const KINDS = ["audit", "diff", "refresh", "webhook", "parse", "build"] as const;
type Kind = (typeof KINDS)[number];

interface Case {
  description: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  input: any;
  expected: { ok: true; data: unknown } | { ok: false; error: unknown };
}

const ms = (iso: string) => Date.parse(iso);

/** JSON input as the boundary takes it: compact text, or empty for null. */
const text = (v: unknown) => (v === null || v === undefined ? "" : JSON.stringify(v));

/** What the wasm answers for a case's input, as parsed JSON (the same inputs as the native test). */
function answer(kind: Kind, input: Case["input"]): unknown {
  switch (kind) {
    case "audit":
      return JSON.parse(
        raw.audit(
          JSON.stringify(input.snapshot),
          text(input.config),
          ms(input.now),
          input.options === null ? undefined : JSON.stringify(input.options),
        ),
      );
    case "diff":
      return JSON.parse(raw.diff(text(input.previous), JSON.stringify(input.current)));
    case "refresh":
      return JSON.parse(
        raw.decide_refresh(text(input.meta), JSON.stringify(input.event), ms(input.now)),
      );
    case "webhook":
      return JSON.parse(raw.verify_webhook(input.body, input.signature, input.secret, ms(input.now)));
    case "parse": {
      const body = input.fixture === null ? input.body : fixture(input.fixture);
      const meta: Record<string, unknown> = { status: input.status };
      for (const key of ["retryAfterSecs", "rateLimitResetMs"]) {
        if (input[key] !== null && input[key] !== undefined) meta[key] = input[key];
      }
      return JSON.parse(raw.parse_response(input.operation, JSON.stringify(meta), body, ms(input.now)));
    }
    case "build":
      return JSON.parse(raw.build_request(input.operation, text(input.params)));
  }
}

/** What a golden holds of an answer: a built request keeps the first line of its query only. */
function normalized(kind: Kind, answer: any): unknown {
  if (kind === "build" && answer.ok === true) {
    const body = JSON.parse(answer.data.body);
    body.query = body.query.split("\n")[0];
    return { ...answer, data: { ...answer.data, body } };
  }
  return answer;
}

function cases(kind: Kind): [string, Case][] {
  const dir = new URL(`${kind}/`, GOLDEN);
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => [f.replace(/\.json$/, ""), JSON.parse(readFileSync(new URL(f, dir), "utf8")) as Case]);
}

const accepts = (schema: z.ZodType, value: unknown, what: string) => {
  const r = schema.safeParse(value);
  assert.ok(r.success, `${what} is rejected by its zod schema: ${JSON.stringify(r.error?.issues, null, 1)}`);
};

const rejects = (schema: z.ZodType, value: unknown, what: string) => {
  assert.equal(schema.safeParse(value).success, false, `${what} should be rejected by its zod schema`);
};

/**
 * The zod checks of a case: the inputs and the answer, against the schemas
 * generated from the same Rust types. Inputs the core refuses are listed with
 * the schema that must refuse them too.
 */
function checkWithZod(kind: Kind, name: string, c: Case) {
  const { input, expected } = c;
  const refused: Record<string, [z.ZodType, unknown]> = {
    "audit/error-snapshot-is-not-a-snapshot": [snapshotSchema, input.snapshot],
    "audit/error-unknown-config-key": [auditConfigSchema.partial(), input.config],
    "refresh/error-unknown-event": [refreshEventSchema, input.event],
    "refresh/error-meta-has-an-unknown-key": [refreshMetaSchema, input.meta],
    "diff/error-current-is-not-a-report": [auditReportSchema, input.current],
    "build/error-missing-id": [idParamsSchema, input.params],
    "build/error-unknown-parameter": [noParamsSchema, input.params],
  };
  const refusal = refused[`${kind}/${name}`];
  if (refusal) rejects(refusal[0], refusal[1], `${kind}/${name} input`);

  if (!expected.ok) {
    accepts(errorBodySchema, expected.error, `${kind}/${name} error`);
    return;
  }
  const data = expected.data;
  switch (kind) {
    case "audit":
      accepts(snapshotSchema, input.snapshot, "snapshot");
      if (input.config !== null) accepts(auditConfigSchema.partial(), input.config, "config");
      if (input.options !== null) accepts(auditOptionsSchema.partial(), input.options, "options");
      accepts(auditReportSchema, data, "report");
      break;
    case "diff":
      if (input.previous !== null) accepts(auditReportSchema, input.previous, "previous report");
      accepts(auditReportSchema, input.current, "current report");
      for (const f of data as unknown[]) accepts(findingSchema, f, "finding");
      break;
    case "refresh":
      if (input.meta !== null) accepts(refreshMetaSchema, input.meta, "meta");
      accepts(refreshEventSchema, input.event, "event");
      accepts(refreshDecisionSchema, data, "decision");
      break;
    case "webhook":
      accepts(verificationSchema, data, "verification");
      break;
    case "parse":
      accepts(operationSchemas[input.operation as keyof typeof operationSchemas].data, data, "result");
      break;
    case "build":
      break; // the request body is checked on the raw answer, below
  }
}

for (const kind of KINDS) {
  const all = cases(kind);

  test(`golden/${kind} has cases`, () => {
    assert.ok(all.length >= 5, `${all.length} cases`);
  });

  for (const [name, c] of all) {
    test(`golden ${kind}/${name}: ${c.description}`, () => {
      const got = answer(kind, c.input);
      assert.deepStrictEqual(normalized(kind, got), c.expected);
      checkWithZod(kind, name, c);
      if (kind === "build" && (got as { ok: boolean }).ok) {
        accepts(builtRequestSchema, (got as { data: unknown }).data, "built request");
      }
    });
  }
}
