# 017 — Perfect Unified Kernel: eliminate duplicate architectures, make kernel the single source of truth

Stamped against `main` @ 4bbc83fd (016 native). Priority: P0 (enables kernel-on-by-default without duplication).

## 1. Why kernel is off — and why that looks like duplication

`016` shipped kernel as **dark-native**: code is first-class (`crates/operant-core/src/tools/kernel/*`, `kernel-sidecar/*`, `[tools.kernel]` in `operant.example.toml:306`), but `enabled=false` by default. This is intentional for 016: no existing user breaks, no required `python3.11`/`uv`/`vendor/prime-agent` on every host, and `cargo check` stays green even without the submodule. The cost is perception: two ways to do the same thing appear to coexist, which is the duplication you flagged.

**The five duplicate spots (evidence, not speculation):**

### D1 — Python execution: stateless vs persistent
- `crates/operant-core/src/tools/code_execution.rs:148` `execute_python()` — spawns a fresh `python -c` per call, `kill_on_drop`, `MAX_STDOUT_BYTES 50k`, stateless.
- `crates/operant-core/src/tools/kernel/kernel_tool.rs:14` `kernel_exec` — NDJSON to `kernel-sidecar`, namespace persists across turns, `route_python_to_kernel` gates it (`code_execution.rs:132` `if kernel_cfg.route_python_to_kernel { /* route */ }`).
- **Same feature, two runtimes.** Today the tool description even advertises both: `code_execution` lists `python` as a language and `kernel_exec` lists `python` as a persistent alternative.

### D2 — Harness/state: pure vs live vs vendor
- `crates/operant-harness/src/harness.rs:14` `Harness` — pure Rust Cordis (string-keyed claims, PENDING, Effect LIFO, ABA). Dark-merged, `cargo test -p operant-harness` green, **zero consumers** (`grep -r operant-harness` only shows `Cargo.toml` and `config.rs` comments).
- `kernel-sidecar/kernel_sidecar/harness.py:1` `HarnessService` — live Python that **is** used, via `kernel-sidecar` NDJSON `harness_upsert/overview` (`crates/operant-core/src/tools/kernel/mod.rs:94` `injection_block` calls it every turn).
- `vendor/prime-agent/prime-agent-runtime/src/rlm` — upstream source that `harness.py` vendors via `kernel_sidecar/vendor.py:19` `VENDOR_RUNTIME_SRC`.
- **Same "learned state" (prompt/subagent), three homes.** The pure crate is aspirational, the sidecar is operational, the vendor is upstream. None of the three knows the other exists at runtime.

### D3 — Vendor path duplication
- `vendor/prime-agent` (git submodule, `.gitmodules:1`, ~250MB) + `kernel-sidecar/kernel_sidecar/vendor.py:19` shim + `operant-harness` (which re-implements the same five Cordis semantics in Rust). Two implementations of the same state machine (Rust + Python) plus the upstream it was copied from.

### D4 — Provider runtime: Harness vs ToolRegistry
- `crates/operant-core/src/tools.rs:1` `ToolRegistry`, `crates/operant-core/src/agent/run.rs:1484` `injection_block` as a free function — the live registry.
- `crates/operant-harness/src/lib.rs:14` `Harness::mount(provider)` — a second registry with the *same* five semantics (string claims, PENDING, Effect, HMR, dump) but for abstract `Provider`, not `OperantTool`. `operant-harness` is dark, `ToolRegistry` is live, and `mod.rs:94` `global_runtime()` is a separate `OnceLock<Arc<KernelRuntime>>` rather than a `Harness` instance.

### D5 — Skills: prose vs executable
- `crates/operant-runtime/src/skills/mod.rs:1` `SkillManifest` (prose, `#[forbid(unsafe_code)]`, `audit.rs` etc.) — installed skills.
- `crates/operant-core/src/tools/kernel/mod.rs:85` `SkillReference {type, import, callable, call_pattern}` + `crates/operant-core/src/config.rs:1416` `KernelPyskill {enabled, allowed_imports}` — executable binding that the first system does not know about. `executable_skills_section()` scans `skills_dir/*/SKILL.toml` a second time, parallel to the runtime's own scan.

**Result:** the kernel cannot be turned on by default because turning it on would mean *running the same feature twice* (two pythons, two harnesses, two skill scanners). The flag stays off to hide the seam, which is why "we tried so hard but it's still off."

## 2. Perfect unified architecture (single source of truth)

```
Before (016 dark-native):                  After (017 unified):
┌─────────────────┐  ┌──────────────┐      ┌─────────────────────────────────┐
│ code_execution  │  │ kernel_exec  │      │ Execution (single)              │
│  (stateless py) │  │ (persistent) │  →   │  kernel is the python path;     │
│                 │  │  gated       │      │  code_execution python arm      │
└─────────────────┘  └──────────────┘      │  becomes a thin wrapper:        │
                                           │  `execute_python()` =           │
┌─────────────────┐  ┌──────────────┐      │  `global_runtime().request(     │
│ operant-harness │  │ harness.py   │      │    "exec", {code, session})`   │
│  (pure, dark)   │  │ (live, vend) │  →   │  Stateless path kept only as    │
│                 │  │  + rlm       │      │  fallback when kernel disabled. │
└─────────────────┘  └──────────────┘      └─────────────────────────────────┘

┌──────────────┐  ┌──────────────┐         ┌─────────────────────────────────┐
│ ToolRegistry │  │ Harness      │         │ Registry (single)               │
│  (live)      │  │ (Cordis)     │  →      │  ToolRegistry **is** the        │
│              │  │  dark        │         │  Harness backend:               │
└──────────────┘  └──────────────┘         │  `ToolRegistry::register`       │
                                           │  returns `Effect`, `Harness`    │
                                           │  wraps it. No second registry.  │
                                           └─────────────────────────────────┘

vendor/prime-agent  →  single vendor root, Rust harness deleted or made a
                       thin re-export of the Python rlm types via pyo3/maturin;
                       `vendor.py` shim stays, `operant-harness` either wraps
                       `kernel-sidecar` or is deleted (decision below).

Skills: single scan — `operant-runtime` owns `SKILL.toml` parse, `kernel`
        consumes the already-parsed `SkillManifest.reference` instead of
        re-scanning `skills_dir`.
```

**Invariants after 017:**
- One Python path. `code_execution` with `language=python` and `kernel_exec` are the same NDJSON call; the only branch is `if !enabled { fallback to stateless }`.
- One harness. `harness_overview` is the single `dump()` shape; `operant-harness` (if kept) is a Rust view over the sidecar's store, not a second store.
- One vendor. `vendor/prime-agent` stays on disk, but only one importer (`vendor.py`) touches it.
- One skill scan. No double `read_dir` of `skills_dir`.
- Kernel can be `enabled=true` by default *after* 017, because enabling it no longer means duplication.

## 3. Plan 017 — three deletion-first phases (no new features, only unification)

### Phase A — Collapse Python execution (1 PR, ~80 lines deleted)

**Goal:** `code_execution` python arm disappears as a separate runtime.

1. `crates/operant-core/src/tools/code_execution.rs:132` — change `execute_python()` from `spawn python -c` to:
   ```rust
   if let Some(rt) = kernel::global_runtime().filter(|r| r.settings().enabled) {
       return rt.request("exec", json!({"code": code, "timeout": timeout})).await;
   }
   // fallback: current stateless spawn (kept, but only for enabled=false)
   spawn_python_subprocess(code, timeout).await
   ```
   The `route_python_to_kernel` flag is deleted; `enabled` is the only gate.

2. Delete `execute_python`/`execute_javascript` split for python only — keep `execute_javascript`/`execute_shell` as before (they have no kernel equivalent).

3. Update `code_execution` tool description from "Execute code in python..." to "Execute code (python via persistent kernel when enabled, else stateless)..." and `MAX_STDOUT_BYTES` handling becomes the sidecar's `max_output_bytes` (200k) when routed, 50k fallback otherwise — document the two caps.

4. Tests: existing `test_code_execution_python_happy_path` now asserts persistence (`x=1; x+=1` across two calls) when kernel enabled, stateless when disabled.

**Net:** `code_execution.rs` loses ~40 lines of python spawn; `kernel_tool.rs` loses its separate `kernel_exec` wrapper only if we keep the `code_execution` name (decision: keep `kernel_exec` as an alias for one release, then deprecate — cheapest path is to keep both names pointing at the same `global_runtime().request("exec")` for now, delete `kernel_exec` in 018).

### Phase B — Unify harness (1 PR, ~300 lines deleted or moved)

**Goal:** one store, one injection, one GC.

1. **Delete or thin `crates/operant-harness`** — two options, pick A (cheaper):
   - **A (delete):** remove `crates/operant-harness` from `Cargo.toml` members, delete `crates/operant-harness/*`. The Cordis semantics stay proven by the sidecar's `harness.py` + `tool_bridge.rs` (which already implements allowlist + per-call timeout). The pure crate was Phase 0 scaffolding; its tests (`dark_merge_scaffold_is_sound`) are superseded by `kernel-sidecar/test/test_kernel.py` (13 passed).
   - **B (keep as view):** make `operant-harness` a *read-only* Rust projection over the sidecar store (pyo3 call to `harness_overview`), no second `Harness::mount`. Not recommended until WASM hosts need it — YAGNI, so delete.

2. **Single harness module:** move `crates/operant-core/src/tools/kernel/harness_tools.rs` (Rust `harness_upsert/overview` wrappers) to `crates/operant-core/src/harness.rs` and make it the *only* harness surface. `kernel/mod.rs:94` `injection_block` stays but now calls `crate::harness::overview()` instead of `rt.request("harness_overview")` directly (same NDJSON underneath, but one call site).

3. **Registry wrapping (deferred to 017-B2 if needed):** if we keep any Rust `Harness` concept, make it a newtype around `ToolRegistry`: `Harness(ToolRegistry)` where `Harness::mount` = `ToolRegistry::register` returning `Effect`. Do not introduce a second `HashMap` of claims — reuse `ToolRegistry`'s map. This is a 30-line wrapper, not a second registry. If even this is too much, skip — `ToolRegistry` already is the harness.

4. **Vendor single import:** keep `vendor/prime-agent` path on disk (see 016 decision: renaming breaks `git submodule update`), but delete the Rust re-implementation. `kernel_sidecar/vendor.py:19` remains the single importer; `operant-harness`'s Rust copy is gone.

**Net:** `crates/operant-harness` deleted (~400 lines), `harness_tools.rs` moved, one injection call site, GC (`runtime.rs:211` `gc_sessions`) now lives in `crates/operant-core/src/harness.rs` next to overview.

### Phase C — Unify skills scan and flip default (1 PR, ~60 lines deleted)

**Goal:** one parse, kernel on by default.

1. **Single scan:** delete `crates/operant-core/src/tools/kernel/mod.rs:88` `executable_skills_section()`'s own `read_dir(skills_dir)`. Instead, add `fn skill_references() -> Vec<SkillReference>` to `operant-runtime` that returns already-parsed `SkillManifest.reference` for manifests where `reference.is_some()` and file exists. Kernel calls that. No second dir walk, no second TOML parse. `is_import_allowed` (`mod.rs:101`) stays in kernel (policy), but the data comes from runtime.

2. **Config flip:** `crates/operant-core/src/config.rs:1420` `KernelSettings {enabled: false}` → `true` once A+B are done and `cargo check` + `pytest` are green. Keep `pyskill.enabled=false` until allowlist is populated — executable skills stay opt-in even when kernel is on.

3. **Tool list:** `crates/operant-core/src/tools/builtin.rs:308` `working_diff` etc. already list kernel tools via `toolset: "kernel"`; with kernel on, `operant tools list` goes 79 → 82 (adds `kernel_exec`, `kernel_state`, `kernel_refine`) without duplication because `kernel_exec` is now the python path.

4. **Docs:** `docs/kernel.md:14` and `operant.example.toml:306` change from `# enabled = false # set true after submodule` to `enabled = true # requires python3.11 + submodule (auto-falls back to stateless)`.

**Net:** one skill parse, kernel on by default, `route_python_to_kernel` deleted, `operant-harness` deleted, net **~400 lines removed**, no new runtime dep.

## 4. What we deliberately do not do

- Do not keep `operant-harness` as a second store "for later WASM" — YAGNI, delete and re-add when WASM needs it; the current pure crate has zero consumers.
- Do not rename `vendor/prime-agent` on disk — cost > benefit, hide in docs as 016 did.
- Do not delete `kernel_exec` tool name in 017 — keep as alias to `code_execution` python for one release, deprecate in 018. Immediate deletion would break any trajectory that already uses `kernel_exec`.
- Do not auto-author `[reference]` in background review in 017 — still 018.

## 5. Verification (same as 016, plus deletion proof)

- `cargo check --manifest-path operant/Cargo.toml` — still `Finished dev` after each phase (A, B, C separate commits; `cargo check` is the gate, not `cargo build` which needs `libsonic`).
- `cargo clippy -p operant-core -- -D warnings` — clean (no new `collapsible_if` from the wrapper).
- `uv run pytest operant/kernel-sidecar/test -v` — 13 passed (harness test still needs submodule, still fails there, expected).
- `grep -r operant-harness operant --include="*.rs"` — 0 hits after B.
- `grep -r "route_python_to_kernel" operant` — 0 hits after A.
- `grep -r "executable_skills_section" operant/crates/operant-core/src/tools/kernel/mod.rs` — 0 (replaced by runtime call) after C.
- `operant tools list | grep kernel` — 3 after C (when enabled).
- Live: `operant run -q "use python to set x=1 then read it back" --record-trajectory` — second call must see `x==1` (persistence proof) — this is the single-Python-path proof that 016 could not show.

## 6. Rollout

- Branch `017-unified-kernel` off `main` (`4bbc83fd`), three PRs (A, B, C) each ~100-300 lines net-negative, each cherry-pickable.
- After C, open `018` only if executable skill auto-authoring is needed; otherwise kernel is perfect and on.

