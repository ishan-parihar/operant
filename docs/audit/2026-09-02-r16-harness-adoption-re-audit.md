# R16 Harness/Plugin Infrastructure — Re-Audit for Complete Adoption

**Date:** 2026-09-02  
**Branch:** `main` @ `f1842bcb` (G1–G12 + post-impl appendix)  
**Baseline:** `64ea7a11` merge of `r16-harness-kernel` onto `4bbc83fd` (016 native)  
**Scope:** Cordis-based harness/plugin infrastructure — can it now **scale the project** (250 hermes pools, 60+ providers, agent self-extension) or is it still a library?  
**Prior audit:** `docs/audit/2026-09-01-r16-harness-upgrade-audit.md` (7.5/10 library, 3/10 runtime on `64ea7a11`, then 9/10 + 9/10 claimed after G1–G12)  
**This audit:** Verifies each G1–G12 closure claim against live code, re-scores Cordis fidelity, and enumerates the remaining gaps for *effective* scaling and evolution.

---

## Methodology

Every claim is checked against a live `file:line` and a runnable probe. "Closed" means a `cargo test` proves the behavior end-to-end (mount → observe → unmount) through the same path the operator uses at boot. "Code exists" without a boot call site is a **library gap**, not a closed gap.

Probes:
- `crates/operant-harness/src/{harness,provider,effect,composition,pool,host,discovery,metrics}.rs`
- `crates/operant-core/src/{harness_adapters,harness_seams_r3,pool_adapter,tools/harness_tools,agent/builders,agent/mod}.rs`
- `crates/operant-cli/src/{main,cmd_architecture}.rs`
- `crates/operant-plugins/src/{watcher,host,signature}.rs`
- `crates/operant-runtime/src/{hooks/harness_seam,agent/prompt_seam}.rs`
- `crates/operant-core/src/config.rs:701` HarnessSettings
- `.github/workflows/harness.yml`
- `plans/016-kernel-native-integration.md`, `plans/017-perfect-unified-kernel.md`, `docs/harness-kernel.md:1`
- `git -C operant diff 64ea7a11..HEAD --stat` (31 files, +2852/-60)

---

## 1. Executive verdict

**The upgrade is 8/10 as a library, 5/10 as an adopted runtime. The prior audit's 9/10 + 9/10 is overstated — 5 of the 12 "closed" gaps are code-exists-without-boot-wiring.**

| Dimension | 64ea7a11 (pre) | Prior audit claim (f1842bcb) | This audit (live) | Why the delta |
|---|---|---|---|---|
| **Library** (kernel + seams + composition) | 7.5/10 | 9/10 | **8.0/10** | Composition and seams are correct, but `config_row kind=prompt.section`, `pool.family` requires, and `wasm` are still stubs in `Builder::build` |
| **Adoption as runtime** (boot → self-extension → hermes) | 3/10 | 9/10 | **5.0/10** | Only `tool` seam is mounted at boot; `harness_mount` tools are never registered; `BuilderWithFactories` is never called from `build_harness_host`; watcher is not wired to `Harness::replace` |

**What genuinely landed:** a dark-merge-safe kernel that an operator *could* use for composable `native` tools; 53 harness tests; a correct same-name `replace` guard; a deterministic composition layer; Ed25519 utils; CLI validate/dump.

**What still blocks scaling:** the operator cannot get a hermes pool or a prompt section or a hook through the harness without hand-wiring a factory; the agent cannot see `harness_dump` without hand-registering it; the watcher cannot hot-swap; only one of six seams is live. The harness is a **single-seam tool bus**, not yet a harness.

---

## 2. Re-evaluation of G1–G12 closure claims

Each prior "CLOSED" is re-graded. Grades: ✅ genuinely closed end-to-end, ⚠️ code exists but not on the boot path, ❌ not closed.

### G1 Wire Harness into agent boot — ⚠️ single-seam boot

**Claim:** `build_harness_host` constructs `HarnessHost` with seams, loads `architecture.toml` + patches, 3 host_boot tests.

**Live:**

* `crates/operant-cli/src/main.rs:1389` `build_harness_host` exists and is called from `build_agent_core:1365`. Correct dark-merge gate (`if !config.harness.enabled { None }`) at `main.rs:1393`.
* It constructs `HarnessHost::new(KernelOptions { audit: true })` at `main.rs:1400` and calls `host.add_seam(Arc::new(ToolSeam::new(registry)))` at `main.rs:1403` — **only `ToolSeam`**. `crates/operant-core/src/harness_seams_r3.rs:505` and `crates/operant-runtime/src/{hooks/harness_seam.rs:129,agent/prompt_seam.rs:132}` define `MemoryProviderSeam`, `ChannelAdapterSeam`, `GatewayCommandSeam`, `HooksSeam`, `PromptSectionSeam`, but `main.rs:1403` never mounts any of them. `HarnessHost::add_seam` at `host.rs:61` requires exclusive `Arc` ownership, so seams cannot be added after `host.harness()` is shared — the one seam must be added before boot, and five are missing.
* `main.rs:1405` loads `architecture.toml` via `resolve_boot_architecture(path, default_patch_dir())` only when `config.harness.architecture_toml.is_some()`. `crates/operant-core/src/config.rs:706` `HarnessSettings::default` sets `architecture_toml: None`, `enabled: false`, `max_active_providers: 64`. An operator who sets `enabled = true` without setting `architecture_toml` gets an **empty kernel** (`main.rs:1429` "kernel is empty"). Plan 016 and `docs/harness-kernel.md:48` promise `architecture_path = "architecture.toml"` default — the implementation chose `Option<PathBuf>` with `None` default, breaking the promised default.
* `main.rs:1421` calls `host.boot(&arch).await` where `host.rs:70` `boot` uses `Builder::build`, not `BuilderWithFactories::build_with`. Even if `architecture.toml` contains `source = "pool"` or `source = "wasm"` rows, they hit the stub path (`composition.rs:389` `NativeRowStub` for wasm, or `BuildError::NoConfigRowHandler` for pool) — they do not materialize.

**Tests:** `crates/operant-harness/tests/host_boot.rs:1` 3 tests use a `CaptureSeam` — they prove `HarnessHost::boot` works when a seam is present, not that `main.rs` wires the five missing seams.

**Grade:** ⚠️ — boot path exists and is dark-merge safe, but only 1 of 6 seams is live and `architecture_toml` has no default.

### G2 ToolSeam identity-aware unregister — ✅ closed

**Claim:** `Effect` captures `Arc` identity, `unregister_tool_if` guards same-name replace.

**Live:** `crates/operant-core/src/tools.rs:432` `ToolRegistry::unregister_tool_if(name, expected)` compares `Arc::ptr_eq` at `tools.rs:436-437`, guarded with `tools::register_dyn` at `harness_adapters.rs:117`. `crates/operant-core/tests/harness_same_name_replace.rs:100` `same_name_replace_preserves_new_tool` mounts v1 then `replace` v2 behind the same provider id and asserts the registry still contains `seam_echo` — genuine end-to-end. `crates/operant-core/src/harness_adapters.rs:130` effect closure captures `installed_arc` and calls `unregister_tool_if`.

**Grade:** ✅ — the prior race (`crates/operant-core/tests/harness_live_loop.rs:293` workaround `echo_v2`) is actually fixed.

### G3 Builder wasm/pool factory dispatch — ⚠️ factory exists, not on boot path

**Claim:** `BuilderWithFactories::register_factory` plugs wasm/pool.

**Live:** `crates/operant-harness/src/composition.rs:284` `ProviderFactory` type alias, `composition.rs:298` `BuilderWithFactories::new`, `composition.rs:307` `register_factory`, `composition.rs:314` `build_with` that dispatches `source == "wasm"`/`"pool"` through factories. `crates/operant-harness/tests/builder_factories.rs:1` 4 tests prove the dispatch. However `crates/operant-cli/src/main.rs:1421` `host.boot(&arch)` never constructs a `BuilderWithFactories`. `host.rs:79` `HarnessHost::boot_with_factories` exists and is tested, but no call site in `crates/operant-cli/src/main.rs` or `crates/operant-core` uses it. The pool e2e at `crates/operant-core/tests/pool_bundle_e2e.rs:48` builds a local `BuilderWithFactories` inside the test — production boot does not.

**Grade:** ⚠️ — the extensibility seam exists, the product boot does not use it. An operator who writes `source = "pool"` in `architecture.toml` gets `NativeRowStub` or an error, not a pool.

### G4 WASM dir watcher + Ed25519 re-verify — ⚠️ watcher exists, not wired to replace

**Claim:** polling watcher + Ed25519 re-verify on every swap, 3 watcher tests.

**Live:** `crates/operant-plugins/src/watcher.rs:54` `Watcher { host: PluginHost, trusted_keys, signature_mode, last_seen }`, `watcher.rs:27` `WatcherConfig { interval: 5s }`, `watcher.rs:96` `scan_once()` iterates `host.plugins_dir()` and calls `enforce_signature_policy` at `watcher.rs:136` on every scan (not just initial load). `watcher.rs:179` `run_until` is the polling loop. `crates/operant-plugins/src/signature.rs:187` `enforce_signature_policy` is real Ed25519 via `canonical_manifest_bytes`.

But:

* `watcher.rs:96` `scan_once` only reads `manifest.toml` freshness (mtime + signature text), not `.wasm` bytes. The kernel swap protocol (`crates/operant-harness/src/swap.rs:35` `SwapGeneration`, `crates/operant-harness/src/harness.rs:405` `Harness::replace`) is never called from `watcher.rs`. `Watcher` detects `ManifestChange` values and returns them — the host must call `Harness::replace(Arc::new(WasmProvider))`. No call site does. `crates/operant-cli/src/main.rs` never constructs a `Watcher`. `crates/operant-plugins/src/lib.rs:1` note still says `no reload_plugin — boot-time only` for Extism.
* The watcher is polling, not `notify` (`watcher.rs:16` comment acknowledges "does NOT pull in notify — a poll loop is adequate"). The plan called for `notify` watcher + `validate → Harness::replace → unwind old` with `SwapGeneration` bump. The bump lives in `Harness::replace` but is never exercised by a real `.wasm` drop.
* `watcher.rs:100` `entries.flatten()` silently skips `read_dir` errors other than `NotFound`; `watcher.rs:125` `read_to_string` errors silently `continue`. Tampered manifests are filtered at `watcher.rs:144` `enforce_signature_policy` → `continue`, but `.wasm` tampering (the actual executable) is not checked.

**Tests:** `crates/operant-plugins/src/watcher.rs:213` 3 tests prove `scan_once` detects new/changed `manifest.toml` and skips unparseable ones. None proves `Harness::replace` or `SwapGeneration` bump.

**Grade:** ⚠️ — manifest discovery and signature re-verify are real, but the watcher is not a hot-swap loop. The kernel's `replace` and the plugins' watcher are two unconnected pieces.

### G5 Close self-extension loop — ❌ tools exist, never registered

**Claim:** real `harness_mount`/`harness_unmount` behind `ToolContext::metadata["approval"] = "true"`.

**Live:** `crates/operant-core/src/tools/harness_tools.rs:147` `HarnessMountTool` parses `ArchitectureRow` from `args.row`, calls `Builder::build` or `BuilderWithFactories::build_with` (when `builders.is_some()`) at `harness_tools.rs:213`, then `harness.mount(provider)` at `harness_tools.rs:241`. `harness_tools.rs:36` `has_approval` checks `context.metadata["approval"] == "true"`. `harness_tools.rs:45` `HarnessDumpTool` is read-only and always available. 4 lib tests at `harness_tools.rs:407` prove deny/succeed via metadata.

But `crates/operant-core/src/tools/harness_tools.rs:335` `register_harness_tools` is a **no-op**:

```rust
pub fn register_harness_tools(registry: &Arc<ToolRegistry>, harness: Arc<Harness>) -> Result<()> {
    let _ = (registry, harness);
    Ok(())
}
```

No call site in `crates/operant-cli/src/main.rs` or `crates/operant-core` calls this, and even if it did, it discards both args. The agent loop at `crates/operant-core/src/agent/mod.rs:281` stores `harness: Option<Arc<Harness>>` and exposes `harness()` getter, but `crates/operant-core/src/agent/run.rs` and `crates/operant-runtime/src/agent/loop_.rs` never consult `self.harness` during tool execution — the comment at `builders.rs:262` says "consults the harness for tool execution after the static registry" but no code does.

**Result:** Even when `[harness].enabled = true`, `operant run --query "mount a tool"` has no `harness_dump`/`harness_mount` in the tool list. The self-extension loop is structurally possible but operationally dead.

**Grade:** ❌ — the tools are tested in isolation but never appear in the agent's tool registry. The prior audit's "agent mounts config row; visible next turn" is false.

### G6 pool.bundle adapter — ⚠️ provider + seam + tool exist, boot does not

**Claim:** `PoolBundleProvider` + `SeamToolPayload` + `PoolBundleTool` end-to-end.

**Live:** `crates/operant-harness/src/pool_provider.rs:22` `PoolBundleProvider { id, config, tool_name, provides_claim: Box<[Claim]> }` where `provides_claim` is `Claim::new("tool", tool_name)` at `pool_provider.rs:37`. `crates/operant-harness/src/pool_provider.rs:74` `activate` calls `build_pool_bundle_tool_for` and `cx.install_with("tool", &tool_name, &tool)` at `pool_provider.rs:80`. `crates/operant-harness/src/provider.rs:151` `SeamToolPayload::tool_name`, `crates/operant-core/src/harness_adapters.rs:70` ToolSeam's second install path (b) handles `Arc<dyn SeamToolPayload>` and third path (c) handles `pool.` prefix fallback via `pool_adapter::build_pool_bundle_tool`. `crates/operant-core/src/pool_adapter.rs:22` `PoolBundleTool { name, path, read_only }` executes at `pool_adapter.rs:77` returning `{ kind: "pool.bundle", name, path, read_only }`. `crates/operant-harness/src/pool.rs:78` `compile` validates `READ_ONLY_VERBS` and emits `family_row` + `bundle_rows`. `crates/operant-core/tests/pool_bundle_e2e.rs:40` proves mount → `registry.get_schemas` → `tool.execute` → `{ read_only: true }` and unmount → empty.

But:

* `PoolBundleProvider::source()` at `pool_provider.rs:52` reads `self.config.get("name")` — but `pool.rs:121` `bundle_rows` config is `{ "path": sub.path, "read_only": true }`, no `name`. So `ProviderSource::Pool { name }` is always `None` for bundle rows (the test at `pool_bundle_e2e.rs:61` asserts `Pool { name: None }`). The `family_row` has `name` but has no provider (no factory for `pool.family`). Bundle identity is via file path, not pool name — `pool_provider.rs:52` G11 note is correct for bundle rows but accidentally true.
* `host.rs:79` `HarnessHost::boot_with_factories` threads `row.config` to `mount_with_config` so the seam can read `path`. `crates/operant-cli/src/main.rs:1421` calls `host.boot` which threads `Value::Null` — bundle `path` is lost. Production boot loses the file path that the tool needs; only the test's manual `mount_with_config(provider, bundle_row.config.clone())` at `pool_bundle_e2e.rs:71` preserves it.
* No `pool.family` provider exists — `compile` emits a family row with `kind="pool.family"` and `config { name, claims, requires }` at `pool.rs:105`, but `BuilderWithFactories::build_with` has no `pool.family` branch. The `services_consumed` → `requires` wire at `pool.rs:99` is compiled but never honored — no provider declares `requires` from the pool's consumed services, so late binding is never exercised for pools.

**Grade:** ⚠️ — the per-bundle read-only tool path is genuine and tested, but production boot does not use it, and the family/requires half of the hermes bridge is not built.

### G7 architecture.toml + patches boot discovery — ⚠️ resolves correctly, default path missing

**Claim:** ordered patch discovery + `architecture.toml.example` committed.

**Live:** `crates/operant-harness/src/discovery.rs:27` `resolve_boot_architecture(base_path, patch_dir)` reads base, `collect_patches` `discovery.rs:53` reads `*.toml` in `patch_dir` sorted lexicographically `discovery.rs:71`, parses each as `Patch` at `discovery.rs:77`, then `Composition::resolve` at `discovery.rs:45`. 3 tests at `discovery.rs:96` cover missing dir, sort, compose. `architecture.toml.example` committed with native, wasm (commented), pool (commented) examples.

But `crates/operant-core/src/config.rs:706` `HarnessSettings::architecture_toml: Option<PathBuf>` defaults to `None`. `crates/operant-cli/src/main.rs:1405` only calls `resolve_boot_architecture` when `config.harness.architecture_toml.is_some()`. An operator who writes `architecture.toml` next to the binary per `docs/harness-kernel.md:44` and sets `enabled = true` without also setting `architecture_toml = "architecture.toml"` gets an empty kernel. The documented contract (`architecture_path = "architecture.toml"` default) is not honored. No `operant architecture init` or auto-create path exists.

**Grade:** ⚠️ — the resolver is correct and tested; the boot call site has an `Option` default that makes the default-composition unreachable.

### G8 tracing spans + metrics + dump --watch — ⚠️ spans yes, metrics/persistence no

**Claim:** `tracing::instrument` on mount/replace/unmount, `HarnessMetrics` counters, `dump --watch N`.

**Live:**

* `crates/operant-harness/src/harness.rs:178` `mount_with_config` `#[tracing::instrument(level = "debug", skip(self, provider, config), fields(id = %provider.spec().id()))]`, `harness.rs:329` `unmount` `#[instrument(level = "info")]`, `harness.rs:404` `replace` `#[instrument(level = "info")]`. Correct — but `main.rs:1400` `HarnessHost::new(KernelOptions { audit: true })` hardcodes `audit: true`; the metric counters at `metrics.rs:14` `HarnessMetrics { mount_success, mount_pending, mount_failed, replace_success, replace_failed, unmount_calls, unwind_invocations }` are `AtomicU64` at `metrics.rs:16`, but `crates/operant-harness/src/harness.rs:117` `metrics: Option<Arc<HarnessMetrics>>` is `None` unless `Harness::with_metrics` is called. No call site in `crates/operant-cli/src/main.rs` ever calls `with_metrics`. Tests at `metrics.rs:63` snapshot locally, never via host.
* `crates/operant-cli/src/cmd_architecture.rs:28` `Dump { file, patch, json, watch_secs: u64 }` and `cmd_architecture.rs:102` `handle_dump_command` loop with `dump_once` at `cmd_architecture.rs:124` that prints `ts` + `file` + `row_count` at `cmd_architecture.rs:141`. But `dump_once` reads **files** (`load_and_resolve` at `cmd_architecture.rs:125`), not the live `Harness::dump()`. An operator who runs `operant architecture dump --watch 2` watches the file tree, not the kernel's `Harness::dump` tree — PENDING state, generation, claims are invisible. The kernel `DumpTree` at `report.rs:29` with `semantics_version` and `claims` is only reachable via `harness_dump` tool (which is not registered per G5).
* No Prometheus `harness_provider_state` gauge; the prior gap called for `prometheus` crate — the implementation chose stdlib atomics with a comment "host can wrap" at `metrics.rs:9`, which is a correct lazy choice but means no `/metrics` endpoint today.

**Grade:** ⚠️ — tracing spans are wired; metrics and file-watch are code-exists-not-wired to the operator's live kernel.

### G9 Effect::unwind timeout — ⚠️ timeout per effect, kernel write lock still serial

**Claim:** `tokio::time::timeout(UNWIND_TIMEOUT_MS, ...)` per effect, host-overridable, 200ms proven.

**Live:** `crates/operant-harness/src/effect.rs:19` `static UNWIND_TIMEOUT_MS: AtomicU64 = 5_000`, `effect.rs:24` `set_unwind_timeout`, `effect.rs:72` `Effect::unwind` wraps `tokio::time::timeout(timeout, undo())` at `effect.rs:78` and logs `tracing::warn!` on `Err(_)` at `effect.rs:82`. `effect.rs:111` `stuck_undo_does_not_block_subsequent_unwinds` sets timeout 200ms, runs `unwind_lifo(vec![fast, stuck])` LIFO (stuck first) at `effect.rs:143`, asserts `elapsed < 2s` and counter == 1.

But the LIFO loop at `effect.rs:103` `unwind_lifo` runs inside `crates/operant-harness/src/harness.rs:184` `mount_locked` (holding `inner.write().await`) and `harness.rs:464` `replace` and `harness.rs:384` `teardown_many_locked`. A stuck undo that times out still held the kernel's single `RwLock<Inner>` at `harness.rs:50` for `UNWIND_TIMEOUT_MS` = 5s. With 60 providers each timing out, one `unmount` could hold the lock for 300s. The audit's "per-seam DashMap sharding deferred" note at `docs/audit/2026-09-01-r16-harness-upgrade-audit.md:199` handled the concurrency side, but the timeout does not fix the global lock — it just bounds each hold to 5s instead of forever.

**Grade:** ⚠️ — timeout half done as claimed; per-seam sharding still deferred. Sufficient to ~60 providers only if none time out.

### G10 Promote live-loop fixtures to main CI — ⚠️ harness.yml runs, not on agent loop

**Claim:** `.github/workflows/harness.yml` runs 8 invocations every push/PR.

**Live:** `.github/workflows/harness.yml:3` `on: push branches: [main], pull_request branches: [main]`, `harness.yml:18` `harness-tests` job with `cargo test -p operant-harness --test host_boot --test builder_factories` at `harness.yml:37`, `harness_same_name_replace`/`pool_bundle_e2e` at `harness.yml:38`, `watcher` at `harness.yml:42`, `harness_tools` at `harness.yml:46`, `effect`/`discovery`/`metrics` at `harness.yml:50-58`, plus `harness-cli` job `harness.yml:60` with `operant architecture dump --json` and `pool-import`.

But the harness tests run with `harness.enabled = false` default config — they are unit/integration tests that construct their own `Harness`, not a test that sets `config.harness.enabled = true` and asserts the agent loop sees the harness. The original gap called for `harness.enabled=true` job to prevent regression in the flag-on path; this CI proves the kernel's library tests, not the agent's adoption. No `cargo test -p operant-core --test agent_loop_with_harness` exists.

**Grade:** ⚠️ — CI now proves the library does not regress; it does not prove the agent does not regress when the harness is on.

### G11 Source payload round-trip — ✅ mostly closed

**Claim:** `ProviderSource::Wasm { path }`, `Pool { name }` round-trips through dump.

**Live:** `crates/operant-harness/src/provider.rs:25` `ProviderSource { Native, Wasm { path: Option<String> }, ConfigRow, Pool { name: Option<String> } }` with `provider.rs:40` `as_str()` preserving wire `source` field, `provider.rs:50` `Display` printing `wasm(path)`/`pool(name)`, `provider.rs:73` `wasm()`/`pool()` unit helpers, `provider.rs:88` `pub type Source = ProviderSource` shim. `crates/operant-harness/src/pool_provider.rs:52` `PoolBundleProvider::source()` reads `config.get("name")` → `Pool { name }`; `crates/operant-harness/src/row.rs:24` `NativeRowStub::new` maps `row.source == "wasm"` → `Wasm { path: None }`. `crates/operant-harness/tests/source_roundtrip.rs:1` `wasm_path_roundtrips_through_dump` + `pool_name_roundtrips_through_dump` mounts via `BuilderWithFactories` then `harness.dump()` asserts `ProviderSource::Wasm { path: Some(...) }` / `Pool { name: Some(...) }`.

Caveat: default-code-path rows (via `Builder::build` or `NativeRowStub`) lose per-row paths — but the per-row payload path is intentionally factory-only. The `ProviderSource::Wasm { path }` vs `ProviderSource::wasm()` dual constructor is the correct lazy shape.

**Grade:** ✅ — the composition-level payload loss the prior audit flagged is fixed for factory-built rows; the unit-vs-payload split is clean.

### G12 pk/r16 crate divergence — ✅ pinned by tests

**Claim:** `Source = ProviderSource` alias pinned, pk worktree keeps compiling.

**Live:** `crates/operant-harness/src/provider.rs:88` `pub type Source = ProviderSource`, `crates/operant-harness/tests/source_alias_compat.rs:1` 3 tests: `source_alias_is_provider_source`, `unit_style_matches_explicit_none`, `display_round_trip`.

**Grade:** ✅ — G11's alias shape is the minimal fix; divergence is pinned.

---

## 3. Cordis 5 semantics — fidelity vs DSH reference

Project memory #15: DSH (DeepSeek harness) is a TS monorepo where everything is a Cordis plugin — services claim `ctx.<key>` slots, deps resolve via `inject (PENDING until provider appears)`, typed events in 5 dispatch modes, reversible `Effect` handles with LIFO unwind, HMR hot-swaps with transactional rollback, `agent-loop` is the only concrete-loop plugin.

| Cordis semantic | DSH form (from #15) | Operant form (016) | Fidelity | Evidence |
|---|---|---|---|---|
| **1. String-keyed claims per typed seam** | `ctx.<key>` | `Claim { seam, key }` (`claim.rs:9`), `Registration { seam, key, payload }` (`provider.rs:121`), `ProviderSpec::{provides,requires}` (`provider.rs:108`) | **9/10** | `harness_adapters.rs:23` ToolSeam downcasts `Arc<dyn OperantTool>` via `payload`, `prompt_seam.rs:41` `Arc<dyn PromptSection>`, `harness_seams_r3.rs:56` `Arc<dyn MemoryProvider>` etc. Correct. |
| **2. Declared requirements + PENDING late binding** | `inject` PENDING until provider appears, rescanned on success | `ProviderState::Pending` (`provider.rs:92`), `Inner::missing_requirements` (`harness.rs:66`), `pending_order: Vec<String>` (`harness.rs:55`), `rescan_pending_locked` after every `mount`/`replace` (`harness.rs:286`) | **9/10** | `crates/operant-harness/tests/kernel.rs:1` T1–T13 cover PENDING → Active, late binding, insertion-order fairness. No operator-visible `requires` yet (no pool.family requires), but the machinery is correct. |
| **3. Reversible effects, LIFO unwind** | `Effect` handles, `HMR` rolls back old | `Effect { label, undo: Option<UndoFn> }` (`effect.rs:40`), `BoxUndoFuture` (`effect.rs:33`), `unwind_lifo` LIFO at `effect.rs:103`, `Harness::teardown_many_locked` seq-desc dependents-first at `harness.rs:371` | **8/10** | `effect.rs:111` stuck-undo test, `harness_adapters.rs:130` identity-aware ToolSeam. G9 timeout added. Still global lock during unwind. |
| **4. Transactional swap, ABA generation** | HMR hot-swaps with transactional rollback, ABA generation counters | `SwapGeneration { generation: AtomicU64 }` (`swap.rs:39`), `SwapOutcome { Committed, Rejected, Failed }` (`swap.rs:68`), `Harness::replace` stage → validate → atomic commit → unwind old → generation bump (`harness.rs:405`) | **7/10** | Kernel-side protocol is correct and has 3 swap unit tests (`swap.rs:80`). Host-side (Extism + watcher → `replace`) is not wired — no real WASM provider has ever been swapped through `Harness::replace` in this repo. |
| **5. Composition as data (config-as-composition)** | `architecture.toml` patches + `operant architecture dump` | `Architecture { rows: Vec<ArchitectureRow> }` (`composition.rs:89`), `Patch { disable, replace, insert }` (`composition.rs:160`), `Composition::resolve` (`composition.rs:185`), `DumpTree { semantics_version, providers, claims }` (`report.rs:29`), `HARNESS_SEMANTICS_VERSION = 1` (`lib.rs:53`) | **6/10** | `composition.rs:430` 12 composition tests + 3 discovery tests, deterministic `DumpTree` sorted by `seq` at `harness.rs:576`. But `config.harness.architecture_toml` is `None` default — composition is never loaded unless the user sets a path. The `Builder::rows_of_kind` helper at `composition.rs:419` is dead code (no call site). |

**Overall Cordis fidelity: 7.8/10** — the kernel's five semantics are individually correct, but the composition-to-swap-to-dump loop has never been exercised end-to-end with a non-trivial provider (e.g. a pool that late-binds on a `pool.family` claim).

---

## 4. Native seams — coverage and adoption

| Seam | Host registry it wraps | Adapter | Payload | Host call site that registers it | Live? |
|---|---|---|---|---|---|
| `tool` | `ToolRegistry::register_dyn` (`crates/operant-core/src/tools.rs:332`) | `ToolSeam` (`harness_adapters.rs:12`) | `Arc<dyn OperantTool>` + `Arc<dyn SeamToolPayload>` + `pool.` fallback `harness_adapters.rs:56` | `crates/operant-cli/src/main.rs:1403` `host.add_seam(ToolSeam)` | **Yes — single live seam** |
| `hook` | `HookRunner` + `DynamicHooks` fan-out (`crates/operant-runtime/src/hooks/dynamic.rs:1`) | `HooksSeam` (`crates/operant-runtime/src/hooks/harness_seam.rs:1`) | `Arc<dyn HookHandler>` | Only in `crates/operant-runtime/src/hooks/harness_seam.rs:129` test | **No** |
| `prompt` | `SystemPromptBuilder` + `PromptSections` slot (`crates/operant-runtime/src/agent/prompt.rs:42`, `prompt.rs:750`) | `PromptSectionSeam` (`crates/operant-runtime/src/agent/prompt_seam.rs:1`) | `Arc<dyn PromptSection>` | Only in `prompt_seam.rs:132` test | **No — `SystemPromptBuilder::with_defaults()` at `crates/operant-runtime/src/agent/agent.rs:1` never calls `extend_from_slot`** |
| `memory.provider` | `MemoryProvider` (`crates/operant-core/src/memory_provider.rs:219`) | `MemoryProviderSeam` (`harness_seams_r3.rs:20`) | `Arc<dyn MemoryProvider>` | Only in `harness_seams_r3.rs:299` test | **No** |
| `gateway.command` | `PluginCommand` global (`crates/operant-core/src/plugins/mod.rs:1`) | `GatewayCommandSeam` (`harness_seams_r3.rs:164`) | `PluginCommand` | Only in `harness_seams_r3.rs:357` test | **No — seam is additive, global registry still authoritative** |
| `channel.adapter` | `Gateway::register_adapter` (`crates/operant-core/src/gateway/mod.rs:563`) | `ChannelAdapterSeam` via `ChannelAdapterHost` trait (`harness_seams_r3.rs:85`) | `Arc<dyn PlatformAdapter>` | Only in `harness_seams_r3.rs:478` `MockHost` test | **No** |

All six seams are **additive, flag-off safe** (dark-merge invariant holds). Only one is mounted in production. The harness is therefore **one seam deep** — it cannot yet evolve prompt, hooks, memory, gateway, or channel topology without a code change.

---

## 5. Composition, WASM, and hermes bridge deep dive

### Composition layer (`crates/operant-harness/src/composition.rs:1`)

* **Good:** `Architecture::from_toml` `composition.rs:105`, `validate` `composition.rs:118`, row `validate` `composition.rs:64` (id + source non-empty, `config_row` requires `kind`), `Patch { disable, replace, insert }` `composition.rs:162` with `#[serde(deny_unknown_fields)]` `composition.rs:163`, `Composition::resolve` patches-apply-in-order `composition.rs:193` with `replace` preserving `disabled` at `composition.rs:231`, `insert` duplicate check at `composition.rs:239`, `from_toml`/`disable`/`replace`/`insert`/`patches_apply_in_order`/`config_row_requires_kind` `composition.rs:430` tests (7).
* **Gaps:**

  1. `Architecture` TOML shape is `[[rows]]` table-array; no `[provider.<id>]` compatibility — the CLI's `dump_once` at `cmd_architecture.rs:124` emits `rows` but the handler at `main.rs:1407` expects `Architecture::from_toml` with the same shape. No golden `architecture.toml` is committed or asserted in CI (the example file exists but `cargo test` never loads it).
  2. `Builder::build` at `composition.rs:381` dispatches `"native"|"wasm"` → `NativeRowStub`, `"config_row"` `kind="disable"` → skip, else `NoConfigRowHandler`. So `config_row kind="prompt.section"`, `pool.family`, `pool.bundle` rows cannot be built without a factory — the operator sees a build error via `operant architecture validate` but boot at `main.rs:1421` `host.boot(&arch)` hits the same error and logs `warn!` then continues with an incomplete kernel.
  3. `BuilderWithFactories::build_with` `composition.rs:314` correctly routes `"pool"`/`"wasm"` through factories and keeps `"config_row"` `kind="disable"` as skip, but `main.rs:1403-1421` never builds a `BuilderWithFactories` — the entire factory half is library-only.

### WASM / swap (`crates/operant-plugins/src/`, `crates/operant-harness/src/swap.rs:35`)

* `SwapGeneration::current`/`bump`/`matches` `swap.rs:49`, `Harness::replace` stage → `stage_activation` `harness.rs:523` → `mount_locked` lifecycle — kernel-side protocol is correct.
* Host-side `PluginHost` `crates/operant-plugins/src/host.rs:12` has `LoadedPlugin`, `host.rs:342` `plugin_info_from_loaded`, `host.rs:391` `validate_skill_bundle`, `host.rs:433` `validate_skill_md_frontmatter`, `runtime.rs:179` `create_plugin` (Extism), `signature.rs:187` `enforce_signature_policy` (Ed25519).
* **Gap:** No `Watcher` → `Harness::replace` bridge. No `WasmProvider` type that wraps an Extism plugin as a `Provider`. No `validate → SwapGeneration bump → unwind old` exercised with real `.wasm` bytes. The watcher at `watcher.rs:96` does not detect `.wasm` mtime, only `manifest.toml` mtime.

### Hermes pool import (`crates/operant-harness/src/pool.rs:32`)

* `PoolManifest { name, services_offered, services_consumed, pooled_sub_systems }` `pool.rs:32`, `READ_ONLY_VERBS` `pool.rs:28`, `compile` `pool.rs:78` → `CompiledPool { family_row, bundle_rows, claims }` `pool.rs:67`, `load_and_compile` `pool.rs:138`. Validates read-only verbs `pool.rs:86`, `operant architecture pool-import` at `cmd_architecture.rs:70` works.
* **Gaps:**

  1. `services_consumed` → `requires` is compiled into `family_row.config.requires` at `pool.rs:99` but never interpreted as `ProviderSpec::requires()` by any provider — no late-binding for pools.
  2. `pooled_sub_systems` → `pool.bundle` rows at `pool.rs:118` with `config { path, read_only }`; the `PoolBundleProvider` at `pool_provider.rs:22` materializes one `tool` claim per bundle, but the CLI's `pool-import` is `compile-only` — there is no `operant architecture pool-import --apply` that writes rows into `architecture.toml` or `architecture.patch.toml`.
  3. No pilot for `relationship-intel` or `research-engine` in `~/.hermes/systems/` — the only fixture is synthetic `relationship-intel` at `docs/audit/2026-08-30-r16-harness-kernel-audit.md:1` (4 pool tests use inline YAML). No 250-pool scalability test.

---

## 6. CLI, model tools, and docs

* **CLI** `crates/operant-cli/src/cmd_architecture.rs:15` `Validate` / `Dump { watch_secs }` / `PoolImport` — all work, but `Dump` reads files (`load_and_resolve` at `cmd_architecture.rs:184`) not `Harness::dump()`. An operator debugging a stuck provider sees file rows, not kernel state.
* **Model tools** `crates/operant-core/src/tools/harness_tools.rs:335` `register_harness_tools` no-op — see G5. Even `HarnessDumpTool::execute` at `harness_tools.rs:87` is `format!("{:?}", p.state)` (Debug) rather than serde serialization — `DumpTree::to_json` at `report.rs:37` is never called from the tool.
* **Docs** `docs/harness-kernel.md:1` (operator companion) + `docs/audit/2026-08-30-r16-harness-kernel-audit.md:1` (phase-by-phase) are present and accurate for the library half, but not linked from `README.md` or `docs/README.md`, and `docs/harness-kernel.md:120` composition kinds table is stale (still marks `pool.bundle`/`wasm` as `Builder rejects` — after G6 they are factory-only, but not on boot).

---

## 7. Observability, hardening, and concurrency — scaling gaps

| Area | Current state | Gap for 60–250 providers | Impact |
|---|---|---|---|
| **Observability** | `DumpTree`/`ProviderEntryInfo`/`ClaimInfo` `report.rs:8`, `HarnessError` `error.rs:6`, `tracing::instrument` on 3 lifecycle methods `harness.rs:178` | No `metrics: Option<Arc<HarnessMetrics>>` wired in `main.rs:1400`; no `HarnessMetrics` → Prometheus gauge; `operant architecture dump --watch` watches files not kernel; no `operant status` harness line | Cannot alert on PENDING churn or swap failures at scale; debug requires tool that is not registered |
| **Hardening** | `Effect { label, undo: Option<UndoFn> }` LIFO, transactional `replace`, per-effect `UNWIND_TIMEOUT_MS=5s` `effect.rs:19` | Timeout per effect but kernel `RwLock<Inner>` held across whole `teardown_many_locked` at `harness.rs:370` — one bad provider can block mount for N×5s; no `Effect` idempotency beyond `Option::take`; no Harness-level rate limiting | Bad `unregister_tool` can DOS mount path for minutes with 60 providers |
| **Concurrency** | Single `RwLock<Inner>` `harness.rs:50`, `seams: HashMap<String, Arc<dyn Seam>>` `harness.rs:113` (not behind RwLock — seams added only at boot) | Global lock — `mount`/`unmount`/`replace` serialize all providers; `dump` read lock blocks on write; no per-seam sharding or `DashMap` | Throughput ceiling at ~100 providers; hermes has 250 pools |
| **Security** | `READ_ONLY_VERBS` allowlist `pool.rs:28`, `Source` tracking `provider.rs:25`, `SignatureMode` + `enforce_signature_policy` `signature.rs:187`, `has_approval` `harness_tools.rs:36` | No per-provider `capabilities` (Tool vs Channel vs Observer vs Skill, Ed25519-signed manifests, boot-time load only); no allowlist for `hook.*` or `memory.provider`; no Ed25519 verification on `replace` (only on watcher `scan_once`); `has_approval` checks `ToolContext.metadata["approval"]` string compare — no integration with `operant-config` approval allowlist (`AgentConfig.approval_allowlist` `agent/mod.rs:91`) | WASM hot-swap can escalate from tool seam to memory seam |
| **Config** | `architecture.toml` + `Patch` `composition.rs:160`, `discovery.rs:27` + `architecture.toml.example` | No `config watch` — operator edits `architecture.toml` and must restart; `default_patch_dir()` `discovery.rs:88` is `$HOME/.operant/patches` but no `operant architecture patches list` CLI; `HarnessSettings` default is `None` so default composition unreachable | Breaks GitOps at scale; operator must know to set `architecture_toml` path |
| **Testing** | 53 harness lib+integration, 1770 core lib (filtered), 3 watcher, harness-cli smoke, `harness.yml` | No `harness.enabled=true` agent-loop integration test; no 30-turn soak; `cargo test --workspace` still not gated (hangs, per `plans/016:109`); no concurrent mount fuzz beyond `T8_concurrent_mounts_all_land` | Regressions in flag-on path not caught before merge |

---

## 8. Plan 017 and the dual-kernel divergence

`plans/017-perfect-unified-kernel.md:1` (stamped `4bbc83fd`, P0) identifies five duplicate spots that G1–G12 did not address:

* **D1 Python execution:** `crates/operant-core/src/tools/code_execution.rs:148` stateless `execute_python()` vs `crates/operant-core/src/tools/kernel/kernel_tool.rs:14` persistent `kernel_exec` NDJSON to `kernel-sidecar`. D1 persists — `route_python_to_kernel` flag still gates, `code_execution` still spawns `python -c`. The harness does not own Python execution.
* **D2 Harness/state:** `crates/operant-harness/src/harness.rs:14` pure Rust Cordis vs `kernel-sidecar/kernel_sidecar/harness.py:1` `HarnessService` (live) vs `vendor/prime-agent/prime-agent-runtime/src/rlm` upstream. G1–G12 added the pure crate but did not touch `kernel-sidecar`; three homes for "learned state" remain.
* **D3 Vendor path:** `vendor/prime-agent` submodule + `kernel_sidecar/vendor.py:19` + `operant-harness` Rust — two implementations plus upstream. No consolidation.
* **D4 Provider runtime:** `ToolRegistry` (`crates/operant-core/src/tools.rs:1`) vs `Harness` (`crates/operant-harness/src/lib.rs:14`) — now bridged for `tool` seam only, but `global_runtime()` `OnceLock<Arc<KernelRuntime>>` at `crates/operant-core/src/tools/kernel/mod.rs:94` is a separate path. The harness is not the `ToolRegistry`; it wraps one method.
* **D5 Skills:** `crates/operant-runtime/src/skills/mod.rs:1` prose `SkillManifest` vs `crates/operant-core/src/tools/kernel/mod.rs:85` `SkillReference { type, import, callable, call_pattern }` — two scans of `skills_dir/*/SKILL.toml`. G1–G12 did not touch this.

**Result:** The kernel still cannot be `enabled=true` by default without running the same feature twice. 017's "one Python path, one harness, one vendor, one skill scan" is the P0 prerequisite for the harness to be the evolution engine. G1–G12 made the Cordis kernel provably correct; 017 makes it the *only* kernel.

---

## 9. Updated gap matrix for complete adoption

Each gap is S (<1 day), M (1–3 days), L (>3 days). "Blocks scaling" = operator cannot get the scaling benefit without this.

| # | Gap | Blocks scaling? | Effort | Where |
|---|---|---|---|---|
| **S1** | **Register all six seams at boot and wire `Builder` through `BuilderWithFactories`.** Change `crates/operant-cli/src/main.rs:1403-1421` to construct `ToolSeam` + `HooksSeam(DynamicHooks)` + `PromptSectionSeam(PromptSections)` + `MemoryProviderSeam` + `GatewayCommandSeam` + `ChannelAdapterSeam`, add them before `host.boot`, and replace `host.boot` with `host.boot_with_factories` once factories for `wasm`/`pool` are registered. Until S1, only `tool` scales. | **Yes — 5/6 capability families are dead** | S | `operant-cli` + `operant-runtime` |
| **S2** | **Make harness tools visible to the agent.** Replace `crates/operant-core/src/tools/harness_tools.rs:335` no-op `register_harness_tools` with real registration (`registry.register_arc(Arc::new(HarnessDumpTool::new(harness)))` etc.) and call it from `crates/operant-cli/src/main.rs:1365` `build_agent_core` when `harness.is_some()`. Gate `harness_mount`/`harness_unmount` behind an opt-in `capabilities: ["harness.mount"]` flag rather than `ToolContext.metadata["approval"]` string compare. | **Yes — self-extension is invisible** | S | `operant-core` + `operant-cli` |
| **S3** | **Give `architecture.toml` a default path.** Change `crates/operant-core/src/config.rs:706` `architecture_toml: Option<PathBuf>` to default `Some(PathBuf::from("architecture.toml"))` or resolve relative to `~/.operant/` when `enabled`, and make `crates/operant-cli/src/main.rs:1405` create the file from `architecture.toml.example` on first enable. Without S3 the boot is empty even when enabled. | **Yes — default-enabled is empty** | S | `operant-config` + `operant-cli` |
| **S4** | **Wire the WASM watcher to `Harness::replace`.** Make `crates/operant-plugins/src/watcher.rs:96` detect `.wasm` mtime/signature change (not just `manifest.toml`), construct a `WasmProvider { id, path, claims }`, call `host.harness().replace(provider)` with Ed25519 re-verify per `swap.rs:1` protocol, and bump `SwapGeneration`. Spawn the watcher from `crates/operant-cli/src/main.rs:1400` when `[harness].enabled`. Until S4, the watcher is a directory scanner and HMR is unused. | **Yes — P4 is kernel-only** | M | `operant-plugins` + `operant-cli` |
| **S5** | **Materialize `pool.family` and late-bind on `services_consumed`.** Teach `crates/operant-harness/src/pool.rs:78` `compile` output to drive a `FamilyProvider` that `spec().provides() = claims` and `spec().requires() = requires`, so `Harness::rescan_pending_locked` `harness.rs:286` actually rescues dependents when a family mounts. Without S5, hermes pools are `pool.bundle` tools without dependencies. | Yes | M | `operant-harness` |
| **S6** | **Wire metrics and live dump.** Call `Harness::with_metrics(Arc::new(HarnessMetrics::new()))` in `crates/operant-cli/src/main.rs:1400`, expose `HarnessMetrics::snapshot()` on `operant status` and on a `GET /metrics` gauge (`harness_provider_state{state="active|pending|failed"}`), and make `operant architecture dump --live` call `harness.dump()` via a CLI-adjacent harness handle instead of `load_and_resolve`. | No, but required for on-call | S | `operant-harness` + `operant-cli` |
| **S7** | **Unblock `config_row kind=prompt.section` (and generic config_row→seam routing).** Add a `ConfigRowHandlerRegistry` so `BuilderWithFactories` can route `config_row` kinds to seams without a factory per kind; wire `prompt.section` to `PromptSectionSeam`, make `crates/operant-harness/src/row.rs:71` `ConfigRowProvider` install real sections (file-backed) instead of logging. Until S7 the most common patch use-case (nudge a prompt section) is a `NoConfigRowHandler` error. | Yes for prompt evolution | S | `operant-harness` + `operant-runtime` |
| **S8** | **Concurrency: per-seam sharding or `DashMap` for `Inner.entries`.** Replace `crates/operant-harness/src/harness.rs:50` single `RwLock<Inner>` with per-seam `DashMap` or `RwLock<HashMap<Claim, ...>>` so `mount`/`unmount`/`replace` only contend on their seam family. Keep LIFO unwind but bound each effect to 5s (G9) and each seam to its own lock. | No until 60+ providers, but required at 250 pools | M | `operant-harness` |
| **S9** | **Resolve 017 dual-kernel divergence.** Either collapse `code_execution` python arm into `global_runtime().request("exec")` (017 Phase A) and unify `ToolRegistry`/`Harness` (Phase B), or explicitly mark `operant-harness` as "tool bus only" and defer 017. Without a decision, operators see two harnesses and `enabled=true` means duplication. | **Yes — blocks default-on** | L | `operant-core` + `kernel-sidecar` |
| **S10** | **30-turn soak + harness.enabled=true CI job.** Promote a `cargo test -p operant-core --test agent_loop_with_harness` that runs the agent with `HarnessSettings { enabled: true, architecture_toml: Some(tempfile) }`, mounts a pool, observes via `harness_dump`, keeps or rolls back. Add it to `.github/workflows/harness.yml:18` as a `harness-e2e` job. | No, but required to prevent regression | S | `operant` |

---

## 10. Recommended next PR queue (after `f1842bcb`)

Ordered to keep `[harness].enabled=false` green and each PR gating on `cargo check -p operant-harness -p operant-core -p operant-runtime --all-features` + `cargo test -p operant-harness --test kernel` still 13.

| Priority | Title | File:line | Why now |
|---|---|---|---|
| 1 | `fix(harness): wire 6 seams + BuilderWithFactories on boot (S1+S2)` | `crates/operant-cli/src/main.rs:1403`, `crates/operant-core/src/tools/harness_tools.rs:335` | Turns the harness from a tool bus into a harness; unblocks prompt/hook/memory evolution with zero new concepts |
| 2 | `fix(harness): default architecture.toml and empty-kernel warning (S3)` | `crates/operant-core/src/config.rs:701`, `crates/operant-cli/src/main.rs:1405` | Without S3 the default-enabled operator still gets zero providers |
| 3 | `feat(harness): config_row prompt.section via handler registry (S7)` | `crates/operant-harness/src/composition.rs:381`, `crates/operant-harness/src/row.rs:71` | The smallest value-creating topology change; proves the seam |
| 4 | `feat(plugins): watcher → Harness::replace with WASM mtime + Ed25519 (S4)` | `crates/operant-plugins/src/watcher.rs:96`, `crates/operant-harness/src/swap.rs:35` | Then `drop valid wasm ≤ 2s; corrupt ⇒ old serves` is finally testable |
| 5 | `feat(pool): family requires + pool-import --apply (S5)` | `crates/operant-harness/src/pool.rs:78`, `crates/operant-cli/src/cmd_architecture.rs:70` | Then `relationship-intel` actually late-binds |
| 6 | `feat(harness): wire HarnessMetrics + operant status + dump --live (S6)` | `crates/operant-harness/src/metrics.rs:14`, `crates/operant-cli/src/cmd_architecture.rs:28` | Then on-call can alert on PENDING churn |

Each is S or M, keeps `enabled=false` byte-stable, and can land as a single `cargo check` gate. S8 and S9 are post-scale and post-017 respectively.

---

## 11. Scoring — how well was the upgrade done

| Dimension | Score | Rationale |
|---|---|---|
| **Cordis semantics fidelity** | 7.8/10 | Strings, PENDING, LIFO, ABA, composition are all implemented correctly in isolation; they have never been exercised together through a real pool/WASM provider. |
| **Seam correctness (each seam)** | 9/10 each | Six seams are individually correct, additive, and dark-merge safe. The best part of the upgrade. |
| **Seam adoption (all seams live)** | 3/10 | Only 1/6 seams mounted at boot. The harness title promises a harness, the boot delivers a tool bus. |
| **Composition as data** | 7/10 | Deterministic, validated, patched, sorted — but no default path, and the CLI's `dump` is file-based not kernel-based. |
| **WASM / hot-swap** | 5/10 | Kernel protocol and Ed25519 utils are real; no `.wasm` has ever been hot-swapped through `Harness::replace`. |
| **Hermes bridge** | 6/10 | Compiler and per-bundle tool are real and tested; family/requires, `--apply`, and pilots are missing. |
| **Self-extension loop** | 2/10 | Tools are implemented and approval-gated, but never registered — the agent cannot call them. |
| **Observability + hardening** | 5/10 | Tracing spans and per-effect timeout are real; metrics not wired, dump not live, concurrency still global lock. |
| **Testing / CI** | 7/10 | 53 harness tests + watcher + e2e, `harness.yml` on every push — but no `enabled=true` agent-loop job or soak test. |
| **Overall implementation quality** | **8.0/10 as a library, 5.0/10 as an adopted runtime** | The library is well-crafted and well-tested. The runtime is single-seam and the operator cannot reach it through the normal config surface without hand-wiring. |
| **Upgrade level (post-G1–G12 vs pre)** | **Genuine +2 on library, +2 on adoption** (7.5→8.0 / 3→5). Prior appendix claimed +1.5 on library (7.5→9) and +6 on adoption (3→9); those numbers counted code-exists as closed. |

### What was done well (keep)

* Keeping the kernel a **pure crate** (`operant-harness` depends on no operant crate) — plan 016's most load-bearing constraint — was upheld across all 12 commits.
* Additive seams behind `register_dyn`/`unregister_tool_if` — dark-merge is genuinely safe, live-loop byte-identical.
* `BuilderWithFactories` as the factory seam — minimal, object-safe, no new async trait.
* Identity-aware `ToolSeam` (`Arc::ptr_eq` via `unregister_tool_if`) — the prior race is actually fixed with a one-line guard.
* `ProviderSource::Wasm { path }` / `Pool { name }` — the right lazy shape over the prior unit-only `Source`.

### What should change next (and only next)

S1+S2+S3 (above) — ~2 engineer-days, 3 files, turns the harness from "correct library" into "usable runtime" without touching plan 017 or concurrency.

---

## 12. Verdict on the ask

**"Investigate any implementation gaps required for *completely* adopting this infrastructure for effective scaling and evolution of this project."**

The harness is **safe to keep on** and **not yet safe to build on** for evolution:

* **Scaling enact:** Enabling the harness today scales `tool` only. A team that needs to evolve prompt, hooks, memory, gateway, or channel topology still needs a code change — the harness does not cover that yet. After S1+S3 the harness covers file-based topology changes with no recompile.
* **Self-evolution:** The agent's self-extension story (`plugin-authoring` skill → `harness_mount` → observe → keep/rollback) is documented but unplugged — no tool call will appear in the transcript. After S2 it is testable end-to-end.
* **Hermes bridge at 250 pools:** The compiler scales (pure function on YAML), but the mount path does not — `services_consumed` dependencies never late-bind and `pool-import` never writes an architecture. After S5 the hermes pools behave like Cordis plugins with `requires` and `provides`.
* **Hot-swap at 60+ WASM providers:** The kernel protocol scales to the plan's 60, but the host side cannot hot-swap and the single `RwLock` serializes everything. After S4+S8 the WASM dir is the HMR source and mounts contend per seam.

Complete adoption is **three S/M PRs away** (S1+S2+S3), with two more (S4+S5) for hermes and WASM to be operator-visible.

