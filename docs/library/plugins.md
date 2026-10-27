# Plugins — WASM extensions

Owns: the WebAssembly plugin system — manifests, capabilities, the host,
install, and the tool/memory bridges.
Not for: the harness kernel ([harness.md](harness.md)), MCP
([mcp.md](mcp.md)).

Source: `crates/operant-plugins/src/` (lib, host, runtime, signature,
wasm_tool, watcher, error),
`crates/operant-cli/src/plugin_tools.rs` (tool bridge),
`crates/operant-cli/src/plugin_memory.rs` (memory bridge),
`crates/operant-cli/src/plugins_install.rs`,
`crates/operant-cli/src/cmd_plugins.rs`.

## The model

Plugins are WebAssembly modules loaded via Extism, enabled with the
`plugins-wasm` feature. A plugin declares a manifest
(`manifest.toml` alongside the `.wasm`) with capabilities:

- **tool** — the module exports a tool surface; `plugin_tools.rs`
  adapts the `WasmTool` to the core `OperantTool` trait and registers it
  so the agent calls it exactly like a built-in.
- **memory** — the module can back the core `MemoryProvider` trait
  (hermes's `plugins/memory/<name>` parity); `plugin_memory.rs` is the
  bridge. The WASM ABI mirrors the trait's hook surface.
- **channel** — messaging platform integrations.

The host (`crates/operant-plugins/src/host.rs`) owns the lifecycle:
discovery, signature checks, and a watcher for live reload.

## Install + manage

From git repositories or paths (`plugins_install.rs`);
`operant plugins list/install/remove/enable/disable` is the operator
surface.

## Intention

Plugins are the answer to "extend without recompiling": the harness
kernel composes *Rust* providers at runtime; plugins let *third parties*
ship capabilities as sealed modules — signature-checked, sandboxed by
the WASM boundary, hot-loadable through the watcher. The two bridges
(tools, memory) reuse the core traits rather than inventing a plugin
API, so a plugin is interchangeable with a native provider at the same
seam.
