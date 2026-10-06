// Workers bundle `.wasm` imports as a precompiled WebAssembly.Module.
declare module "*.wasm" {
  const mod: WebAssembly.Module;
  export default mod;
}
