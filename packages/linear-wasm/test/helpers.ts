import { readFileSync } from "node:fs";
import { createLinear } from "../src/index.ts";

const here = (path: string) => new URL(path, import.meta.url);

/** The wasm module, built by `npm run build:wasm` into src/wasm. */
export const wasmBytes = readFileSync(here("../src/wasm/linear_wasm_bg.wasm"));

export const linear = createLinear(wasmBytes);

/** A response captured from a real Linear, with names and ids anonymized (crates/core/tests/fixtures). */
export function fixture(name: string): string {
  return readFileSync(here(`../../../crates/core/tests/fixtures/${name}.json`), "utf8");
}

/** 2026-10-20T12:00:00Z: the fixed "now" of the audit tests. */
export const NOW = Date.UTC(2026, 9, 20, 12, 0, 0);
