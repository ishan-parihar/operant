# Plugins — WASM extensions

Owns: the WebAssembly plugin system — manifests, capabilities, the host,
lifecycle gating, install, and the tool/memory/skill/observer bridges.
Not for: the harness kernel ([harness.md](harness.md)), MCP
([mcp.md](mcp.md)), the tool registry's interception seam
([tools.md](tools.md)).

Source: `crates/operant-plugins/src/` (lib, host, runtime, signature,
wasm_tool, watcher, error),
`crates/operant-cli/src/plugin_tools.rs` (tool + observer bridges),
`crates/operant-cli/src/plugin_memory.rs` (memory bridge),
`crates/operant-cli/src/plugins_install.rs`,
`crates/operant-cli/src/cmd_plugins.rs`.

## The model

Plugins are WebAssembly modules loaded via Extism (on by default since
iter-1720; the `plugins-wasm` cargo feature exists so CI can build
without it). A plugin declares a manifest (`manifest.toml` alongside the
`.wasm` — the ONE manifest authority; the git installer validates it
with the host's own parser) with capabilities and, since iter-1721,
`requires`:

- **tool** — the module exports a tool surface (`tool_metadata`,
  `execute`); `plugin_tools.rs` adapts the `WasmTool` to the core
  `OperantTool` trait and registers it so the agent calls it exactly
  like a built-in (name collisions: the built-in wins, KB-11).
- **memory** — the module can back the core `MemoryProvider` trait
  (hermes's `plugins/memory/<name>` parity); `plugin_memory.rs` is the
  bridge. Selected by `memory.provider = "plugin:<name>"`.
- **skill** — content, not a provider: a plugin ships a `skills/` bundle
  (each subdirectory a SKILL.md with `name`/`description` frontmatter).
  Each plugin's `skills/` dir rides `SkillManager::add_extra_root` as an
  integration root — operator pool wins name collisions, Holon runs skip
  it (org bodies carry no personal integrations). Gateway
  `/reload-skills` re-adds the roots so a rescan does not drop plugin
  skills.
- **observer** — the module exports `observe`; it is registered as a
  tool-seam interceptor and may veto any tool call
  (pre-execute before dispatch, post-execute with the result). Fail-open:
  a broken observer allows and logs, never bricks the tool loop.
- **channel** — messaging platform integrations. Deliberately deferred:
  gateway adapters are a compile-time registry, and no dynamic channel
  consumer exists yet.

`requires` (deepseek-harness `inject` parity, collapsed): names of OTHER
plugins that must be present. A plugin whose requires are unmet is
skipped at discovery, transitively (A requires missing B drops A; C
requiring A drops too); mutual requires are satisfied, not a deadlock.
Shortcut: there is no fiber-reload when a provider returns (dsh's
PENDING state) — a restart or watcher re-scan covers it.

The host (`crates/operant-plugins/src/host.rs`) owns the lifecycle:
discovery, signature checks, and a watcher for live reload. Skill
content is gated by the same signature policy as tool code — a
strict-mode deployment does not load an unsigned plugin's skills either.

## Install + manage

From git repositories or paths (`plugins_install.rs`);
`operant plugins list/install/remove/enable/disable` is the operator
surface. `[plugins] plugin_dirs / signature_mode /
trusted_publisher_keys / disabled` in the config is the policy surface.

## Intention

Plugins are the answer to "extend without recompiling": the harness
kernel composes *Rust* providers at runtime; plugins let *third parties*
ship capabilities as sealed modules — signature-checked, sandboxed by
the WASM boundary, hot-loadable through the watcher. The bridges (tools,
memory, skills, observers) reuse the core traits and seams rather than
inventing a plugin API, so a plugin is interchangeable with a native
provider at the same seam. The capability map and the interception
semantics are a port of deepseek-harness's "everything is a plugin"
architecture (its `inject` lifecycle and `tools/pre-execute` /
`tools/post-execute` waterfalls); the port deliberately did NOT carry
Cordis's service-store/fiber state machine, its five dispatch modes, or
its declarative patch language — operant's capability enum, config
toggles, and `[plugins].disabled` cover those roles with less code.
