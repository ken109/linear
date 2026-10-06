declare module "cloudflare:workers" {
  export const env: { LINEAR_API_KEY: string };
}

// Workers bundle `.wasm` imports as a precompiled WebAssembly.Module.
declare module "*.wasm" {
  const mod: WebAssembly.Module;
  export default mod;
}
