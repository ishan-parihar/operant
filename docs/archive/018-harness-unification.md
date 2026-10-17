# 018 — Harness Unification: one harness, one store, one config

Stamped against `main` @ c401652e (017-A + 017-B audit). Priority: P0 — fixes the fragmentation that keeps kernel dark.

## 1. The fragmentation (evidence)

Two independent harness modules exist in the same binary, with no shared store:

| | Rust `crates/operant-harness` | Python `kernel-sidecar/harness.py` |
|---|---|---|
| Files | `crates/operant-harness/src/{lib,claim,harness,provider,effect,pool,wasm_provider,host}.rs` (30 files, `Harness`/`Seam`/`Builder`/`Architecture`) | `kernel-sidecar/kernel_sidecar/harness.py` (296 lines, `HarnessService`/`HarnessState` from `vendor/prime-agent`) |
| Store | In-memory `Harness {providers, claims, pending_order, generation}` + `dump()` snapshot, composition via `architecture.toml` + `collect_patches` | File-backed `~/.local/share/operant/kernel/harness/{global,sessions/<slug>/local}` via `rlm.get_harness_state`, ledger + snapshot/rollback |
| Kinds | 6 seams: `tool`, `pool`, `wasm`, `hook`, `prompt.section`, `subagent` (via `Seam` trait, `Provider` `apply`) | 2 kinds: `prompt`, `subagent` only, locked (`HARNESS_KINDS = ("prompt","subagent")`) — comment says “skills/memories stay in operant's own lanes” |
| Config | `[harness] enabled=false` (`crates/operant-core/src/config.rs:705` `HarnessSettings`) | `[tools.kernel] enabled=false` (`config.rs:1390` `KernelSettings`) — **two master switches for one feature** |
| Injection | `harness_dump` tool via `Harness::dump` | `injection_block` (`kernel/mod.rs:94`) via `harness_overview` NDJSON every turn |
| GC | None (in-memory) | `gc_sessions` (`kernel/runtime.rs:211`, 168h) |
| Lifecycle | `Effect` LIFO, `pending_order` on `MissingSeam`, ABA generation | File `store.save()`, `snapshot()`/`_restore_snapshot()`, ledger `refinement-ledger.jsonl` |
| Wiring | `crates/operant-core/src/tools/harness_tools.rs:24` `operant_harness::{Architecture, Harness}` live in `cmd_architecture.rs:243`, `cmd_status.rs`, `main.rs` WASM watcher | `crates/operant-core/src/tools/kernel/harness_tools.rs` wrapper over NDJSON `harness_upsert/overview` |

**The duplication is not perceived — it is structural.** Both implement Cordis's 5 semantics (string claims, PENDING late binding, Effect/LIFO, transactional swap with generation, dump), both expose `prompt`/`subagent`, both are `enabled=false` by default, and neither knows the other exists at runtime. A lesson written to the Rust harness never appears in `injection_block`, and a `prompt` written to the Python harness never appears in `harness_dump`.

**Two configs for one feature** (`[harness]` + `[tools.kernel]`) makes the kernel look like an add-on, which is why it stays dark.

## 2. The single-harness principle

> There is one harness. It has one store. It has one config. Every seam is a module of that store.

The lazy fix is **not** to delete one harness and re-implement its 6 seams in the other language (80% of `operant-harness` is WASM/pool/composition that has no Python equivalent). The fix is to make the Python harness a **persistence seam** of the Rust harness.

```
Before (fragmented):

  [harness] → Rust Harness (in-mem, 6 seams) ──→ dump() ──→ harness_dump tool
                                          └─→ no file persistence

  [tools.kernel] → Python HarnessService (file, 2 kinds) ──→ overview() ──→ injection_block
                                                        └─→ no tool/pool/wasm

After (unified):

  [harness] (single) → Rust Harness (1 store, 6 seams)
                          ├─ memory seams: Tool / Pool / Wasm / Hook / Prompt.section (Rust-native)
                          └─ persistence seam: Prompt+Subagent → delegate to kernel-sidecar/harness.py file store via NDJSON
                                 (the Python process is the file store, not a second harness)

  [tools.kernel].enabled becomes an alias for [harness].enabled for one release, then removed.
  kernel-sidecar/harness.py loses its own HarnessService class and becomes a thin
  `harness_store.py` with only `upsert/get/delete/overview` + file + ledger.
```

**Why this shape:**
- Keeps WASM hot-swap, pool family, and `architecture.toml` where they already work (Rust, no rewrite).
- Keeps `prompt`/`subagent` file persistence, GC, and ledger where they already work (Python, file-backed).
- Injection becomes single: `injection_block` calls `Harness::dump` once (which already includes the persistence seam's `overview`).
- One config: `[harness].enabled` is the only master switch; `[tools.kernel].enabled` becomes `#[serde(alias = "enabled")]` for one release.

## 3. Implementation — three deletion-first phases (net negative, no new feature)

### Phase U1 — Boundary seam (1 PR, ~50 lines, no deletion yet)

**Goal:** make `prompt`/`subagent` a Rust seam that delegates to the sidecar, without deleting the Python harness.

1. New file `crates/operant-harness/src/persistence_seam.rs` (40 lines):
   ```rust
   pub struct PersistenceSeam { rt: Option<Arc<KernelRuntime>> }
   impl Seam for PersistenceSeam {
       fn claim_kind(&self) -> &str { "prompt" } // and "subagent" via second impl or kind param
       async fn apply(&self, claims: &[Claim]) -> Vec<Effect> {
           // for each claim, call rt.request("harness_upsert", {kind,title,content,scope}) 
           // return Effect that calls harness_delete on unwind
       }
   }
   ```
   The seam is constructed with `KernelRuntime::global_runtime()` (already `OnceLock`, `code_execution.rs:351` pattern). When `rt` is `None` (kernel off), `apply` returns `MissingSeam("persistence")` → PENDING, which is the correct fallback (prompt lessons stay pending until kernel is on — no data loss, just delayed).

2. `crates/operant-harness/src/lib.rs:1` — export `persistence_seam`, docs: “This crate owns 6 seams; the persistence seam delegates `prompt`/`subagent` to the sidecar file store.”

3. `crates/operant-harness/src/harness.rs` — mount the seam by default when `Harness::new` is called with `KernelOptions { persistence: true }` (or just always, with PENDING fallback).

No deletion, no config change, just a seam. `cargo check` stays green, existing `prompt`/`subagent` file data is still read via `injection_block`'s direct `harness_overview` — the new seam is additive.

### Phase U2 — Single store, single injection (1 PR, ~80 lines deleted)

**Goal:** injection and GC become single-path.

1. `crates/operant-core/src/tools/kernel/mod.rs:94` `injection_block` — change from two direct `rt.request("harness_overview")` calls (local+global, 5s cache) to `Harness::dump()` single call that already includes the persistence seam's overview. The 5s cache (`runtime.rs:190`) now caches the single dump.

2. Delete the direct `harness_overview` loop in `kernel/mod.rs` (40 lines). Keep `runtime.rs` `cached_injection`/`store_injection` but keyed by `Harness::dump` output instead of sidecar overview.

3. `crates/operant-core/src/tools/kernel/harness_tools.rs` — delete its `harness_upsert` wrapper (20 lines); Rust seam now does it. Keep `kernel_state`/`kernel_refine` only for legacy direct file access until U3.

4. `kernel-sidecar/kernel_sidecar/harness.py` — keep as file store, but delete `HarnessService` class and expose only `upsert/get/delete/overview` functions that `persistence_seam.rs` calls. The Python file is no longer a harness, it's a store.

### Phase U3 — Single config (1 PR, ~20 lines deleted)

**Goal:** one master switch.

1. `crates/operant-core/src/config.rs:705` `HarnessSettings` keeps `enabled: bool`, add `#[serde(alias = "enabled")]` for `kernel.enabled`? Actually make `KernelSettings::enabled` a `#[serde(alias = "enabled")]` that mirrors `HarnessSettings::enabled`, or make `KernelSettings` field `#[serde(alias = "enabled", default)]` that is ignored and `HarnessSettings::enabled` is the single source. The cheapest: add `#[serde(alias = "enabled")] pub enabled_alias: Option<bool>` to `HarnessSettings` that, when `Some`, overrides `harness.enabled`, and keep `KernelSettings::enabled` as `#[deprecated]` alias for one release.

2. `crates/operant-cli/src/config.rs:1181` `KernelConfigV2::enabled` becomes `#[serde(alias = "enabled")]` that writes to the same `AppConfig.harness.enabled` on `From` conversion.

3. `operant.example.toml:306` — collapse `[tools.kernel]` comments to “`enabled` is an alias for `[harness].enabled`; set `[harness].enabled=true`”, or just keep `[tools.kernel]` as alias section for one release.

4. After one release, delete `KernelSettings` entirely and make `kernel-sidecar` a child of `Harness` (no second config).

**Net after U1-U3:** ~130 lines deleted, one harness (`operant-harness`), one store (Python file via seam), one config (`[harness].enabled`), one injection path. The kernel can then be `enabled=true` by default in `operant.example.toml` without duplication.

## 4. What we deliberately do not do

- Do not delete `operant-harness` (see 017-B audit `docs/audit/2026-09-02-017-B-harness-audit.md:34` — 6 seams have no Python equivalent).
- Do not delete `kernel-sidecar/harness.py` file store (Rust has no file persistence for prompt/subagent today; re-implementing it in Rust would be a new feature, not a deletion).
- Do not move WASM/pool to Python — WASM hot-swap is Rust-side for a reason (signature verification, mtime watcher in `main.rs`).

## 5. Verification

- `cargo check -p operant-harness --lib` — still green after U1 (new seam is additive).
- `cargo check --manifest-path operant/Cargo.toml -p operant-core` — green after each phase (U1, U2, U3 separate commits).
- `cargo clippy -p operant-core -- -D warnings` — clean (no new dead_code after seam).
- `cargo test -p operant-harness --lib` — green (existing 22 lib + pending seam test).
- `grep -r "harness_overview" operant/crates/operant-core/src/tools/kernel/mod.rs` — 0 after U2.
- `grep -r "\[tools\.kernel\]" operant/operant.example.toml` — still present as alias in U3, removed in follow-on.
- Live: `operant run -q "create a prompt lesson via harness and verify it appears in injection"` — must appear after U1.

