// For Cloudflare Workers (wrangler, @cloudflare/vite-plugin): the bundler turns
// a `.wasm` import into a compiled `WebAssembly.Module`.

import wasm from "./wasm/linear_wasm_bg.wasm";
import { createLinear } from "./index.ts";

export * from "./index.ts";

/** The typed functions, ready to use. */
export const linear = createLinear(wasm);
