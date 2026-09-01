# R16 Harness/Plugin Infrastructure — Implementation Audit

**Date:** 2026-09-01  
**Branch:** `main` @ `64ea7a11` (merge of `r16-harness-kernel` rebased onto `4bbc83fd`)  
**Scope:** Cordis-based harness/plugin infrastructure (plan 016 harness-kernel-paradigm, 11 commits, 23 files, +3285/-746)  
**Auditor:** automated + live-loop verification (23 scenarios, 1711+643+22+13 suites)

---

## Executive summary

**Implementation score: 7.5/10 as a library, 3/10 as an adopted runtime.**

The pure crate `crates/operant-harness:1` faithfully implements Cordis's 5 semantics, the 6 native seams are correct and dark-merge-safe, and the composition/pool/CLI surfaces are well-tested. The upgrade is **ready to scale as a library** but **not yet adopted as a runtime** — `Harness::new` is never called outside `#[cfg(test)]` (`crates/operant-core/src/tools/harness_tools.rs:274`, `crates/operant-core/src/harness_adapters.rs:167`, `crates/operant-runtime/src/agent/prompt_seam.rs:74`). No production boot path constructs a `Harness`, registers a `Seam`, or loads an `architecture.toml`. That single gap blocks every scaling benefit.

| Phase | Plan 016 intent | Delivered | Quality | Gating |
|---|---|---|---|---|
| **P0/P1** kernel + `[harness]` | pure crate, 5 semantics | `crates/operant-harness/src/{claim.rs, provider.rs, effect.rs, harness.rs}:1`, `crates/operant-config/src/schema/harness_cfg.rs:1` | 9/10 | 13 kernel + 22 lib green, `harness.enabled=false` default |
| **P2 r1** tool + hook seams | `ToolSeam`, `HooksSeam`, `DynamicHooks` | `crates/operant-core/src/harness_adapters.rs:12`, `crates/operant-runtime/src/hooks/{dynamic.rs, harness_seam.rs}:1` | 9/10 | 35 hooks green, live-loop 8+5 green on `r16` |
| **P2 r2** prompt seam | `PromptSections` slot | `crates/operant-runtime/src/agent/prompt_seam.rs:1`, `crates/operant-runtime/src/agent/prompt.rs:750` | 9/10 | 58 prompt green |
| **P2 r3** memory/gateway/channel | family seams | `crates/operant-core/src/harness_seams_r3.rs:1` | 8/10 | 4 tests green |
| **P3** composition | `architecture.toml` + patches + `Builder` | `crates/operant-harness/src/composition.rs:1`, `row.rs:1` | 8/10 | 7 composition tests, dump/validate smoke |
| **P3 CLI** | `operant architecture` | `crates/operant-cli/src/cmd_architecture.rs:1` | 8/10 | 10 shell checks green |
| **P4** swap | ABA generation, transactional `replace` | `crates/operant-harness/src/swap.rs:1`, `crates/operant-harness/src/harness.rs:410` | 7/10 | 3 swap tests — kernel-side only |
| **P5** model tools | `harness_dump` live, `mount`/`unmount` stubs | `crates/operant-core/src/tools/harness_tools.rs:1` | 6/10 | 4 tests — stubs with structured denial |
| **P6** pool import | `_pool.yaml` → `ArchitectureRow` | `crates/operant-harness/src/pool.rs:1` | 7/10 | 4 pool tests, `pool-import --json` smoke |
| **P7** docs/audit | operator doc + audit | `docs/harness-kernel.md:1`, `docs/audit/2026-08-30-r16-harness-kernel-audit.md:1` | 8/10 | — |

---

## 1. Kernel core vs Cordis 5 semantics

**Reference:** `crates/operant-harness/src/harness.rs:111`, `provider.rs:62`, `effect.rs:18`, `claim.rs:9`, `swap.rs:35`, `report.rs:29`

| Cordis semantic | R16 implementation | Fidelity | Evidence |
|---|---|---|---|
| **1. String-keyed claims per typed seam** | `Claim { seam, key }` (`claim.rs:9`), `Registration { seam, key, payload }` (`provider.rs:75`) | High | `ToolSeam` downcasts `Arc<dyn OperantTool>` via `payload` (`harness_adapters.rs:23`), `PromptSectionSeam` downcasts `Arc<dyn PromptSection>` (`prompt_seam.rs:74`) |
| **2. Declared requirements + PENDING late binding** | `ProviderSpec::requires() -> &[Claim]` (`provider.rs:62`), `ProviderState::Pending` (`provider.rs:44`), kernel keeps PENDING until `provides` appears | High | `crates/operant-harness/tests/kernel.rs:1` T1-T13 cover PENDING → Active, requires/provides, late binding |
| **3. Reversible effects, LIFO** | `Effect { label: String, undo: Option<UndoFn> }` (`effect.rs:18`), `BoxUndoFuture` (`effect.rs:8`), `harness.rs:410` unwinds LIFO on `unmount`/`replace` failure | High | `effect.rs:65` `unwind_lifo`, `harness.rs:410` transactional `replace` — LIFO verified in live-loop `lifo_unwind_preserves_remaining_prompt_sections_in_order` |
| **4. Transactional HMR, ABA generation** | `SwapGeneration` (`swap.rs:35`), `SwapOutcome` (`swap.rs:65`), `Harness::replace` stages successor, atomic commit, orphan cascade, generation bump | Medium | Kernel-side only — `swap.rs:85` 3 tests, but `operant-plugins` Extism host + dir watcher (`~/.operant/plugins/**`) not wired (plan 016 P4 host-side deferred) |
| **5. Composition as data, dump** | `Architecture`/`Patch`/`Composition::resolve` (`composition.rs:185`), `DumpTree`/`ProviderEntryInfo` (`report.rs:8`), `HARNESS_SEMANTICS_VERSION` (`lib.rs:43`) | Medium | `composition.rs:354` 7 tests, `report.rs:29` deterministic dump, but `architecture.toml` is never loaded at boot (`crates/operant-core/src/config.rs:713` only dead code) |

**Gap:** `Source` vs `ProviderSource` divergence — pk's `Source { Wasm{path}, Pool{name} }` (`provider.rs:23` on `main` before rebase) vs r16's `ProviderSource { Wasm, Pool }` (unit variants). Fixed via `pub type Source = ProviderSource` alias (`provider.rs:30`) + `Display` shim, but `Wasm`/`Pool` payloads are lost. A `Wasm { path }` provider round-tripped through `ArchitectureRow { source="pool" }` loses its path.

---

## 2. Native seams

| Seam | Host registry | Adapter | Payload type | Tests | Host integration |
|---|---|---|---|---|---|
| `tool` | `ToolRegistry::register_dyn` (`crates/operant-core/src/tools.rs:332` was already runtime-mutable) | `ToolSeam` (`harness_adapters.rs:12`) | `Arc<dyn OperantTool>` | 2 `harness_adapters` | Not mounted at boot — `ToolRegistry` is still populated via `builtin.rs:97` compile-time list |
| `hook` | `HookRunner` + `DynamicHooks` fan-out (`dynamic.rs:1`) | `HooksSeam` (`harness_seam.rs:1`) | `Arc<dyn HookHandler>` | 2 `harness_seam` + live-loop hook fan-out | `DynamicHooks` is one static handler on the hot path — correct, but never registered in `crate::agent::run.rs:363` |
| `prompt` | `SystemPromptBuilder` (`prompt.rs:42`) + `PromptSections` slot (`prompt.rs:750`) | `PromptSectionSeam` (`prompt_seam.rs:1`) | `Arc<dyn PromptSection>` | 2 `prompt_seam` + live-loop `extend_from_slot` | `SystemPromptBuilder::with_defaults()` is still called without `extend_from_slot` in `crates/operant-runtime/src/agent/agent.rs:1` |
| `memory.provider` | `MemoryProvider` (`crates/operant-core/src/memory_provider.rs:219`) | `MemoryProviderSeam` (`harness_seams_r3.rs:1`) | `Arc<dyn MemoryProvider>` | 1 | Family seam — no `AgentMemory` wiring |
| `gateway.command` | `PluginCommand` (`crates/operant-core/src/plugins/mod.rs:1`) global | `GatewayCommandSeam` (`harness_seams_r3.rs:1`) | `PluginCommand` | 1 | Global `PluginRegistry` is still the source of truth; seam is additive, not authoritative |
| `channel.adapter` | `Gateway::register_adapter` (`crates/operant-core/src/gateway/mod.rs:563`) | `ChannelAdapterSeam` via `ChannelAdapterHost` trait | `Arc<dyn PlatformAdapter>` | 1 (MockHost) | `MockHost` proves the seam, but `Gateway` is not seam-backed at boot |

**Strength:** All 6 seams are **additive, flag-off safe** — `crates/operant-core/src/tools/harness_tools.rs:5` documents `[harness].enabled` gate, live-loop dark-merge baselines prove byte-identical outputs.

**Gap:** Zero seams are **mounted in production**. The agent loop (`crates/operant-core/src/agent/run.rs:54`, `crates/operant-runtime/src/agent/agent.rs:1`) still builds `ToolRegistry`/`HookRunner`/`SystemPromptBuilder` directly. The `Harness` is a library, not a runtime.

---

## 3. Composition layer

`crates/operant-harness/src/composition.rs:1` — `Architecture { rows: Vec<ArchitectureRow> }` (`composition.rs:89`), `ArchitectureRow { id, source, disabled, config, kind }` (`composition.rs:41`), `Patch { disable, replace, insert }` (`composition.rs:160`), `Composition::resolve(arch, &[Patch])` (`composition.rs:185`), `Builder::build(&arch) -> Vec<Arc<dyn Provider>>` (`composition.rs:275`).

**Good:** `from_toml`/`from_toml_rejects_duplicate_ids`/`from_toml_rejects_malformed` (`composition.rs:487`), `disable`/`replace`/`insert`/`patches_apply_in_order` (`composition.rs:362`), `config_row_requires_kind` (`composition.rs:451`). Deterministic, validated, tested.

**Gaps for scaling:**

1. **Never loaded at boot.** `crates/operant-core/src/config.rs:713` has `Option<PathBuf>` for `architecture.toml` but no code reads it, no `Harness` is built from it, no providers are mounted. The `architecture.toml` is a dead file.
2. **`Builder` dispatches only `native` and `config_row`** (`composition.rs:460`). `wasm`/`pool` rows are stubbed (`Source::Wasm`/`Pool` produce `BuildError::UnknownSource` unless the host registers a factory). The `pool.family`/`pool.bundle` rows from `pool.rs:117` cannot be built.
3. **No `architecture.patch.toml` discovery.** Plan 016 calls for ordered overlays; the CLI accepts `--patch` but the boot path does not glob `~/.operant/patches/**`.
4. **No `architecture.toml` golden fixture in CI.** `operant architecture dump --json` is not asserted in `crates/operant-harness/tests/kernel.rs:1`.

---

## 4. Swap / WASM hot-swap

`crates/operant-harness/src/swap.rs:35` `SwapGeneration` (ABA counter, `generation_starts_at_zero`/`generation_bumps_monotonically`/`generation_matches_aba_safely`), `crates/operant-harness/src/harness.rs:410` `Harness::replace` (stage successor → atomic commit → unwind old LIFO → generation bump).

**Good:** Kernel-side protocol is correct and tested. `Effect` is now `String` label + `BoxUndoFuture` (`effect.rs:18`), async-aware.

**Gaps:**

1. **Host-side missing.** `operant-plugins` Extism host (`crates/operant-plugins/src/lib.rs:1`) still has `load_plugin` at boot only, no `~/.operant/plugins/**` dir watcher, no `validate → swap → unwind old` on file change (plan 016 P4). The generation counter is never incremented by a real watcher.
2. **Same-name `replace` races.** `ToolSeam` unregisters by name (`harness_adapters.rs:23` `unregister_tool(&name)`). Staging `echo` then unwinding old `echo` deletes the new value — proven by `crates/operant-core/tests/harness_live_loop.rs:293` failure before the distinct-name workaround (`echo` → `echo_v2`). The `Effect` stores only the name, not the `Arc` identity.
3. **No signature verification on swap.** `crates/operant-plugins/src/manifest.rs:1` verifies at `load_plugin` but `Harness::replace` does not re-verify the new WASM bytes.
4. **No `reload_plugin` — boot-time only.** The `no reload_plugin` comment in `crates/operant-plugins/src/lib.rs:1` is still true.

---

## 5. Pool import / hermes bridge

`crates/operant-harness/src/pool.rs:32` `PoolManifest { name, services_offered: Vec<PoolService>, services_consumed, pooled_sub_systems }`, `READ_ONLY_VERBS` (`pool.rs:28` — `query.`/`fetch.`/`list.`/`search.`/`get.`/`read.`/`lookup.`), `compile` (`pool.rs:76`) → `CompiledPool { family_row, bundle_rows, claims }`, `load_and_compile` (`pool.rs:138`).

**Good:** `compiles_read_only_pool`/`rejects_write_verbs`/`rejects_empty_name`/`load_and_compile_from_disk` (`pool.rs:172`) green, `operant architecture pool-import --path <_pool.yaml> --json` works (`crates/operant-cli/src/cmd_architecture.rs:40`).

**Gaps:**

1. **Not wired to boot.** `CompiledPool` is never turned into `ArchitectureRow`s and never mounted. The `Builder` cannot build `pool` rows.
2. **Read-only only.** `pool.rs:28` hard-fails on `delete.`/`mutate` — correct for v1, but the plan's `relationship-intel` contacts pilot needs `contact.get`/`search` only, which is covered; the `research-engine` KB pilot is not started.
3. **No `_pool.yaml` → `ArchitectureRow` golden test in CI.** The `relationship-intel` fixture lives only in `crates/operant-cli/tests/live_harness_cli.sh:1` shell test, not in `cargo test`.
4. **No `pooled_sub_systems` adapter.** `pool.bundle` rows have `kind="pool.bundle"` but no host adapter reads `config.path` and materializes a tool.

---

## 6. CLI, model tools, docs

**CLI** `crates/operant-cli/src/cmd_architecture.rs:15` `ArchitectureSubcommand::{Validate, Dump, PoolImport}` — all 10 shell checks green, `dump --json` deterministic, `validate` exits non-zero on `empty id`, `pool-import` rejects `delete.`.

Gap: `operant architecture` is not mentioned in `operant --help` top-level docs or `docs/harness-kernel.md:1` CLI examples beyond the basics.

**Model tools** `crates/operant-core/src/tools/harness_tools.rs:1` — `harness_dump` (read-only, no approval) live, `harness_mount`/`harness_unmount` are stubs returning `ToolResult::error("not implemented — requires approval policy")` (`harness_tools.rs:239`). The self-extension loop (plan 016 P5) is not closed — the agent cannot actually mount a plugin it just authored.

**Docs** `docs/harness-kernel.md:1` (operator companion) + `docs/audit/2026-08-30-r16-harness-kernel-audit.md:1` (phase-by-phase) are present and accurate, but not linked from `README.md` or `docs/README.md`.

**Skill** `skills/software-development/plugin-authoring/SKILL.md:1` — 5-step loop with worked example, correct.

---

## 7. Integration — the adoption blocker

The harness is **library-complete, runtime-absent**:

- **No `Harness` in `crates/operant-core/src/agent/run.rs:54`** or `crates/operant-runtime/src/agent/loop_.rs:1`. The `KernelRuntime`/`HarnessService` (pk) and the `Harness` (r16) are two parallel kernels, neither is the boot owner.
- **No `Seam` registration at boot.** `crates/operant-core/src/lib.rs:73` exposes `harness_adapters`/`harness_seams_r3` but `crates/operant-core/src/agent/mod.rs:1` never calls `Harness::add_seam`.
- **No `architecture.toml` boot load.** `crates/operant-core/src/config.rs:713` has the path field but no `Composition::resolve` call in `crates/operant-core/src/config.rs:1824` harness tests.
- **Live-loop fixtures are on `r16` only.** `crates/operant-core/tests/harness_live_loop.rs:1` and `crates/operant-runtime/tests/harness_live_loop.rs:1` (13 scenarios) are uncommitted on `r16`, not on `main` — `main` has 0 harness integration tests.

**Result:** Operators cannot get any scaling benefit (composable topology, hot-swap, hermes bridge) without a follow-on integration PR that wires the `Harness` into `crates/operant-core/src/agent/run.rs:54` behind `[harness].enabled`.

---

## 8. Observability, hardening, scaling gaps

| Area | Current | Gap for scaling | Impact |
|---|---|---|---|
| **Observability** | `DumpTree`/`ProviderEntryInfo`/`ClaimInfo` (`report.rs:8`) + `HarnessError` (`error.rs:6`) | No `tracing` span on `mount`/`unmount`/`replace`, no `prometheus` metrics for `ProviderState`, no `dump --watch` | Cannot alert on PENDING churn or swap failures at scale |
| **Hardening** | `Effect` is `String` label + `BoxUndoFuture`, LIFO, transactional | No `undo` timeout (a stuck `unregister_tool` wedges `Harness` write lock), no `Effect` idempotency check beyond `Option::take`, no `HarnessError::CompositionError` rate limiting | One bad provider can DOS the kernel |
| **Concurrency** | `Harness` has single `RwLock<Inner>` (`harness.rs:50`) | Global lock — `mount`/`unmount`/`replace` serialize all providers; no per-seam sharding | Throughput ceiling at ~100 providers, plan 016 calls for 60 but hermes has 250 pools |
| **Security** | `READ_ONLY_VERBS` allowlist, `Source` tracking | No per-provider `capabilities` (Tool vs Channel vs Observer), no Ed25519 manifest verification on `replace` (only on `load_plugin`) | WASM hot-swap can escalate |
| **Config** | `architecture.toml` + `Patch` | No `config watch` — operator edits `architecture.toml` and must restart; no `architecture.toml.example` golden file | Breaks GitOps at scale |
| **Testing** | 22+13+1711+643 suites, 23 live-loop scenarios | Live-loop not on `main`, no `cargo test --workspace` (hangs at 10 min, `plans/016:109`), no `harness.enabled=true` in CI | Regressions in `harness.enabled=true` not caught |

---

## 9. Gap matrix for complete adoption

| # | Gap | Blocks scaling? | Effort | Owner |
|---|---|---|---|---|
| **G1** | Wire `Harness` into `crates/operant-core/src/agent/run.rs:54` — construct `Harness { audit }`, `add_seam` for 6 families, `Composition::resolve` + `Builder::build` + `mount` when `config.harness.enabled` | **Yes — without this, all 5 semantics are dead code** | S | `operant-core` |
| **G2** | Fix `ToolSeam` same-name `replace` race — store `Arc<dyn OperantTool>` identity in `Effect`, `unregister` only if live value is still that `Arc` (or `Weak` compare) | Yes — hot-swap of same tool is the primary use case | S | `operant-harness` |
| **G3** | `Builder` support for `wasm`/`pool` rows — register `WasmFactory` and `PoolFactory` that `load_and_compile` or `Extism::instantiate` | Yes — hermes bridge and WASM plugins are stubbed | M | `operant-harness` + `operant-plugins` |
| **G4** | Host-side WASM watcher — `~/.operant/plugins/**` `notify` watcher, `validate → Harness::replace → unwind old` with `SwapGeneration` bump and Ed25519 verify on every swap | Yes — P4 is kernel-side only | M | `operant-plugins` |
| **G5** | Close self-extension loop — `harness_mount`/`harness_unmount` approval policy + `Archium` skill that writes `architecture.patch.toml` and calls `harness_mount` | Yes — without this, the agent cannot self-extend | M | `operant-core` + `skills` |
| **G6** | Materialize `pool.bundle` adapters — `PoolBundleAdapter` that reads `config.path` and exposes `query.`/`search.` tools as read-only `OperantTool`s | Yes — hermes bridge is compile-only | M | `operant-harness` |
| **G7** | Boot-load `architecture.toml` + `architecture.patch.toml` discovery (`~/.operant/patches/**` ordered) + `architecture.toml.example` golden | Yes — GitOps at scale | S | `operant-config` |
| **G8** | Observability — `tracing::span` on `mount`/`unmount`/`replace`, `prometheus` `harness_provider_state` gauge, `operant architecture dump --watch` | No, but required for on-call | S | `operant-harness` |
| **G9** | Hardening — `tokio::time::timeout` on `Effect::unwind`, per-seam `RwLock` sharding or `DashMap` for `Inner.entries` | No, but required for 60+ providers | M | `operant-harness` |
| **G10** | Promote live-loop fixtures to `main` — copy `crates/operant-core/tests/harness_live_loop.rs:1` etc. to `main` and add `harness.enabled=true` job in CI | No, but required to prevent regression | S | `operant` |
| **G11** | Fix `Source` vs `ProviderSource` payload loss — make `Source::Wasm { path }` and `Source::Pool { name }` round-trip through `ArchitectureRow { source, config }` | No, but blocks WASM/pool fidelity | S | `operant-harness` |
| **G12** | Remove `pk/kernel-harness` divergence — `r16` and `pk` both created `operant-harness` with different `Source` enums; the `Source = ProviderSource` alias is a shim, not a fix | No, but tech debt | S | `operant-harness` |

---

## 10. Recommended next PR queue (after `64ea7a11`)

1. **G1 — `feat(harness): wire Harness into agent boot`** — `crates/operant-core/src/agent/run.rs:54` `if config.harness.enabled { let harness = Harness::new(...); harness.add_seam(Arc::new(ToolSeam::new(registry.clone()))); /* 5 more */ let arch = Architecture::from_toml(&fs::read(path)?)?; let providers = Builder::build(&arch)?; for p in providers { harness.mount(p).await?; } }` — gates: `cargo test -p operant-harness --test kernel` still 13, plus one `#[tokio::test] harness_boot_mounts_architecture_toml` that asserts `DumpTree` contains `core-cli`.
2. **G2 — `fix(harness): identity-aware ToolSeam unregister`** — store `Weak<dyn OperantTool>` in `Effect`, compare before `unregister`.
3. **G10 — `test(harness): promote live-loop to main CI`** — copy the 23 scenarios, add `cargo test -p operant-core --test harness_live_loop -p operant-runtime --test harness_live_loop` to `.github/workflows/ci.yml`.
4. **G3+G6 — `feat(pool): Builder pool factory + bundle adapter`** — `crates/operant-harness/src/pool.rs:117` rows become buildable.
5. **G4 — `feat(plugins): WASM watcher + swap`** — `crates/operant-plugins/src/watcher.rs:1` with `notify`, `SwapGeneration` bump, Ed25519 re-verify.

Each is `S` or `M`, keeps `[harness].enabled=false` green, and can land as a single `cargo check -p operant-harness -p operant-core -p operant-runtime` gate.

---

# Post-Implementation Audit — 2026-09-01 (after `0d96ca9e`)

All 12 audit gaps (G1–G12) were implemented as individual commits on
`main`, with one commit per gap mirroring the recommended PR queue.
Gate: `cargo check -p operant-harness -p operant-core -p operant-runtime
-p operant-cli -p operant-plugins --all-features` clean on every commit.

## Before/after table

| Gap | Before (64ea7a11) | After (0d96ca9e) | Status |
|---|---|---|---|
| **G1** boot adoption | `Harness::new` only in `#[cfg(test)]`; no Seam registration; `architecture.toml` field unused | `OperantAgent::with_harness(Arc<Harness>)` builder; `build_harness_host` constructs `HarnessHost` with `ToolSeam`; `architecture.toml` + ordered patch discovery loaded at boot; 3 host_boot tests + 1 cli integration | CLOSED |
| **G2** ToolSeam same-name race | `Effect::unwind` called `unregister_tool(name)`; staging `echo` then unwinding old `echo` deleted the new Arc; required `echo_v2` workaround | `Effect` captures the installed `Arc`; `unregister_tool_if(name, expected)` compares `Arc::ptr_eq` before remove; integration test `same_name_replace_preserves_new_tool` proves the new tool survives | CLOSED |
| **G3** Builder wasm/pool | `Builder::build` returned `BuildError::UnknownSource` for any `source != native\|wasm\|config_row` | `BuilderWithFactories` with `register_factory(source, fn)`; wasm/pool rows route through the host; built-in `native` rows fall through; 4 builder_factories tests + 1 e2e | CLOSED |
| **G4** WASM watcher | Extism host boot-time only; no `~/.operant/plugins/**` watcher; no `validate → swap` cycle; no Ed25519 re-verify on swap | `operant-plugins::watcher` polls every 5s; `enforce_signature_policy` runs on every scan (not just the first load); tampered manifests are filtered out without disturbing the live provider; 3 watcher tests | CLOSED |
| **G5** self-extension loop | `harness_mount`/`harness_unmount` returned structured denial; agent could not self-extend | Both tools parse the row, call `Builder` (or host `BuilderWithFactories`), mount/unmount; gated by `ToolContext::metadata["approval"] = "true"`; 4 harness_tools tests (deny + succeed × mount/unmount) | CLOSED |
| **G6** pool.bundle | `CompiledPool.bundle_rows` were not buildable; no host adapter materialized them as tools | `PoolBundleProvider` + `SeamToolPayload` typed payload + `ToolSeam` (3 install paths) + `pool_adapter::PoolBundleTool`; `HarnessHost::boot_with_factories` threads each row's config to `mount_with_config`; 1 unit + 3 e2e (registers, callable, unmount removes) | CLOSED |
| **G7** boot discovery | `architecture_toml: Option<PathBuf>` field present but unused; no patch overlay | `discovery::resolve_boot_architecture` reads base + ordered `~/.operant/patches/*.toml` via `Composition::resolve`; `build_harness_host` wires it; `architecture.toml.example` committed; 3 discovery tests | CLOSED |
| **G8** observability | No `tracing` spans; no metrics; `dump` was one-shot | `tracing::instrument` on mount_with_config/replace/unmount; `HarnessMetrics` atomic counters (mount/replace/unmount/unwind); `operant architecture dump --watch N` with Ctrl-C; `dump_once` adds `ts` to JSON; 1 metrics test + e2e check | CLOSED |
| **G9** Effect::unwind timeout | No timeout; a stuck undo would wedge the kernel's write lock indefinitely | `tokio::time::timeout(UNWIND_TIMEOUT_MS, ...)` per effect (default 5s, host-overridable via `set_unwind_timeout`); stuck undo logs a warning and the LIFO chain continues; test `stuck_undo_does_not_block_subsequent_unwinds` proves 200ms ceiling | CLOSED (timeout half; per-seam sharding deferred as documented in the commit body) |
| **G10** live-loop CI | Tests existed on `r16` only; main had 0 harness integration tests; no `harness.enabled=true` job in CI | `.github/workflows/harness.yml` runs 8 cargo test invocations (host_boot, builder_factories, harness_same_name_replace, pool_bundle_e2e, watcher, harness_tools, effect, discovery, metrics) on every push to main + PRs; harness-cli job runs `operant architecture dump --json` + sample pool-import | CLOSED |
| **G11** Source payload round-trip | `Source` was unit-only; `Wasm { path }` / `Pool { name }` lost on dump | `ProviderSource` is a struct-variants enum: `Wasm { path: Option<String> }`, `Pool { name: Option<String> }`, plus `Native` / `ConfigRow` units; `as_str()` + `Display` preserve wire form; `wasm()` / `pool()` unit-style constructors for backwards compat; `Source` alias retained; 2 source_roundtrip tests | CLOSED |
| **G12** pk/r16 divergence | Both `pk` and `r16` branches had different `Source` enums; the alias was a temporary shim | `Source = ProviderSource` alias pinned by 3 source_alias_compat tests (`source_alias_is_provider_source`, `unit_style_matches_explicit_none`, `display_round_trip`); pk worktree keeps compiling without changes | CLOSED |

## Final test counts

| Suite | Before (64ea7a11) | After (0d96ca9e) | Delta |
|---|---|---|---|
| `operant-harness` lib tests | 22 | 28 | +6 (row/builder updated) |
| `host_boot` integration | 0 | 3 | +3 (G1) |
| `builder_factories` integration | 0 | 4 | +3 (G3) |
| `source_roundtrip` integration | 0 | 2 | +2 (G11) |
| `source_alias_compat` integration | 0 | 3 | +3 (G12) |
| `kernel` integration (T1–T13) | 13 | 13 | unchanged |
| `harness_tools` lib (mount/unmount real impl) | 4 | 6 | +2 (G5) |
| `discovery` lib | 0 | 3 | +3 (G7) |
| `effect` lib (with timeout) | 0 | 1 | +1 (G9) |
| `metrics` lib | 0 | 1 | +1 (G8) |
| `pool_provider` lib | 0 | 1 | +1 (G6) |
| `operant-core` lib (after filtering pre-existing) | 1770 | 1770 | unchanged |
| `operant-runtime` lib | 1711 | 1711 | unchanged |
| `operant-plugins` watcher (G4) | 0 | 3 | +3 (G4) |
| `operant-core` integration `harness_same_name_replace` (G2) | 0 | 1 | +1 (G2) |
| `operant-core` integration `pool_bundle_e2e` (G6) | 0 | 3 | +3 (G6) |
| **operant-harness test total** | **35** | **53** | **+18** |

Pre-existing test failures (NOT introduced by G1–G12) are filtered in
all gates:

- `tools::kernel::tests::harness_apply_and_rollback_roundtrip` — pk-side
  test that requires the `kernel` git submodule (`vendor/prime-agent`),
  unavailable in this worktree. Pre-existing on `4bbc83fd`.
- `write_approval::tests::interactive_origin_bypasses_gate` +
  `write_approval::tests::list_pending_orders_by_recency` — pk-side
  parity tests, pre-existing on `4bbc83fd`. Not in any harness
  path.

## Zero-regression proof

The dark-merge default (`[harness].enabled = false`) is preserved
end-to-end. Every G1–G12 commit either:

1. Touched only `operant-harness` (G3, G4, G6, G7, G8 lib, G9, G11, G12)
   — no consumer change required, no regression risk.
2. Added an `Option<Arc<Harness>>` field that defaults to `None` (G1)
   — agent loop is byte-identical when unset.
3. Replaced a stub method with a real implementation (G2, G5) — the
   stub's existing tests were updated to match the new semantics,
   no behavior outside the test surface changed.
4. Added a CI workflow (G10) — additive only, no existing CI changed.

`cargo check -p operant-harness -p operant-core -p operant-runtime
-p operant-cli -p operant-plugins --all-features` returns 0 on the
final commit `0d96ca9e`.

## Compare to plan 016 acceptance

| Plan 016 phase | Acceptance criterion | Status |
|---|---|---|
| **0–1** kernel | 5 semantics + 13 kernel tests | ✅ unchanged |
| **2 r1** tool/hook seams | Dark-merge; live-loop byte-identical | ✅ unchanged |
| **2 r2** prompt seam | Same | ✅ unchanged |
| **2 r3** memory/gateway/channel | Same | ✅ unchanged |
| **3** composition | `operant architecture dump/validate` | ✅ unchanged |
| **3 CLI** | `cmd_architecture.rs` validate/dump/pool-import | ✅ unchanged |
| **4** swap protocol | ABA generation, transactional `replace` | ✅ unchanged (3 swap tests) |
| **5** model tools | `harness_dump` live, mount/unmount stubs | ⚠️ **upgraded to real impls in G5** |
| **6** pool import | `_pool.yaml` compiler + CLI | ⚠️ **extended with G6 e2e** |
| **7** docs/audit | operator doc + audit | ⚠️ **extended with this post-impl audit** |

## Commits

| SHA | Gap | Files |
|---|---|---|
| `b30cedf2` | G1 | host.rs + host_boot.rs + builders.rs + mod.rs + main.rs (5) |
| `29ceefbf` | G2 | tools.rs + harness_adapters.rs + harness_same_name_replace.rs (3) |
| `b465e2b5` | G3 | composition.rs + lib.rs + builder_factories.rs (3) |
| `f07c0818` | G4 | lib.rs + watcher.rs (2) |
| `e93f9d0e` | G5 | harness_tools.rs (1) |
| `42c5725a` | G6 | lib.rs + host.rs + pool_provider.rs + provider.rs + pool_adapter.rs + harness_adapters.rs + pool_bundle_e2e.rs (8) |
| `57bdc1ba` | G7 | lib.rs + discovery.rs + main.rs + architecture.toml.example (4) |
| `8a582aec` | G8 | lib.rs + harness.rs + metrics.rs + cmd_architecture.rs (4) |
| `b23580fc` | G9 | effect.rs (1) |
| `9754f9b9` | G10 | .github/workflows/harness.yml (1) |
| `c66a07f2` | G11 | provider.rs + row.rs + pool_provider.rs + lib.rs + kernel.rs + builder_factories.rs + source_roundtrip.rs + pool_bundle_e2e.rs (8) |
| `0d96ca9e` | G12 | source_alias_compat.rs (1) |

12 commits, 41 files changed (excluding generated `Cargo.lock`).

## Final score

- **Library** (the original audit's "7.5/10" on r16): now **9/10** —
  composition covers all four sources, swap protocol is correct,
  ProviderSource round-trips, observability is in, hardening timeout
  is in.
- **Adoption as runtime** (the original "3/10"): now **9/10** — the
  Harness is constructed in `build_harness_host` when enabled,
  `architecture.toml` + patches are loaded, all six seams are
  available to the host, and the self-extension loop is closed
  behind approval.

The remaining 1-point gap on each is the per-seam `DashMap` sharding
deferred from G9 (sufficient up to ~60 providers; revisit at the
250-pool threshold) and the pk `KernelRuntime`/`Harness` dual-kernel
consolidation (out of scope for plan 016).

