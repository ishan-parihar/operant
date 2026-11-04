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
  (pre-execute before dispatch, post-execute with the result). The module's
  instances are RESIDENT — a `runtime::PluginPool` checked out per event, so
  a call pays no WASM compile (iter-1725 replaced the per-call instantiate;
  `extism::Plugin` is Send + Sync, and extism ships `Pool` for exactly
  this). Instance state persists on checked-out modules. The manifest's
  free-form `[config]` table rides every observe envelope as `"config"`
  (deepseek-harness parity: its policy plugins are all config-driven;
  schema-free here — the host never interprets the table). Fail-open: a
  broken observer, a failed call, or a pool checkout past the 10 s wait
  ceiling allows and logs, never bricks the tool loop.
- **channel** — messaging platform integrations. Deliberately deferred:
  gateway adapters are a compile-time registry, and no dynamic channel
  consumer exists yet.

`requires` (deepseek-harness `inject` parity, collapsed): names that must
be present at discovery — either ANOTHER PLUGIN's name or a service name
another plugin `provides` (iter-1726; dsh composes on service names:
`ctx.provide('memory.backend')` then `inject: ['memory.backend']` — operant
collapses the seam to a name match, no fibers). A plugin whose requires
are unmet is skipped at discovery, transitively (A requires missing B
drops A; C requiring A drops too); mutual requires are satisfied, not a
deadlock. `provides` declares the service names a plugin offers; the host
never resolves them beyond this match — they are composition labels, not
runtime objects.
Shortcut: there is no fiber-reload when a provider returns (dsh's
PENDING state). The hot-swap watcher now feeds BOTH seams (gap G6,
iter-1733): the kernel harness seam (`Harness::replace`/`mount` of
ArchitectureRows) AND the agent surfaces — `apply_plugin_changes`
(plugin_tools.rs) re-registers the plugin's tool (replacing its own
registration; a name the plugin does not already own is still refused,
KB-11) and its observer (named interceptor, replace semantics — a
changed module never stacks a second copy) on the LIVE registry, so a
plugin installed while the gateway runs becomes callable/active without
a restart. Skill roots are the remaining restart-bound surface
(SkillManager is a boot-scanned owned struct; live roots need it
shareable — the named upgrade).

Plugin tools and observer calls run on RESIDENT module instances
(`runtime::PluginPool`, iter-1725): one WASM compile per module per
process, instances checked out up to `available_parallelism`, reused
across calls — the deepseek-harness residency semantics without fibers.

The host (`crates/operant-plugins/src/host.rs`) owns the lifecycle:
discovery, signature checks, and a watcher for live reload. Skill
content is gated by the same signature policy as tool code — a
strict-mode deployment does not load an unsigned plugin's skills either.

## Install + manage

From git repositories or paths (`plugins_install.rs`);
`operant plugins list/install/remove/enable/disable` is the operator
surface. `[plugins] plugin_dirs / signature_mode /
trusted_publisher_keys / disabled` in the config is the policy surface.

## Creating a plugin (creator mode, gap G7 — iter-1730)

`operant plugins new <name> --capability <tool|observer|memory|skill>`
scaffolds a project directory in the CWD: `manifest.toml` (with the chosen
capability), the capability's export-contract `README.md`, and for WASM
capabilities a `Cargo.toml` + `src/lib.rs` PDK stub (extism-pdk). Skill
scaffolds write `skills/<name>/SKILL.md` and carry no WASM. The export
contracts the scaffold names: tools export `tool_metadata` + `execute`;
observers export `observe` (the interception envelope: point, tool, args,
result, config — verdict `allow`/`block`, host fails open); memory exports
`prefetch`/`sync_turn`/`tool_schemas` with graceful degradation. Build with
`cargo build --release --target wasm32-wasip1`, copy the module next to the
manifest, `operant plugins validate <dir>` (the installer's own manifest
validator — one authority), then `operant plugins install file://<abs-path>`.
A scaffold's directory passes the validator as-written; the loop after that
is edit → validate → build → install.

An in-tree fixture module pins the live contracts against REAL WASM
(gap G8 — iter-1732): `crates/operant-plugins/fixtures/fixture.wasm`
(source: `fixtures/fixture-plugin/`, rebuild with
`scripts/build-plugin-fixture.sh`, needs `rustup target add
wasm32-wasip1`) exports `tool_metadata` + `execute` (echo) + `observe`
(allow by default; blocks on tool name or on the envelope's
`config.block_all`), and `crates/operant-plugins/tests/fixture_plugin.rs`
exercises all of it — execute roundtrip, metadata read, allow/block
verdicts, config passthrough — without serde-only pins.

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
