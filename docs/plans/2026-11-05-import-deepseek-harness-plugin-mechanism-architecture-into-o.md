# Import deepseek-harness plugin mechanism architecture into operant (ponytail port)

Dated-frozen: published `2026-11-05`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Source of the architecture: `../external-repos/deepseek-harness` (dsh) — vendored
Cordis, "everything is a plugin". Reference exploration report archived in the
session transcript; the essential findings are restated here.

## Mechanism

The import is **wake + complete + gate**, not rebuild. dsh's plugin essence
(plugin object with `inject`-gated lifecycle, capability/service seams, effect
disposers, interception waterfalls) maps almost one-to-one onto operant's
EXISTING dormant machinery (`operant-plugins` Extism WASM host +
`PluginCapability {Tool, Channel, Memory, Observer, Skill}`), which is
feature-gated off with Skill/Observer unmapped. The plan wakes it, completes the
capability map, and ports dsh's two missing semantics:

- **iter A (Phase 1)** — wake the host: `plugins-wasm` into operant-cli default
  features; fold `register_plugin_tools` into one `activate_plugins()` pass
  (Tool → registry [exists], Skill → `SkillManager::add_extra_root`, Memory →
  existing `PluginMemoryProvider`, Observer → Phase 3 placeholder, Channel →
  explicitly deferred); call from the one build funnel in `main.rs`.
- **iter B (Phase 2)** — dsh lifecycle semantics: `PluginManifest.requires` +
  `PluginHost` skip logic (dsh's PENDING-until-available collapsed to a filter —
  named shortcut, watcher re-scan covers live reload); reconcile the two
  manifest formats onto ONE (`manifest.toml`).
- **iter C (Phase 3)** — dsh's hook architecture port: tool-seam interception
  waterfalls (`tools/pre-execute` / `tools/post-execute`) in `ToolRegistry`,
  reusing the vestigial `PipelineAction {Allow, Block(String)}`;
  `PluginCapability::Observer` bridges to this seam. Fire-and-forget
  `HookRegistry` untouched; serial/bail dispatch modes intentionally not ported.
- **iter D (Phase 4)** — `operant.example.toml` `[plugins]`, AGENTS.md
  design-preferences note, `docs/library/` entry, `docs/CHANGELOG.md`.

dsh's Cordis service-store/fiber state machine, 5-dispatch-mode event bus,
declarative patch language, ESM dynamic loading, and plugin marketplace are
**deliberately not ported** — operant's capability enum + config toggles +
Extism sandboxing + binary_updater content installs already cover those roles
with less code. The dsh web UI is NOT imported (it is plugin-booted and speaks
a proprietary Typert RPC protocol; reimplementing `session.*` in Rust is the
opposite of ponytail) — the first-party dashboard stays; a full dsh-UI import
remains an opt-in follow-up.

## Gates / success criteria

(fixed before the run)

- `plugins-wasm` feature on by default; `cargo check -p operant-cli --bin operant`
  green without explicit feature flags.
- A plugin manifest declaring `skill` capability results in a `SkillManager`
  extra root (TUI `/plugins` hub + `operant plugins list` show it); a plugin
  with an unmet `requires` is skipped at discovery (test).
- A pre-execute Block handler registered on `ToolRegistry` stops a builtin tool
  from executing and returns a typed error result (regression test that fails
  without the fix).
- `cargo test -p operant-plugins` green; `bash scripts/clippy-warning-gate.sh`
  green; `cargo fmt --all` clean.
- Each iter: number from `origin/main` after fetch, commit `feat(iter-N)`, push,
  deploy per AGENTS.md §7.

## Cost

- iter A: ~2 files touched beyond Cargo.toml (plugin_tools.rs, main.rs), one new
  function; no new crates, no new deps (extism already in Cargo.lock).
- iter B: manifest field + host filter + installer validation, ~3 files.
- iter C: ToolRegistry seam (~40 lines) + WASM observer adapter + tests.
- iter D: docs only.
