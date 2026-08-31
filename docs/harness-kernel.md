# Operant Harness Kernel (plan 016)

The operant harness is the **composable provider runtime** that lets the
agent load, swap, and unload its own capabilities at runtime without a
restart. It is **dark-mergeable**: the default config keeps it disabled,
and the legacy boot path is byte-stable when it is off.

This is the operational companion to `plans/016-harness-kernel-paradigm.md`.
The plan is the design contract; this doc is the operator view.

## Five adopted semantics

| # | Semantic | Operant form |
|---|---|---|
| 1 | String-keyed claims per seam | `tool`, `hook.*`, `memory.provider`, `gateway.command`, `channel.adapter`, `prompt.<key>`, `pool.*` |
| 2 | Declared requirements + PENDING late binding | `ProviderSpec::requires()`; unmet ⇒ PENDING, rescanned on every successful activation |
| 3 | Reversible effects (LIFO) | Every `Seam::install` returns an `Effect` undo handle; unmount unwinds LIFO with per-effect containment |
| 4 | Transactional swap (restore-on-failure) | `Harness::replace` stages activation → atomic slot swap → generation bump (ABA-safe) → unwind old |
| 5 | Composition as data | `architecture.toml` + `architecture.patch.toml`; `operant architecture dump` prints the resolved tree |

## Replaces-not-duplicates

The harness **wraps** every capability surface that already exists in
operant. Existing call sites and tooling are byte-stable when
`[harness].enabled = false`:

| Existing machinery | Wrapped by seam | Status |
|---|---|---|
| `ToolRegistry::register/unregister` | `tool` seam | additive (`register_dyn` + `unregister_tool`) |
| `HookRunner` static dispatch | `hook.*` seam via `DynamicHooks` slot | registered once at boot; children added/removed dynamically |
| `MemoryProvider` selection | `memory.provider` seam | replaces prior selection, restores on undo |
| `PluginCommand` global registry | `gateway.command` seam | dynamic command map, separate from the global (which is unregister-incapable) |
| `Gateway::register_adapter/remove_adapter` | `channel.adapter` seam via `ChannelAdapterHost` trait | additive trait impl on `Gateway` |
| Inline prompt assembly | `prompt.section` seam via `PromptSections` slot | additive `extend_from_slot` on `SystemPromptBuilder` |
| `hermes _pool.yaml` | `pool` seam (compiled) | additive — new source on `ArchitectureRow` |

## Enable the harness

Set `[harness].enabled = true` in `operant.config.toml`. The kernel
boots with the configured `[harness].architecture_path`
(default `architecture.toml`).

```toml
[harness]
enabled = true
architecture_path = "architecture.toml"
patch_paths = ["architecture.patch.toml"]
mount_allowlist = ["core", "extra_prompt", "pool.relationship-intel"]
audit = true
```

## Architecture files

`architecture.toml` lists the providers the kernel should mount at boot:

```toml
[[rows]]
id = "core"
source = "native"
disabled = false

[[rows]]
id = "extra_prompt"
source = "config_row"
disabled = false
kind = "prompt.section"
config = { path = "/etc/prompt.md" }
```

`architecture.patch.toml` overlays modify the base. Three operations:

```toml
[[disable]]
id = "old_thing"

[[replace]]
id = "core"
source = "wasm"
config = { path = "/opt/core.wasm" }

[[insert]]
id = "watcher"
source = "config_row"
kind = "prompt.section"
```

Replace preserves the row's `disabled` flag (replacing does not
silently re-enable a previously-disabled row). Insert fails on duplicate
ids; use replace to mutate.

## CLI

`operant architecture dump` — print the resolved provider tree
(text by default; `--json` for scripts).

`operant architecture validate` — load + patch + build. Non-zero exit
on any error.

`operant architecture pool-import` — compile a `~/.hermes/systems/<pool>/_pool.yaml`
into architecture rows. Read-only verb check enforced.

## Security

- **Mount allowlist** — `[harness].mount_allowlist` is a deny-by-default
  list of provider ids that may mount at boot. Anything outside the
  list is rejected.
- **Approval-gated self-extension** — `harness_mount` and
  `harness_unmount` (model tools) require approval-policy membership.
  `harness_dump` is read-only and always available.
- **Pools are read-only** — the v1 pool compiler rejects any
  `services_offered` whose name does not start with a read-only verb
  (`query.`, `fetch.`, `list.`, `search.`, `get.`, `read.`, `lookup.`).
- **WASM signature** — every WASM load re-validates the Ed25519
  signature via the existing `operant-plugins/src/signature.rs` path.
- **Generation ABA** — every swap bumps `SwapGeneration`; readers that
  snapshotted before a swap see a mismatch on next observation.
- **Audit** — every mount/unmount/swap produces an audit line when
  `[harness].audit = true`.

## Composition kinds and their status

| `source`         | `kind`                | Status (R16)                                 |
|------------------|-----------------------|----------------------------------------------|
| `native`         | (none)                | Mounts as `NativeRowStub` (carries config)   |
| `wasm`           | (none)                | Mounts as `NativeRowStub` (host wires Extism) |
| `config_row`     | `disable`             | Logged; no provider produced                 |
| `config_row`     | `prompt.section`      | Builder rejects (no host handler yet)        |
| `pool`           | `pool.family`         | Compiler only; Builder rejects (host wires)  |
| `pool`           | `pool.bundle`         | Compiler only; Builder rejects (host wires)  |

The Builder's rejection of unhandled config-row / pool kinds is
**intentional and tested** — it surfaces what's missing for the
operator, not a bug.

## What ships in R16 (this branch)

1. `crates/operant-harness` — pure-kernel crate: `Claim`, `Provider`,
   `Seam`, `Effect`, `Harness`, `Composition` / `Builder` /
   `Architecture` / `Patch`, `SwapGeneration` / `SwapOutcome`,
   `pool` compiler.
2. Native seams in `operant-core` and `operant-runtime`:
   `tool` (via `ToolSeam` + `register_dyn` / `unregister_tool`),
   `hook.*` (via `DynamicHooks` slot + `HooksSeam`),
   `memory.provider`, `gateway.command`, `channel.adapter` (all three
   in `harness_seams_r3.rs`),
   `prompt.<key>` (via `PromptSections` slot + `PromptSectionSeam`).
3. `operant architecture` CLI: `dump`, `validate`, `pool-import`.
4. Model tools: `harness_dump` (live, read-only), `harness_mount` and
   `harness_unmount` (structurally present, gated behind approval
   policy — Phase 5 host boot pass).
5. `plugin-authoring` skill (`skills/software-development/plugin-authoring/`).
6. `crates/operant-config` `[harness]` table (default-off, byte-stable).
7. Hermes pool compiler (`pool.rs` + CLI).

## What's deferred (per the plan's R-round sizing)

- **Host-side boot pass** that wires the `config_row kind=prompt.section`
  and `pool.family` / `pool.bundle` kinds to live adapter constructors.
  Currently the Builder surfaces a `NoConfigRowHandler` /
  `UnknownSource` error, which the operator can see via
  `operant architecture validate`.
- **WASM dir watcher** in operant-plugins (Phase 4 host-side). The
  kernel-level swap protocol (`SwapGeneration`) is in place.
- **Approval policy** for `harness_mount` / `harness_unmount`. The
  tools return a structured denial pointing at this exact wiring
  point.
- **A 30-turn soak test** demonstrating mount + observe + keep-or-
  rollback. The skill describes the loop; the harness supports it.

## Acceptance (per the plan)

| Phase | Plan acceptance | This branch |
|---|---|---|
| 0 | dark-mergeable, no behavior change | ✓ (default-off, byte-stable) |
| 1 | full kernel suite green, no operant-core dep | ✓ (13 lifecycle tests, pure crate) |
| 2 | flag-off byte-stable, flag-on equivalence | ✓ (tool/hook/memory/gateway/channel/prompt seams added without changing existing surfaces) |
| 3 | patch replace/insert/disable, malformed ⇒ boot refuses, dump matches golden | ✓ (12 composition tests + 3 from_toml tests + 3 CLI test runs) |
| 4 | drop valid wasm ≤ 2s; corrupt ⇒ old serves; signature every load | partial (kernel-side ABA generation done; dir watcher + Extism integration in operant-plugins is host-side) |
| 5 | agent mounts config row; visible next turn; unauthorized ⇒ denial; 30-turn soak | partial (harness_dump live; mount/unmount stubs return structured denial; skill authored) |
| 6 | both pilots importable without recompile; write attempts structurally impossible | ✓ (pool compiler + read-only verb check; CLI surface; no write-capable adapter exists) |
| 7 | final cargo test --workspace --all-features --lib green | see audit pass below |
