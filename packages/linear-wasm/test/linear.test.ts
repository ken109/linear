// The typed wrapper over the real wasm module, with the generated zod schemas
// checking what comes out. Run with `npm test` (which builds the wasm first).

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  LinearError,
  LinearPanic,
  linearFromExports,
  type AuditReport,
  type OperationName,
  type Snapshot,
  type WasmExports,
  SCHEMA_VERSION,
} from "../src/index.ts";
import {
  auditReportSchema,
  operationSchemas,
  refreshDecisionSchema,
  verificationSchema,
} from "../src/schemas.ts";
import { NOW, fixture, linear } from "./helpers.ts";

/** Each operation with the fixture that Linear's response to it was captured in. */
const FIXTURES: [OperationName, string][] = [
  ["whoami", "whoami"],
  ["issue", "issue"],
  ["assigned_started_issues", "assigned_issues"],
  ["projects", "projects"],
  ["issue_comments", "issue_comments"],
  ["templates", "templates"],
  ["initiatives", "initiatives"],
  ["issue_view", "issue_view"],
  ["project_view", "project_view"],
  ["milestones_of_project", "milestones"],
  ["milestone_view", "milestone_view"],
  ["initiative_view", "initiative_view"],
  ["labels", "labels"],
  ["teams", "teams"],
  ["users", "users"],
];

const ok = (body: string) => ({ status: 200, body });

test("the wasm module and the generated types are the same version", () => {
  assert.equal(linear.schemaVersion, SCHEMA_VERSION);
});

test("every operation has a fixture, so the table above is complete", () => {
  assert.deepEqual(FIXTURES.map(([op]) => op).sort(), Object.keys(operationSchemas).sort());
});

// ---------------------------------------------------------------- buildRequest

test("buildRequest says where and what to POST", () => {
  const whoami = linear.buildRequest("whoami");
  assert.equal(whoami.url, "https://api.linear.app/graphql");
  assert.match(JSON.parse(whoami.body).query, /viewer/);

  const issue = linear.buildRequest("issue", { id: "KK-12" });
  assert.deepEqual(JSON.parse(issue.body).variables, { id: "KK-12" });

  const page = linear.buildRequest("projects", { first: 3, after: "abc" });
  assert.deepEqual(JSON.parse(page.body).variables, { first: 3, after: "abc" });
  // The first page needs no parameters; the size defaults per listing.
  assert.equal(JSON.parse(linear.buildRequest("projects").body).variables.first, 10);
});

test("buildRequest refuses what it cannot build, and the types say so first", () => {
  // @ts-expect-error `issue` needs an id
  assert.throws(() => linear.buildRequest("issue"), (e: unknown) => {
    assert.ok(e instanceof LinearError);
    assert.equal(e.code, "usage");
    assert.match(e.message, /missing field `id`/);
    return true;
  });
  // @ts-expect-error not an operation
  assert.throws(() => linear.buildRequest("nope"), /unknown operation: nope/);
  // @ts-expect-error `whoami` takes no parameters
  assert.throws(() => linear.buildRequest("whoami", { id: "x" }), /unknown field/);
});

// --------------------------------------------------------------- parseResponse

for (const [operation, name] of FIXTURES) {
  test(`parseResponse(${operation}) turns a real response into data the zod schema accepts`, () => {
    const data = linear.parseResponse(operation, ok(fixture(name)), NOW);
    const checked = operationSchemas[operation].data.safeParse(data);
    assert.ok(checked.success, JSON.stringify(checked.error?.issues, null, 2));
  });
}

test("parseResponse is typed per operation", () => {
  const whoami = linear.parseResponse("whoami", ok(fixture("whoami")), NOW);
  assert.equal(whoami.viewer.isMe, true);
  assert.equal(whoami.organization.urlKey, "example");

  const issue = linear.parseResponse("issue", ok(fixture("issue")), NOW).issue;
  assert.equal(issue.identifier, "EX-23");
  assert.equal(issue.state.type, "started");
  assert.equal(issue.team.key, "EX");
  assert.equal(typeof issue.createdAt, "string");
});

test("an open enum keeps a value Linear adds later", () => {
  const body = fixture("projects").replace('"type": "started"', '"type": "somethingNew"');
  assert.notEqual(body, fixture("projects"), "the fixture no longer has a started project status");
  const data = linear.parseResponse("projects", ok(body), NOW);
  const types = data.projects.nodes.map((p) => p.status.type);
  assert.ok(types.includes("somethingNew"), types.join());
  // And the schema, which lists the known values, does not reject it.
  assert.ok(operationSchemas.projects.data.safeParse(data).success);
});

test("the zod schemas reject data that is not of the shape", () => {
  const data = linear.parseResponse("issue", ok(fixture("issue")), NOW);
  const { id: _removed, ...withoutId } = data.issue;
  assert.equal(operationSchemas.issue.data.safeParse({ issue: withoutId }).success, false);
  assert.equal(
    operationSchemas.issue.data.safeParse({ issue: { ...data.issue, createdAt: "yesterday" } })
      .success,
    false,
  );
  assert.equal(
    operationSchemas.issue.data.safeParse({ issue: { ...data.issue, team: null } }).success,
    false,
  );
});

test("a rejected key is an auth error", () => {
  assert.throws(
    () =>
      linear.parseResponse("whoami", { status: 401, body: fixture("error_unauthenticated") }, NOW),
    (e: unknown) => e instanceof LinearError && e.code === "auth",
  );
});

test("Linear's own error code and a rate limit's wait are on the error", () => {
  assert.throws(
    () => linear.parseResponse("projects", { status: 400, body: fixture("error_too_complex") }, NOW),
    (e: unknown) => e instanceof LinearError && e.apiCode === "INPUT_ERROR",
  );

  const headers = new Headers({ "x-ratelimit-requests-reset": String(NOW + 30_000) });
  assert.throws(
    () => linear.parseResponse("whoami", { status: 429, headers, body: "{}" }, NOW),
    (e: unknown) => e instanceof LinearError && e.retryAfterSecs === 30,
  );
  assert.throws(
    () =>
      linear.parseResponse(
        "whoami",
        { status: 429, headers: new Headers({ "retry-after": "7" }), body: "{}" },
        NOW,
      ),
    (e: unknown) => e instanceof LinearError && e.retryAfterSecs === 7,
  );
});

// ----------------------------------------------------------------------- send

test("send builds the request, fetches it and parses the response", async () => {
  const seen: { url: string; init: RequestInit }[] = [];
  const fetchStub = (async (url: string | URL | Request, init?: RequestInit) => {
    seen.push({ url: String(url), init: init ?? {} });
    return new Response(fixture("whoami"), { status: 200 });
  }) as typeof fetch;

  const data = await linear.send("whoami", undefined, {
    authorization: "lin_api_test",
    fetch: fetchStub,
  });
  assert.equal(data.organization.urlKey, "example");
  assert.equal(seen.length, 1);
  assert.equal(seen[0]?.url, "https://api.linear.app/graphql");
  assert.equal(seen[0]?.init.method, "POST");
  assert.deepEqual(seen[0]?.init.headers, {
    "content-type": "application/json",
    authorization: "lin_api_test",
  });
  assert.match(JSON.parse(String(seen[0]?.init.body)).query, /viewer/);
});

test("send reports Linear's errors the same way", async () => {
  const fetchStub = (async () =>
    new Response(fixture("error_unauthenticated"), { status: 401 })) as typeof fetch;
  await assert.rejects(
    linear.send("whoami", undefined, { authorization: "bad", fetch: fetchStub }),
    (e: unknown) => e instanceof LinearError && e.code === "auth",
  );
});

// ---------------------------------------------------------------------- audit

function snapshot(): Snapshot {
  const issues = linear.parseResponse("assigned_started_issues", ok(fixture("assigned_issues")), NOW);
  const projects = linear.parseResponse("projects", ok(fixture("projects")), NOW);
  return {
    workspace: "example",
    issues: issues.viewer.assignedIssues.nodes,
    projects: projects.projects.nodes,
    templates: [],
  };
}

test("audit runs the rules over a snapshot built from parsed responses", () => {
  const report = linear.audit(snapshot(), { staleDays: 1, statusUpdateDays: 1 }, { now: NOW });
  assert.ok(auditReportSchema.safeParse(report).success);
  assert.ok(report.findings.length > 0);
  for (const f of report.findings) assert.equal(f.workspace, "example");
  assert.deepEqual(report.unresolvedIssues, []);
});

test("audit uses the defaults for a config left out, and Date or milliseconds for now", () => {
  const byDefault = linear.audit(snapshot(), undefined, { now: NOW });
  const byDate = linear.audit(snapshot(), {}, { now: new Date(NOW) });
  assert.deepEqual(byDefault, byDate);
});

test("audit can be narrowed to the issues a caller names", () => {
  const report = linear.audit(
    snapshot(),
    {},
    { now: NOW, scope: { issues: ["EX-999"], since: new Date(NOW - 86_400_000) } },
  );
  assert.deepEqual(report.unresolvedIssues, ["EX-999"]);
});

test("audit reports bad input as a usage error", () => {
  assert.throws(
    () => linear.audit({ workspace: "x" } as unknown as Snapshot, {}, { now: NOW }),
    (e: unknown) => e instanceof LinearError && e.code === "usage" && /invalid snapshot/.test(e.message),
  );
  assert.throws(
    () => linear.audit(snapshot(), {}, { now: NOW, scope: { since: new Date(NOW) } }),
    /--since needs --issues/,
  );
});

test("diff reports only what the previous report did not have", () => {
  const strict = linear.audit(snapshot(), { staleDays: 0, statusUpdateDays: 0 }, { now: NOW });
  const all = linear.diff(null, strict);
  assert.deepEqual(all, strict.findings);
  assert.deepEqual(linear.diff(strict, strict), []);
  const earlier: AuditReport = { ...strict, findings: strict.findings.slice(0, -1) };
  assert.deepEqual(linear.diff(earlier, strict), strict.findings.slice(-1));
});

// -------------------------------------------------------------- decideRefresh

test("decideRefresh answers from a cache entry's times, an event and now", () => {
  const meta = { schemaVersion: SCHEMA_VERSION, fetchedAt: new Date(NOW - 60_000).toISOString() };
  const fresh = linear.decideRefresh(meta, { kind: "read" }, NOW);
  assert.deepEqual(fresh, {
    refresh: false,
    reason: "fresh",
    freshness: { state: "fresh", ageSecs: 60 },
  });
  assert.ok(refreshDecisionSchema.safeParse(fresh).success);

  assert.equal(linear.decideRefresh(null, { kind: "read" }, NOW).reason, "never-fetched");
  const hook = linear.decideRefresh(meta, { kind: "webhook", resourceType: "Issue", action: "update" }, NOW);
  assert.equal(hook.refresh, true);
  assert.ok(refreshDecisionSchema.safeParse(hook).success);
  assert.equal(
    linear.decideRefresh({ ...meta, ttlSecs: 30 }, { kind: "read" }, NOW).reason,
    "ttl-expired",
  );
});

// ---------------------------------------------------------------- verifyWebhook

const BODY =
  '{"action":"update","type":"Issue","organizationId":"org-1","webhookTimestamp":1760000000000}';
const SECRET = "lin_wh_testsecret";
// printf %s "$BODY" | openssl dgst -sha256 -hmac "$SECRET"
const SIGNATURE = "19882a77e4dae983f01a21fbe0a53dd8142d0d699c37633bc9258a6237b13830";

test("verifyWebhook accepts a known signature and says what the delivery is", () => {
  const v = linear.verifyWebhook(BODY, SIGNATURE, SECRET, 1_760_000_030_000);
  assert.ok(verificationSchema.safeParse(v).success);
  assert.equal(v.status, "valid");
  if (v.status === "valid") {
    assert.equal(v.event.type, "Issue");
    assert.equal(v.event.organizationId, "org-1");
  }
});

test("verifyWebhook rejects without throwing, and says why", () => {
  assert.deepEqual(linear.verifyWebhook(BODY, SIGNATURE, SECRET, 1_760_000_061_000), {
    status: "invalid",
    reason: "stale-timestamp",
  });
  assert.deepEqual(linear.verifyWebhook(BODY, SIGNATURE, "other", 1_760_000_000_000), {
    status: "invalid",
    reason: "signature-mismatch",
  });
  assert.deepEqual(linear.verifyWebhook(BODY, "nope", SECRET, 1_760_000_000_000), {
    status: "invalid",
    reason: "malformed-signature",
  });
  assert.throws(
    () => linear.verifyWebhook(BODY, SIGNATURE, "", 1_760_000_000_000),
    (e: unknown) => e instanceof LinearError && e.code === "usage",
  );
});

// ----------------------------------------------------------- failures of wasm

function exportsThat(overrides: Partial<WasmExports>): WasmExports {
  return {
    audit: () => "",
    build_request: () => "",
    decide_refresh: () => "",
    diff: () => "",
    last_panic: () => undefined,
    parse_response: () => "",
    schema_version: () => SCHEMA_VERSION,
    verify_webhook: () => "",
    ...overrides,
  };
}

test("a trap becomes a LinearPanic that carries the panic message", () => {
  const trap = new WebAssembly.RuntimeError("unreachable");
  const broken = linearFromExports(
    exportsThat({
      build_request: () => {
        throw trap;
      },
      last_panic: () => "panicked at src/x.rs:1:1:\nindex out of bounds",
    }),
  );
  assert.throws(
    () => broken.buildRequest("whoami"),
    (e: unknown) => {
      assert.ok(e instanceof LinearPanic);
      assert.match(e.message, /index out of bounds/);
      assert.equal(e.cause, trap);
      return true;
    },
  );
});

test("a trap without a recorded message still says what happened", () => {
  const broken = linearFromExports(
    exportsThat({
      diff: () => {
        throw new WebAssembly.RuntimeError("unreachable");
      },
    }),
  );
  assert.throws(() => broken.diff(null, { findings: [], unresolvedIssues: [] }), (e: unknown) => {
    return e instanceof LinearPanic && e.message === "unreachable";
  });
});

test("other exceptions pass through untouched", () => {
  const boom = new TypeError("not a wasm trap");
  const broken = linearFromExports(
    exportsThat({
      verify_webhook: () => {
        throw boom;
      },
    }),
  );
  assert.throws(() => broken.verifyWebhook("", "", "s"), (e: unknown) => e === boom);
});

test("a module of another schema version is refused at start-up", () => {
  assert.throws(
    () => linearFromExports(exportsThat({ schema_version: () => SCHEMA_VERSION + 1 })),
    /schema version/,
  );
});
