// How a panic inside the wasm looks from JavaScript. Needs a build with the
// `panic-probe` feature, which exports a function that panics:
//
//   FEATURES=panic-probe npm run build:wasm && node --test test/panic.test.ts
//
// Without it the tests are skipped (the normal build has no such function).

import assert from "node:assert/strict";
import { test } from "node:test";
import * as raw from "../src/wasm/linear_wasm.js";
import { linear } from "./helpers.ts";

const probe = raw as unknown as { probe_panic?: () => void; probe_fine?: () => number };
const skip = probe.probe_panic === undefined ? "build with FEATURES=panic-probe" : false;

test("a panic is a RuntimeError, and last_panic has its message and place", { skip }, () => {
  assert.equal(raw.last_panic(), undefined, "nothing has panicked yet");
  assert.throws(
    () => probe.probe_panic?.(),
    (e: unknown) => e instanceof WebAssembly.RuntimeError && e.message === "unreachable",
  );
  const message = raw.last_panic();
  assert.match(message ?? "", /panicked at crates\/wasm\/src\/lib\.rs:\d+:\d+/);
  assert.match(message ?? "", /index out of bounds/);
  assert.equal(raw.last_panic(), undefined, "the message is taken once");
});

test("the instance still answers after a panic", { skip }, () => {
  assert.throws(() => probe.probe_panic?.());
  assert.equal(probe.probe_fine?.(), 42);
  assert.equal(linear.buildRequest("whoami").url, "https://api.linear.app/graphql");
  assert.equal(linear.schemaVersion, raw.schema_version());
});
