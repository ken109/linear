# linear

A fast, scriptable command-line client for [Linear](https://linear.app), written in Rust.

> Status: early development. Nothing here is stable yet.

## Goals

- One static binary; no runtime to install.
- Multiple workspaces, switched through a config file (`~/.config/linear/workspaces.toml`).
- Safe writes: ownership rules and declarative validators are enforced by the CLI itself.
- Machine-friendly: `--json` everywhere, stable exit codes.
- The logic lives in an I/O-free core crate, so the same code can run natively (CLI) and in WebAssembly.

## Layout

| Path            | Package       | Role                                                        |
| --------------- | ------------- | ----------------------------------------------------------- |
| `crates/core`   | `linear-core` | Pure logic: types, request building, response parsing.      |
| `crates/cli`    | `linear`      | The `linear` binary: HTTP, config, credentials, output.     |
| `crates/wasm`   | `linear-wasm` | JSON-in/JSON-out wrapper over the core (skeleton).          |
| `schema/`       | -             | Vendored Linear GraphQL SDL used to type-check queries.     |

## Build

```sh
cargo build --workspace
cargo test --workspace
```

## License

MIT
