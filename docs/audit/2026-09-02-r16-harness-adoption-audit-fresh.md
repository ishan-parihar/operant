# R16 Harness/Plugin Infrastructure — Fresh Adoption Audit (Post S1–S10 Verification)

**Date:** 2026-09-02  
**Branch:** `main` @ `489eb1e2` (S1–S10 batch on `f1842bcb`)  
**Baseline:** `f1842bcb` (re-audit 8.0 library / 5.0 runtime) → `489eb1e2` (final audit claimed 8.5 / 8.0)  
**Scope:** Cordis-based `operant-harness` / `operant-plugins` upgrade — can it now **scale the project** (250 hermes pools, 60+ providers, agent self-extension) and support evolution without rebuild?  
**Method:** Every claim checked against live `file:line` and runnable gate. “Closed” means a `cargo test` proves mount→observe→unmount through the same path the operator uses at boot. Code-exists-without-boot-wiring = library, not adoption.

**Gates run for this audit:**
- `cargo check -p operant-harness -p operant-core -p operant-runtime -p operant-cli -p operant-plugins --all-features` → **0** (plugins 0.05s, harness 0.22s, cli 31s — one `unused_mut` warning `cmd_architecture.rs:197` `let mut stop`)
- `cargo test -p operant-harness --tests` → **54 passed** (28 lib + 3 host_boot + 4 builder_factories + 13 kernel + 1 soak + 3 source_alias + 2 source_roundtrip)
- `cargo test -p operant-harness --lib` → **28 passed**
- `cargo test -p operant-core --tests` → **1785 passed; 3 failed** (pre-existing `kernel::tests::ping_roundtrip`, `kernel::tests::harness_apply_and_rollback_roundtrip` requiring pk submodule, `write_approval::tests::list_pending_orders_by_recency` — filtered in CI)
- `cargo test -p operant-runtime --lib` → **1711 passed**
- `cargo test -p operant-plugins --lib` → **44 passed**

---

## 1. Executive verdict

**The upgrade is 8.6/10 as a library, 6.2/10 as an adopted runtime. The prior final audit’s 8.5/8.0 overstates runtime by ~1.8 points — three S-class gaps are genuinely closed, two are partially closed, and the S1 “4 seams live” claim miscounts.**

| Dimension | `f1842bcb` (re-audit) | Final audit claim (`489eb1e2`) | This audit (live) | Why the delta |
|---|---|---|---|---|
| **Library** (kernel + seams + composition) | 8.0/10 | 8.5/10 | **8.6/10** | `prompt.section` + `pool.family` now buildable and tested; composition + seams are correct and pure-crate invariant held |
| **Adoption as runtime** (boot → self-extension → hermes) | 5.0/10 | 8.0/10 | **6.2/10** | `ToolSeam` + `memory.provider` + `gateway.command` live (3/6), `harness_dump/mount/unmount` visible, default `architecture.toml` discovery, live dump + status — but `prompt`/`hook`/`channel` remain test-only, watcher not spawned, WASM hot-swap never exercised |

**What genuinely landed in `489eb1e2`:**
- Dark-merge-safe `HarnessHost` (`crates/operant-cli/src/main.rs:1399`) with `Harness::with_metrics` (`harness.rs:139`) and `ToolSeam` (`harness_adapters.rs:27`) + `MemoryProviderSeam` + `GatewayCommandSeam` (`harness_seams_r3.rs:27,170`) behind `BuilderWithFactories::build_with` (`composition.rs:314`) threading `row.config` via `mount_with_config` (`host.rs:79`)
- `register_harness_tools` now real (`harness_tools.rs:339` `register_dyn ×3`) called from `build_agent_core:1365` (`main.rs:1369`)
- Default `architecture.toml` discovery (`main.rs:1466` candidates `["architecture.toml", "~/.operant/architecture.toml", "operant/architecture.toml", "architecture.toml.example"]`) honoring `docs/harness-kernel.md:44`
- `ConfigRowProvider::activate` for `prompt.section` (`row.rs:113` `AutoString {content,text,path→fs::read}`) and `Builder::build` + `BuilderWithFactories::build_with` accepting it (`composition.rs:335,381`)
- `PoolFamilyProvider` (`pool_provider.rs:21` `provides=pool/<svc>` `requires=pool/<svc>`) enabling `Harness::rescan_pending_locked` (`harness.rs:309`) + `pool-import --apply` (`cmd_architecture.rs:85` append+validate+write `arch.to_toml()`)
- Watcher `.wasm` mtime tracking (`watcher.rs:134` `wasm_mtime` alongside `mtime`) with Ed25519 re-verify every scan (`watcher.rs:146` `enforce_signature_policy`)
- Metrics increments (`harness.rs:189,210,355,412,439,515`) + `dump --live` (`cmd_architecture.rs:221` live `HarnessHost` + `ToolSeam` + `DumpTree`) + `operant status` harness line (`cmd_status.rs:16` `row/active` counts)
- 30-turn soak (`soak.rs:48` mount 10 / replace 10 / unmount 10 + `generation>=1` + empty claims) + 4 `harness_agent_integration` tests (`harness_agent_integration.rs:17` tools visible, prompt buildable, pool family+beundle boot)

**What still blocks “completely adopted for scaling and evolution”:**
- Only **3/6 seams live at boot** — the harness is a tool+memory+gateway bus, not a full harness. Prompt/hook/channel seams are library-only (their only calls are in `prompt_seam.rs:158,176` and `harness_seam.rs:129,172` tests)
- Watcher is a **directory scanner, not a hot-swap loop** — no `Harness::replace` call, no `WasmProvider`, no Extism host factory for `source="wasm"` (comment at `main.rs:1446` “fall back to NativeRowStub”)
- `prompt.section` rows are **buildable but not observable** — `PromptSectionSeam::install` (`prompt_seam.rs:53` handles `String`/`Arc<String>`/`Arc<dyn PromptSection>`) is correct, but `SystemPromptBuilder::extend_from_slot` (`prompt.rs:73`) is never called from `agent.rs:395` `SystemPromptBuilder::with_defaults()` nor from `build_harness_host`
- `channel.adapter` seam wraps a `PMutex<dyn ChannelAdapterHost>` (`harness_seams_r3.rs:85`) but production `Gateway` never implements `ChannelAdapterHost` behind the harness; `MemoryProviderSeam::current()` (`harness_seams_r3.rs:39`) is never read by `load_repo_memory_manager`

---

## 2. Re-verification of S1–S10 against live code

### S1 Wire 4 seams + BuilderWithFactories on boot — ⚠️ 3/6, not 4

**Claim (final audit):** 4 seams live, `BuilderWithFactories` on boot, `row.config` threaded.

**Live:**

- `crates/operant-cli/src/main.rs:1399` `build_harness_host` creates `HarnessMetrics::new()` and `Harness::new(KernelOptions { audit: true }).with_metrics(metrics)` at `main.rs:1416`, then `host.add_seam(ToolSeam::new(registry))` `main.rs:1417`, `MemoryProviderSeam::new()` `main.rs:1419`, `GatewayCommandSeam::new()` `main.rs:1420` — **3 seams**. Hook/prompt/channel deferred with comment `main.rs:1421` “require runtime hosts (HookRunner, PromptSections, Gateway) and are wired in operant-runtime's Agent builder” — but `crates/operant-runtime/src/agent/agent.rs:395` never calls `PromptSectionSeam`/`HooksSeam`/`ChannelAdapterSeam`, and `grep -rn PromptSectionSeam crates/` outside tests has zero hits beyond `prompt_seam.rs:18` definition.
- `main.rs:1427` registers `pool` factory dispatching `pool.bundle→PoolBundleProvider` / `pool.family→PoolFamilyProvider`; WASM rows without Extism factory fall back to `NativeRowStub` (`main.rs:1446` comment, `composition.rs:323` stub path). Correct for “no Extism yet.”
- `main.rs:1510` calls `host.boot_with_factories(&builder, &arch)` — row `config` threaded via `host.rs:79` `boot_with_factories` → `mount_with_config` so `pool_provider.rs:190` `path/read_only` not lost (prior re-audit G6 gap).

**Grade:** ⚠️ — boot path now correctly uses factories and threading, but seamless count is 3 not 4; the “4/6” in final audit §3 miscounts `prompt` as live when `PromptSections` never feeds `SystemPromptBuilder`.

### S2 Register harness tools — ✅ genuinely closed

`crates/operant-core/src/tools/harness_tools.rs:339` `register_harness_tools` was no-op `let _=(registry,harness)` at `f1842bcb`; now `async` `register_dyn(dump)+register_dyn(mount)+register_dyn(unmount)` `harness_tools.rs:346`. `main.rs:1368` calls it when `harness.is_some()`. `harness_agent_integration.rs:17` `harness_tools_visible_when_enabled` asserts all three names in `registry.get_schemas()`.

### S3 Default architecture.toml — ✅ closed (with one nuance)

`main.rs:1453` explicit path tried via `resolve_boot_architecture(path, patch_dir)`; `main.rs:1466` implicit candidates `["architecture.toml", "~/.operant/architecture.toml", "operant/architecture.toml", "architecture.toml.example"]` tried in order when `config.harness.architecture_toml.is_none()`. Prior `config.rs:708` `architecture_toml: None` + `main.rs:1429` “kernel is empty” is now reachable only when no candidate exists. Nuance: `architecture.toml.example` as last resort is generous — operator editing it in place risks committing example content; prefer `copy to architecture.toml` warning already at `cmd_architecture.rs:149` logic, currently `main.rs:1498` `tracing::info!("using architecture.toml.example (copy...)")`.

### S7 prompt.section handler — ⚠️ buildable, not observable

`row.rs:113` handles `kind=="prompt.section"` reading `content`/`text`/`path` (file at activate, `ActivationFailed` on unreadable). `composition.rs:335,408` `BuilderWithFactories` and `Builder::build` accept it (no longer `NoConfigRowHandler`). `prompt_seam.rs:34` `StringPromptSection` wrapper + `install` accepting `String`/`Arc<String>`/`Arc<dyn PromptSection>` is correct. `harness_agent_integration.rs:52` proves Builder no longer rejects and `Failed` without seam is expected (seam lives in runtime). **But** `agent.rs:395` `SystemPromptBuilder::with_defaults()` never calls `extend_from_slot(slot)` even when a harness exists; the kernel’s `PromptSectionSeam::slot()` (`prompt_seam.rs:27`) is never read. Operator adding `config_row kind=prompt.section` sees `Failed` in `dump --live` unless they hand-wire the seam.

### S5 Pool family late-binding + pool-import --apply — ✅ mostly closed

`pool_provider.rs:21` `PoolFamilyProvider` reads `config.claims`/`requires` (populated by `pool.rs:99` `services_offered→Claim("pool",name)` / `services_consumed→requires`) and `activate` is claim-presence only, so `harness.rs:309` `rescan_pending_locked` rescues dependents — prior G6 “family/requires never honored” is fixed. `cmd_architecture.rs:85` `PoolImport --apply --file` loads existing arch (or empty), skips duplicate ids, validates via `BuilderWithFactories` with pool factory (`cmd_architecture.rs:120`), writes `arch.to_toml()`. `harness_agent_integration.rs:77` proves family+beundle boot via `Harness` + `registry.get_schemas()` len 1. Gap: `pool_adapter::build_pool_bundle_tool` (`pool_adapter.rs:93` requires `path`) still returns `None` if `config.path` absent — bundle rows without `path` fail activation rather than being `Pending`; acceptable for v1 read-only but not late-bindable.

### S4 WASM watcher wasm mtime + Ed25519 — ⚠️ minimal (scanner, not swap)

`watcher.rs:62` `last_seen: HashMap<PathBuf,(SystemTime,SystemTime,Option<String>)>` now `(mtime, wasm_mtime, signature)`; `watcher.rs:134` computes `wasm_mtime` from `manifest.wasm_path` → `path.join(p)`; `watcher.rs:172` change detection now `is_new||mtime_changed||wasm_changed||sig_changed`. Ed25519 re-verify already every scan (`watcher.rs:146`). **But** `watcher.rs:96` `scan_once` still only returns `Vec<ManifestChange>` — no `host.harness().replace(WasmProvider)` call, no `WasmProvider` type, no Extism `create_plugin` (`plugins/src/runtime.rs:179`) wired through `BuilderWithFactories` `source="wasm"` (currently falls back to `NativeRowStub`). The `notify` vs poll choice (`watcher.rs:13` comment “poll loop is adequate”) is fine; the missing piece is the `on_change → Harness::replace` bridge and a real hot-swap soak.

### S6 Metrics + live dump + operant status — ⚠️ half-observable

`harness.rs:189,210,355,412,439,515,585` increments `mount_success/mount_pending/mount_failed/replace_success/replace_failed/unmount_calls/unwind_invocations`; `main.rs:1416` attaches via `with_metrics` — **wired**. `cmd_architecture.rs:221` `dump_live` builds live `HarnessHost` with pool factory + `ToolSeam`, boots best-effort, prints `DumpTree` (states, generation, claims) — **live**. `cmd_status.rs:16` adds `harness: {enabled, architecture_file, max_active_providers, config_exists, row/active_count}` + human line `Harness: enabled (file: N rows, M active)` — **visible**. **But** `MetricsSnapshot` (`metrics.rs:53`) never leaves the `Harness` — `operant status --json | jq .harness` shows rows, not `mount_success/pending/failed` or `unwind_invocations`; `GET /metrics` gauge (`harness_provider_state{state}`) from prior audit remains unwired (`metrics.rs:9` comment “host can wrap” is the lazy choice but operator cannot alert on PENDING churn). `cmd_architecture.rs:197` `let mut stop = watch::channel(false).0` `unused_mut` warning confirms `dump --watch` watches files (`dump_once` `load_and_resolve`) not `Harness::dump()` when `live=false`.

### S8 Per-seam sharding — deferred correctly

Single `RwLock<Inner>` (`harness.rs:114`) retained. Mitigations: `effect.rs:19` `UNWIND_TIMEOUT_MS=5s` per effect bounds each hold to 5s; `harness.rs:270` metrics allow monitoring churn. Sharding to `DashMap` per seam would break transactional `mount` claim-conflict + insert atomicity (`harness.rs:239`). At 250 pools mounts are boot-rare + occasional agent `harness_mount`; global lock is not a throughput ceiling for this workload. Revisit only at 1000+ concurrent mounts or when `mount` latency histograms show contention — **intentionally deferred is correct**.

### S10 Soak + CI — ✅ soak real, CI live but not enabled=true agent-loop

`soak.rs:48` `soak_30_turns_no_leak` mounts 10 / replaces 10 (`generation>=1`) / unmounts 10 → empty providers/claims; `harness_agent_integration.rs:17,31,52,77` 4 tests cover tools visible, agent attach, prompt buildable, pool boot. `.github/workflows/harness.yml:34` runs `host_boot+builder_factories+soak+source_roundtrip+source_alias_compat` + `harness_same_name_replace+pool_bundle_e2e+harness_agent_integration` + watcher/harness_tools/effect/discovery/metrics + `pool-import --apply+validate+dump --live --json` + `status | grep harness`. **But** harness.yml still runs tests that construct their own `Harness` — no `cargo test` that sets `config.harness.enabled=true` and asserts the agent loop sees `harness_dump` via `build_agent_core`. The prior audit’s “no harness.enabled=true agent-loop integration test” remains.

---

## 3. Cordis 5 semantics — fidelity vs DSH reference

| Semantic | DSH form (per #15) | Operant form | Fidelity | Evidence |
|---|---|---|---|---|
| **1. String-keyed claims per typed seam** | `ctx.<key>` | `Claim {seam,key}` (`claim.rs:9`), `Registration` (`provider.rs:121`), `ProviderSpec::{provides,requires}` (`provider.rs:108`) | **9/10** | `harness_adapters.rs:56` `payload` downcasts, `prompt_seam.rs:48`, `harness_seams_r3.rs:56` all correct |
| **2. Declared requirements + PENDING late binding** | `inject` PENDING until provider appears, rescanned on success | `ProviderState::Pending` (`provider.rs:95`), `missing_requirements` (`harness.rs:66`), `pending_order: Vec<String>` (`harness.rs:55`), `rescan_pending_locked` after `mount`/`replace` (`harness.rs:194,344,561`) | **9.5/10** | `kernel.rs` T1–T13 + `pool_provider.rs:21` `requires_claims` now exercised |
| **3. Reversible effects, LIFO unwind** | `Effect` handles, HMR rolls back old | `Effect {label, undo: Option<UndoFn>}` (`effect.rs:40`), `unwind_lifo` LIFO (`effect.rs:103`), `teardown_many_locked` seq-desc (`harness.rs:394`) | **8.5/10** | `effect.rs:118` `stuck_undo_does_not_block_subsequent_unwinds` timeout per effect; still global lock during unwind |
| **4. Transactional swap, ABA generation** | HMR hot-swaps with transactional rollback, ABA generation counters | `SwapGeneration` (`swap.rs:39` `current/bump/matches`), `Harness::replace` stage→validate→commit→unwind old→bump (`harness.rs:432`), `stage_activation` detached (`harness.rs:573`) | **7.5/10** | Kernel protocol correct; host-side Extism→`replace` never exercised with real `.wasm` bytes |
| **5. Composition as data (config-as-composition)** | `architecture.toml` patches + `operant architecture dump` | `Architecture {rows}` (`composition.rs:92`), `Patch {disable,replace,insert}` (`composition.rs:164`), `Composition::resolve` (`composition.rs:193`), `DumpTree {semantics_version,providers,claims}` (`report.rs:30`), `HARNESS_SEMANTICS_VERSION=1` (`lib.rs:53`) | **8/10** | `composition.rs:430` 12 + `discovery.rs:96` 3 tests; `Builder::rows_of_kind` (`composition.rs:431`) dead but harmless; `Architecture::from_toml` `[[rows]]` table-array vs `[provider.<id>]` compat noted `composition.rs:95` |

**Overall kernel fidelity: 8.2/10** — five semantics individually correct and now exercised together for `pool.family`+`pool.bundle`+`prompt.section` (composition→build→mount→late-bind→dump), but never through a non-trivial WASM provider.

---

## 4. Native seams — what lives where

| Seam | Host registry it wraps | Adapter | Payload | Production call site | Live at boot? |
|---|---|---|---|---|---|
| `tool` | `ToolRegistry::register_dyn/unregister_tool_if` (`tools.rs:332,436`) | `ToolSeam` (`harness_adapters.rs:27`) `Arc<dyn OperantTool>` + `SeamToolPayload` + `pool.` fallback | 3 paths (`harness_adapters.rs:65`) | `main.rs:1417` `host.add_seam(ToolSeam::new(registry))` | **Yes** |
| `memory.provider` | `MemoryProvider` (`memory_provider.rs:219`) | `MemoryProviderSeam` (`harness_seams_r3.rs:27`) `Arc<dyn MemoryProvider>` | `Arc<dyn MemoryProvider>` | `main.rs:1419` | **Yes** |
| `gateway.command` | `PluginCommand` global (`plugins/mod.rs`) + dynamic map (`harness_seams_r3.rs:172`) | `GatewayCommandSeam` (`harness_seams_r3.rs:170`) | `PluginCommand` | `main.rs:1420` | **Yes** |
| `prompt` | `SystemPromptBuilder` + `PromptSections` slot (`prompt.rs:42,787`) | `PromptSectionSeam` (`prompt_seam.rs:18`) `Arc<dyn PromptSection>`/`String`/`Arc<String>` | `Arc<dyn PromptSection>` or `String` | Only `prompt_seam.rs:158` tests | **No — `agent.rs:395` never calls `extend_from_slot`** |
| `hook` | `HookRunner` + `DynamicHooks` fan-out (`hooks/dynamic.rs:26`) | `HooksSeam` (`hooks/harness_seam.rs:18`) `Arc<dyn HookHandler>` | `Arc<dyn HookHandler>` | Only `harness_seam.rs:129` tests | **No** |
| `channel.adapter` | `Gateway::register_adapter/remove` (`gateway/mod.rs:563`) | `ChannelAdapterSeam` via `ChannelAdapterHost` trait (`harness_seams_r3.rs:93`) | `Arc<dyn PlatformAdapter>` | Only `harness_seams_r3.rs:476` MockHost | **No — Gateway never implements host trait behind harness** |

All six seams are additive, flag-off safe (dark-merge holds). Only three are mounted in production. The harness title promises a harness, the boot delivers a **tool-central bus with two family-level companions**.

---

## 5. Composition, WASM, and hermes bridge — deep dive

### Composition layer (`composition.rs`)

**Good (unchanged):** `Architecture::from_toml`/`validate`/`to_toml` (`composition.rs:105,118,112`), `Patch {disable,replace,insert}` (`composition.rs:164`) with `deny_unknown_fields`, `Composition::resolve` ordered patches (`composition.rs:193`) with `replace` preserving `disabled` (`composition.rs:231`) and `insert` duplicate check (`composition.rs:239`), 12 composition + 3 `from_toml` tests + 3 discovery tests.

**Gaps still open:**

- No golden `architecture.toml` asserted in CI — `harness.yml:82` `architecture dump --json` just prints, no `validate` gate on the committed `architecture.toml.example`. A malformed example ships without detection.
- `Builder::rows_of_kind` (`composition.rs:431`) dead (zero calls) — harmless but confirms no caller enumerates prompt sections outside tests.
- `discovery.rs:27` `resolve_boot_architecture` correctly handles missing patch dir (`collect_patches` `discovery.rs:53` sorted lexicographically `discovery.rs:71`), but `config.harness.max_active_providers=64` (`config.rs:721`) is never enforced — `host.rs:101` `mount_all` never checks it.

### WASM / swap (`operant-plugins/src/`, `swap.rs:35`)

- Kernel `SwapGeneration::matches` ABA (`swap.rs:60`), `Harness::replace` stage→commit→unwind old→bump + orphan cascade (`harness.rs:543`) + `rescan_pending` (`harness.rs:561`) — **correct**.
- Host `PluginHost` (`host.rs`), `LoadedPlugin`, `validate_skill_bundle`, `create_plugin` Extism (`runtime.rs:179`), `enforce_signature_policy` Ed25519 (`signature.rs:187`) — **real**.
- **Gap:** No `Watcher → Harness::replace` bridge. No `WasmProvider` that wraps Extism plugin as `Provider` and routes installs through `ToolSeam`/`HooksSeam`/etc. No `validate→bump→unwind old` exercised with real `.wasm` bytes. `watcher.rs:110` `entries.flatten()` silently skips `read_dir` errors beyond `NotFound`; `watcher.rs:124` `read_to_string` errors silently `continue` — correct for scanner, but hot-swap failure would be silent too.

### Hermes pool import (`pool.rs:32`)

- `PoolManifest {name, services_offered, services_consumed, pooled_sub_systems}` (`pool.rs:35`), `READ_ONLY_VERBS` (`pool.rs:28`), `compile` (`pool.rs:78` → `CompiledPool {family_row,bundle_rows,claims}`) validating `READ_ONLY_VERBS` (`pool.rs:87`), `load_and_compile` (`pool.rs:139`), `pool-import` `--json`/`--apply` (`cmd_architecture.rs:85`) — **real**.
- **Gaps closing but not closed:**
  1. `services_consumed → requires` now honored (`pool.rs:99` → `PoolFamilyProvider::requires` `pool_provider.rs:72`) — late binding exercised at kernel level but not with a real ~250-pool dependency graph. No `relationship-intel` pilot importing without recompile — `~/.hermes/systems/` on this host has 22 pools (`architecture/`, `autonomous-executor/`, `campaigns/`, …) each with `_org.yaml`/`AGENTS.md`/`data/` but no `_pool.yaml` per `pool.rs:35` shape; `load_and_compile_pool` has four lib tests on synthetic YAML only.
  2. `pooled_sub_systems` → `pool.bundle` rows (`pool.rs:118`) with `config {path,read_only:true}` → `PoolBundleProvider` materializes `tool` claim per bundle (`pool_provider.rs:99` `Claim::new("tool",tool_name)`) — but `pool_adapter::PoolBundleTool::execute` (`pool_adapter.rs:77` returns `{kind:"pool.bundle",path,read_only}`) is a stub hint, not a real read from `path`. Host can swap in richer adapter (`harness_adapters.rs:44` `with_pool_factory`) but none does.
  3. No 250-pool scalability probe — `soak.rs:48` proves 10 mount/replace/unmount with `generation>=1` and empty claims, but concurrent 60-provider mount fuzz is only `kernel.rs: T8_concurrent_mounts_all_land`.

---

## 6. CLI, model tools, and docs

- **CLI** `cmd_architecture.rs:15` `Validate` / `Dump {file,patch,json,watch_secs,live}` / `PoolImport {path,json,apply,file}` — `Validate` now builds via `Builder::build` (`cmd_architecture.rs:324`) while `live` builds `BuilderWithFactories` with pool factory (`cmd_architecture.rs:229`). Inconsistency: `validate` still uses `Builder::build` (factory-less) so `pool` rows fail validation unless `--file` is a post-`--apply` arch. `dump --watch N` (`cmd_architecture.rs:183` `watch_secs` loop) watches files (`load_and_resolve`) not `Harness::dump()` when `live=false`; `live=true` is the kernel view but `harness.yml:115` only tests `--live --json` once.
- **Model tools** `harness_tools.rs:45,125,266` `harness_dump` (read-only, always) / `harness_mount`/`harness_unmount` (approval-gated `has_approval` `harness_tools.rs:36` `metadata["approval"]=="true"`). `HarnessDumpTool::execute` now `format!("{:?}", p.state)` Debug vs `DumpTree::to_json` (`report.rs:37`) — still Debug, not serde; operator prompt cannot parse it as JSON without `verbose=true` (`harness_tools.rs:93` `serde_json::to_string_pretty(&tree)` under `verbose`).
- **Docs** `docs/harness-kernel.md` operator companion + `docs/audit/2026-08-30-r16-harness-kernel-audit.md` phase map are accurate for library, but `docs/harness-kernel.md:120` composition table still marks `pool.bundle`/`wasm` as `Builder rejects` (after S1 they are `BuilderWithFactories` factory-only via boot) and §5 “Enable the harness” still `architecture_path` vs actual `architecture_toml` (`config.rs:717`). Not linked from `README.md`/`docs/README.md`.

---

## 7. Observability, hardening, and concurrency — scaling to 250 pools

| Area | Current state | Gap for 250 providers | Impact at scale |
|---|---|---|---|
| **Observability** | `DumpTree`/`ProviderEntryInfo`/`ClaimInfo` (`report.rs:8`), `HarnessError` (`error.rs`), `tracing::instrument` on mount/replace/unmount (`harness.rs:178,352,432`), `HarnessMetrics` wired (`harness.rs:117` + `main.rs:1416` `with_metrics`) | `MetricsSnapshot` not on `operant status --json` nor `/metrics` Prometheus gauge; `operant architecture dump --watch` watches files not kernel when `live=false`; no `operant status` pending/failed breakdown | On-call cannot alert on PENDING churn or swap failures; debug requires `harness_dump --verbose` (model tool) not CLI |
| **Hardening** | `Effect {label, undo: Option<UndoFn>}` LIFO, transactional `replace` with `stage_activation` detached, per-effect `UNWIND_TIMEOUT_MS=5s` (`effect.rs:19,72`) | Timeout per effect but `RwLock<Inner>` held across `teardown_many_locked` (`harness.rs:393` seq-desc) — one bad provider can block mount for `N×5s` (250×5s worst); no Harness-level rate limiting on `harness_mount` burst | Bad `unregister_tool` can DOS mount path for minutes with 250 providers; `max_active_providers=64` never enforced |
| **Concurrency** | Single `RwLock<Inner>` (`harness.rs:114`), `seams: HashMap<String,Arc<dyn Seam>>` (`harness.rs:113`) not behind RwLock (added only at boot) | Global lock — `mount`/`unmount`/`replace` serialize all providers; `dump` read lock blocks on write; no per-seam `DashMap` | Throughput ceiling ~100 providers burst; hermes has 22 pools today, 250 pools ambition hits boot storm (validate 250 `compile_pool` + 250 `mount_with_config` serial) |
| **Security** | `READ_ONLY_VERBS` allowlist (`pool.rs:28`), `Source` tracking (`provider.rs:25`), `SignatureMode` + `enforce_signature_policy` (`signature.rs:187`), `has_approval` (`harness_tools.rs:36`) | No per-provider `capabilities` (Tool vs Channel vs Observer vs Skill, Ed25519-signed manifests, boot-time load only) enforcement on `replace` (`harness.rs:432` preconditions only check `requires` + claim conflict, not signature); no allowlist for `hook.*` or `memory.provider`; `has_approval` string compare not integrated with `agent/mod.rs:91` `approval_allowlist` glob (`command_allowlist`) | WASM hot-swap can escalate from tool seam to memory seam if a compromised provider ships a novel `HookHandler` payload — seam `install` type check (`harness_seam.rs:40` `downcast_ref::<Arc<dyn HookHandler>>`) is the only guard |
| **Config** | `architecture.toml` + `Patch` (`composition.rs:160`) + `discovery.rs:27` + `architecture.toml.example` (native + `policy-base` prompt.section + commented wasm/pool) | No `config watch` — operator edits `architecture.toml` and must restart; `default_patch_dir()` (`discovery.rs:88` `$HOME/.operant/patches`) has no `operant architecture patches list` CLI; no `operant architecture init` to copy example→live | GitOps at 250 pools requires restart on every patch; `config.harness.audit` hard-coded `audit:true` at `main.rs:1416` not operator-tunable |

---

## 8. Plan 017 and the dual-kernel divergence — still the strategic blocker

`plans/017-perfect-unified-kernel.md` (P0) identifies five duplicate spots G1–G12 and S1–S10 did not address:

- **D1 Python execution:** `tools/code_execution.rs:148` stateless `execute_python()` vs `tools/kernel/kernel_tool.rs:14` persistent `kernel_exec` NDJSON to `kernel-sidecar`. `route_python_to_kernel` flag still gates, `code_execution` still spawns `python -c`. Harness does not own Python execution — correct scope, but operator sees two Python surfaces.
- **D2 Harness/state:** `harness.rs:14` pure Rust Cordis vs `kernel-sidecar/kernel_sidecar/harness.py` `HarnessService` (live) vs `vendor/prime-agent/prime-agent-runtime/src/rlm` upstream. S1–S10 added pure crate but did not touch sidecar; three homes for "learned state" remain.
- **D3 Vendor path:** `vendor/prime-agent` submodule + `kernel_sidecar/vendor.py:19` + `operant-harness` Rust — two implementations plus upstream.
- **D4 Provider runtime:** `ToolRegistry` (`tools.rs:1`) vs `Harness` (`harness/src/lib.rs:14`) — now bridged for `tool` seam (and two companions), but `global_runtime()` `OnceLock<Arc<KernelRuntime>>` at `tools/kernel/mod.rs:94` is a separate path. The harness is not the `ToolRegistry`; it wraps three methods.
- **D5 Skills:** `skills/mod.rs:1` `SkillManifest` vs `tools/kernel/mod.rs:85` `SkillReference {type,import,callable,call_pattern}` — two scans of `skills_dir/*/SKILL.toml`. S1–S10 did not touch.

**Result:** The kernel still cannot be `enabled=true` by default without running the same feature twice for Python/skills. 017's “one Python path, one harness, one vendor, one skill scan” is the P0 prerequisite for the harness to be the evolution engine. S1–S10 made the Cordis kernel provably correct for three seams; 017 makes it the *only* kernel.

---

## 9. Gap matrix for *complete* adoption — what remains to scale and evolve

Effort: S (<1 day), M (1–3 days), L (>3 days). “Blocks scaling” = operator cannot get the scaling benefit without this.

| # | Gap (with file:line) | Blocks scaling to 250 pools? | Effort | Owner |
|---|---|---|---|---|
| **C1** | **Wire `prompt` + `hook` + `channel.adapter` seams at boot and feed the live slots.** Change `crates/operant-cli/src/main.rs:1417` + `crates/operant-runtime/src/agent/agent.rs:395` to construct `PromptSections::new()` + `DynamicHooks::new()` when harness enabled, `host.add_seam(PromptSectionSeam::new(slot))` / `HooksSeam::new(dynamic)` before `boot_with_factories`, store slots on `HarnessHost` (or `AgentCore`), and call `SystemPromptBuilder::extend_from_slot(slot.clone()).await` + `HookRunner::register(Box::new(dynamic.clone()))` at agent construction. Without C1 only `tool` evolves without a code change — the harness title is still a bus. | **Yes — 3/6 families dead; prompt evolution is the highest-value topology change** | S | `operant-cli` + `operant-runtime` |
| **C2** | **Wire WASM watcher → `Harness::replace` with `.wasm` mtime + Ed25519 and an Extism factory.** Make `watcher.rs:96` `scan_once` return `wasm_path` + `manifest.signature` alongside `manifest`, construct `WasmProvider {id, Wasm {path}, claims}` via `runtime::create_plugin` (`runtime.rs:179`), call `validate_skill_bundle` (`host.rs:391`), then `host.harness().replace(provider)` with `enforce_signature_policy` per `swap.rs:1` protocol + `SwapGeneration` bump. Spawn `Watcher::run_until` from `main.rs:1416` when `enabled`. Until C2, P4 is kernel-only. | **Yes — WASM hot-swap is the evolution primitive; pools are read-only companions** | M | `operant-plugins` + `operant-cli` |
| **C3** | **Expose `HarnessMetrics` + live pending/failed on `operant status --json` and `dump --live`.** Change `crates/operant-cli/src/cmd_status.rs:16` to include `metrics: MetricsSnapshot` (`harness.metrics().map(|m| m.snapshot())`) + `pending/failed` counts from `host.dump().await`, and make `cmd_architecture.rs:221` `dump_live` include `metrics` in JSON. Without C3 on-call cannot alert on PENDING churn at 250 pools. | No, but required for on-call (blocks evolution without observability) | S | `operant-harness` + `operant-cli` |
| **C4** | **Prove ~22 hermes pools compile + boot without recompile (pilot, not synthetic).** Write `architecture.toml` rows via `pool-import --apply` for `relationship-intel` + `research-engine` + one more `~/.hermes/systems/*/AGENTS.md` pool, run `operant architecture validate --file architecture.toml` + `dump --live --json` | jq `.providers | length` = expected, `tool.get_schemas` includes `pool.*` tools. Without C4 the 250-pool claim is untested. | Yes — hermes bridge is the 250-pool multiplier | S | `operant-harness` + `operant-cli` |
| **C5** | **Enforce `max_active_providers` and validate `architecture.toml.example` in CI.** Change `host.rs:101` `mount_all` to error when `inner.providers.len() >= max_active_providers` (`config.rs:721` default 64), and add `harness.yml` step `architecture validate --file architecture.toml.example` so a malformed example cannot ship. | Yes at 250 — prevents OOM from 250 mounts | S | `operant-harness` + CI |
| **C6** | **Make `prompt.section` rows that hit a missing seam `Pending` not `Failed` so late binding can rescue them.** Change `row.rs:131` `cx.install_with("prompt",…)` error `MissingSeam` to return `ProviderState::Pending` (like `requires` unsatisfied) and `rescan_pending_locked` to retry once `PromptSectionSeam` appears. Current `harness_agent_integration.rs:71` asserts `Failed` — that blocks the “mount prompt section before prompt seam is live” pattern that `rescan_pending_locked` was built for. | Yes for prompt evolution when C1 ships late | S | `operant-harness` |
| **C7** | **Concurrency: per-seam lock or `DashMap` for `Inner.claims` if mount latency exceeds SLO.** Keep single `RwLock<Inner>` until `soak.rs:48` + `kernel.rs: T8` latency histograms under `tokio::test` with `tokio::join!` over 60 `mount` show contention; if median `mount` > 50ms at 250 pools, shard claims per seam family. Deferred is correct for now; measure before building. | No until 60+ concurrent | M | `operant-harness` |
| **C8** | **Resolve 017 dual-kernel divergence (explicit decision).** Either collapse `code_execution` python arm into `global_runtime().request("exec")` (017 Phase A) and unify `ToolRegistry`/`Harness` + `SkillManifest`/`SkillReference` (Phases B/C), or explicitly mark `operant-harness` as “tool bus only” and defer 017. Without a decision, `enabled=true` by default means duplication. | **Yes — blocks default-on** | L | `operant-core` + `kernel-sidecar` |

---

## 10. Recommended next PR queue (after `489eb1e2`) — keeps `enabled=false` byte-stable

| Priority | Title | file:line | Why now |
|---|---|---|---|
| 1 | `fix(harness): wire prompt + hook seams and feed slots (C1 first half)` | `main.rs:1417`, `agent.rs:395`, `prompt.rs:73` | Turns harness from tool bus to prompt/hook evolution engine; highest operator value (persona/mode/tenant overlays + policy hooks) with zero new concepts |
| 2 | `feat(harness): WASM watcher → replace with Extism factory + .wasm mtime (C2)` | `watcher.rs:96`, `swap.rs:35`, `runtime.rs:179` | Then `drop valid wasm ≤2s; corrupt ⇒ old serves; signature every load` is finally testable — the HMR primitive |
| 3 | `feat(pool): hermes pilot — compile & boot 3 real pools without recompile (C4)` | `pool.rs:78`, `cmd_architecture.rs:85`, `architecture.toml.example` | Then `relationship-intel` actually late-binds and `tool.get_schemas` shows `pool.*` |
| 4 | `feat(harness): expose MetricsSnapshot + pending/failed on status + dump --live (C3)` | `metrics.rs:53`, `cmd_status.rs:16`, `cmd_architecture.rs:221` | Then on-call alerts on PENDING churn at 250 pools |
| 5 | `feat(harness): enforce max_active_providers + validate example in CI (C5)` | `host.rs:101`, `config.rs:721`, `harness.yml` | Prevents 250-pool OOM and malformed example shipping |
| 6 | `fix(harness): MissingSeam → Pending for prompt.section (C6)` | `row.rs:131`, `harness.rs:228` | Enables “mount before seam” pattern after C1 |

Each is S or M, keeps `enabled=false` green, gates on `cargo check -p operant-harness -p operant-core -p operant-runtime --all-features` + `cargo test -p operant-harness --tests` still 54.

---

## 11. Scoring — how well was the upgrade done

| Dimension | Score | Rationale |
|---|---|---|
| **Cordis semantics fidelity** | **8.2/10** | Five semantics correct in isolation and now exercised together for `pool.family`/`pool.bundle`/`prompt.section`; never through a non-trivial WASM provider |
| **Seam correctness (per seam)** | **9/10 each** | Six seams individually correct, additive, dark-merge safe — the best part of the upgrade |
| **Seam adoption (all seams live)** | **4.5/10** | 3/6 seams mounted at boot (tool, memory.provider, gateway.command). Prompt/hook/channel remain test-only |
| **Composition as data** | **8/10** | Deterministic, validated, patched, sorted, default discovery — but `rows_of_kind` dead, example not CI-validated, `max_active_providers` unenforced |
| **WASM / hot-swap** | **5/10** | Kernel ABA + Ed25519 utils real + `.wasm` mtime now tracked; no `.wasm` has ever been hot-swapped through `Harness::replace` |
| **Hermes bridge** | **6.5/10** | Compiler + per-bundle tool + family/requires + `--apply` + live boot real and tested; pilot on real `~/.hermes/systems/` missing, adapter is hint-only |
| **Self-extension loop** | **7/10** | `harness_tools.rs:339` now registers and `build_agent_core:1368` calls it; approval gate via `metadata["approval"]` not yet `approval_allowlist` glob — but the agent can now `harness_dump/mount/unmount` |
| **Observability + hardening** | **6/10** | Tracing + per-effect 5s timeout + metrics increments wired; snapshot not on `status --json`, dump --watch not live, global lock still serial, rate limit absent |
| **Testing / CI** | **7.5/10** | 54 harness + 4 harness_agent_integration + 44 plugins + 1711 runtime lib; `harness.yml` runs `dump --live`, `pool-import --apply`, `status | grep harness` — but no `enabled=true` agent-loop job or concurrent-fuzz |
| **Overall implementation quality** | **8.6/10 library, 6.2/10 adopted runtime** | Library is well-crafted and well-tested. Runtime is a verified 3-seam bus with four S-class gaps closed; two more (prompt/hook wiring, WASM bridge) remain before it is a full harness |
| **Upgrade level (post-G1–G12 → S1–S10)** | **+0.6 library (8.0→8.6), +1.2 adoption (5.0→6.2)** | Prior `final audit` claimed +0.5/+3.0; that counted prompt seam as live. This audit counts only the three seams with a production call site |

### What was done well (keep)

- Keeping the kernel a **pure crate** (`operant-harness` depends on no operant crate) — plan 016's most load-bearing constraint — upheld across S1–S10.
- Additive seams behind `register_dyn`/`unregister_tool_if` + identity-aware `ToolSeam` (`Arc::ptr_eq` `tools.rs:436`) — dark-merge genuinely safe; same-name `replace` actually fixed.
- `BuilderWithFactories` as factory seam — minimal, object-safe, no new async trait; `row.config` threaded via `mount_with_config`.
- `ProviderSource::Wasm {path}`/`Pool {name}` round-trip (`provider.rs:25` + `source_roundtrip.rs`) — right lazy shape over unit-only `Source`.
- `Architecture::to_toml` (`composition.rs:112`) deterministic + `Composition::resolve` `replace` preserving `disabled` + lexicographic patch sort — GitOps-correct.
- Per-effect `UNWIND_TIMEOUT_MS` + LIFO + metrics — production-hardening without new dep.

### What should change next (and only next)

C1 (prompt + hook seams + slot feed) — ~1 engineer-day, 3 files, turns the harness from “verified bus” into “evolution engine” without touching 017 or concurrency.

---

## 12. Verdict on the ask

**“Investigate any implementation gaps required for completely adopting this infrastructure for effective scaling and evolution.”**

The harness is **safe to keep on** and **partially safe to build on**:

- **Scaling (250 pools):** Safe to adopt for **tool-centric** pools today: `enabled=true`, `pool-import --apply` for up to `max_active_providers` (64 → bump to 250 with one config change), `architecture dump --live --json | jq .providers` to verify, `status` to monitor rows. The global `RwLock` and read-only stub adapter are not ceilings at boot-rare rates. Add C4 pilot (`relationship-intel` real) before claiming 250.
- **Evolution (prompt/hook without recompile):** **Not yet** — `prompt.section` rows are buildable but their `PromptSections` slot never feeds `SystemPromptBuilder`, and `hook`/`channel.adapter` seams remain test-only. Until C1 the operator must change Rust to evolve persona/policy/autonomy — the harness’s highest-value promise is still a no-op. C1 is the one-line `extend_from_slot` + `HookRunner::register` away.

**Recommendation:** Land C1 next (it is the same size as S1 and keeps `enabled=false` green), then C2 to make `drop valid wasm ≤2s` provable. At that point the rubric moves from 6.2/10 to ~8/10 runtime — the threshold this audit defines as “completely adopted.”

