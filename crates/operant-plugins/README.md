# operant-plugins

WASM plugin system for Operant: host, manifests, signatures, and the execution bridge.

## Purpose

Plugins are WebAssembly modules loaded through Extism that extend Operant with custom tools and channels (`src/lib.rs`). Modules include `host.rs`, `runtime.rs`, `signature.rs`, `wasm_tool.rs`, `watcher.rs`, and `error.rs`.

## Configuration

- No Cargo features in this crate. The `src/lib.rs` doc says to enable it with `--features plugins-wasm`. That feature is declared in `operant-cli/Cargo.toml`, where it gates this crate.
- `Cargo.toml` includes `toml`; read `src/host.rs` for the manifest format.

## Extension points

- `PluginTool` is a re-export of `operant_api::tool::Tool`. A plugin tool implements the same trait as a built-in tool.
- Wasm-backed tool execution is in `src/wasm_tool.rs`.

## Limitations

- Signatures are checked with `ring` (`Cargo.toml`). The crate docs do not say what is verified beyond "signatures"; read `src/signature.rs`.
- Plugins are only available in builds that enable `plugins-wasm` in `operant-cli`.

## Model experience

A plugin tool reaches the model through the same `Tool` trait as built-in tools (`src/lib.rs`). The crate docs describe nothing else model-facing; read `src/wasm_tool.rs` for the execution path.
