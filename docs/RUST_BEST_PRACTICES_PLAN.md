# Operant — Rust Best Practices & Operational-Loop Fix Plan

**Date:** 2026-08-02
**Author:** Buffy (AI Agent)
**Scope:** Full operant workspace (16 crates) — rust-best-practices compliance + remaining operational-loop gaps vs `hermes-agent-ultra`
**Method:** clippy sweep (all crates), static unwrap/expect/panic scan with test-module exclusion, dead-code suppression audit, error-handling dependency audit, CI-gate verification, architecture contrast vs hermes-agent-ultra

---

## 1. Executive Summary

The core agentic loop is architecturally sound and was validated end-to-end against a live
provider (see `AUDIT_2026-08-02.md` §7). The gaps are **not** in the loop itself but in
**engineering hygiene** and **enforcement**:

| Metric | Current | Target |
|--------|---------|--------|
| Production `unwrap()` (excl. test modules) | **~440** (core: 268, runtime: 141, tools: 18, mem: 6, gw: 4, cfg: 3) | 0 (enforced) |
| Production `expect()` | **~310** (gw: 100, core: 86, tools: 78, cfg: 44, mem: 4) | 0 (enforced) |
| Production `panic!` | 0 (all 5 sites are in test modules) ✅ | 0 |
| Clippy warnings (lib targets) | **20** (core: 14, gateway: 6) | 0 (CI gate) |
| `#![deny(missing_docs)]` | **0** crates | all lib crates |
| Workspace lint config | 1 rule (`unexpected_cfgs`) | comprehensive |
| CI clippy gate | `-D warnings` **currently failing** | green gate |
| `#[allow(dead_code)]`/`#[allow(unused_*)]` | **242** (from DEAD_CODE_GAP_ANALYSIS) | triage to 0 |
| `anyhow` in library deps (should be binaries-only) | memory, gateway | thiserror |

**CI is red today**: `.github/workflows/ci.yml:35` runs `cargo clippy --workspace --all-targets --all-features -- -D warnings` but local clippy shows 20 lib warnings (plus more under `--all-targets`). The last CI run (v0.1.4, 2026-07-18) ended in failure.

---

## 2. Data Collected

### 2.1 unwrap() hotspots (production code, test modules excluded)

| File | unwrap() | Notes |
|------|----------|-------|
| `operant-core/src/database.rs` | 225 | 43 are `lock().unwrap()`; ~182 are DB-result unwraps (`?`-able) |
| `operant-core/src/gateway_session.rs` | 86 | mostly `?`-able |
| `operant-core/src/approval.rs` | 63 | 0 lock-unwraps; all convertible |
| `operant-core/src/skill_usage.rs` | 40 | |
| `operant-core/src/skills_guard.rs` | 34 | |
| `operant-core/src/memory.rs` | 33 | |
| `operant-runtime/src/cron/store.rs` | 131 | largest single runtime hotspot |
| `operant-runtime/src/security/webauthn.rs` | 90 | |
| `operant-runtime/src/cron/scheduler.rs` | 86 | |
| `operant-runtime/src/tools/delegate.rs` | 84 | |
| `operant-runtime/src/tools/file_read.rs` | 81 | |
| `operant-runtime/src/sop/engine.rs` | 67 | |
| `operant-runtime/src/agent/loop_.rs` | 63 | **the runtime agent loop** |
| `operant-runtime/src/skills/audit.rs` | 58 | |

### 2.2 expect() hotspots

| Crate | expect() | Risk |
|-------|----------|------|
| operant-gateway | 100 | highest — many are `unwrap_or_default`-style with custom messages |
| operant-core | 86 | |
| operant-tools | 78 | |
| operant-config | 44 | |
| operant-memory | 4 | |

### 2.3 Clippy warnings (lib targets)

- **operant-core (14):** `manual_range_contains`, `redundant_closure`, `map_or` simplification, `manual_repeat_n` (×2), identical-if-blocks, consecutive `str::replace`, borrowed-expression-traits (×2), derive-able impl, collapsible-if (×2), doc-list-indentation, redundant borrowing.
- **operant-gateway (6):** empty line after doc comment (×2), empty line after outer attribute (×2), let-result-in-block.

### 2.4 Dead code suppressions

242 `#[allow(dead_code)]`/`#[allow(unused_*)]` across 10 crates (core: 52, cli: 42, gateway: 42, channels: 27, providers: 13, tools: 9, runtime: 6, memory: 4, infra: 1, plugins: 1). Largest legitimate category is tool-argument structs; the actionable categories are unwired learning-graph mutations and MCP infrastructure.

### 2.5 Error-handling dependency audit

- ✅ `operant-core`, `operant-tools`: use `thiserror` (correct for libraries)
- ❌ `operant-memory`, `operant-gateway`: list `anyhow` in lib deps (should be `thiserror`; `anyhow` is for binaries)
- `operant-runtime` error posture: mixed (unwrap-heavy)

### 2.6 Architecture contrast vs hermes-agent-ultra

| Capability | hermes-agent-ultra | operant | Gap |
|-----------|-------------------|---------|-----|
| Tool planning (multi-step orchestration) | `hermes-tool-planning` crate | `operant-tool-call-parser` (parsing only) | **Missing planning layer** |
| Telemetry/observability | `hermes-telemetry` crate | inline `Observer` in agent | Partial — not a crate |
| Eval/parity harness | `hermes-eval`, `hermes-parity-tests` | none | **Missing** |
| Provider/app runtime separation | `hermes-provider-runtime` + `hermes-app-runtime` | single `operant-runtime` | Consolidation choice (OK) |
| Clippy enforcement | incremental `clippy-warning-gate.sh --check` | hard `-D warnings` (failing) | Adopt incremental gate |

---

## 3. Fix Plan (prioritized, incremental, commit-per-phase)

### Phase 0 — Stop the bleeding: green CI gate (highest priority, ~30 min)

**Goal:** CI clippy stops failing immediately; enforcement becomes incremental (hermes-style) so the gate never blocks the whole repo again.

1. Add `scripts/clippy-warning-gate.sh` (port hermes's approach): parse `cargo clippy --workspace --all-targets` output, fail only if the warning count **increases** relative to a committed baseline file (`clippy-baseline.txt`), or if any `error` appears.
2. Generate the baseline from today's counts (documented in the file header).
3. Update `.github/workflows/ci.yml:35` to run the gate script instead of hard `-D warnings`.
4. Keep `-D warnings` only for the specific crates that are already clean (tools, memory, config).
5. **Validate:** `bash scripts/clippy-warning-gate.sh` passes; CI config parses.
6. **Commit:** `ci: incremental clippy gate with baseline — repo stops being red`

### Phase 1 — Fix all 20 lib clippy warnings (~1 hr)

Apply `cargo clippy --fix` + manual review for the mechanical lints in `operant-core` (14) and `operant-gateway` (6). All are stylistic (`map_or`, `repeat_n`, collapsible-if, doc spacing). No behavioral change.

1. `cargo clippy --fix -p operant-core -p operant-gateway --allow-dirty`
2. Manual fix of the remaining non-auto-fixable items (identical-if-blocks in `message_safety.rs`, derive-able impl in `prompt_caching.rs`).
3. **Validate:** `cargo clippy -p operant-core -p operant-gateway` → 0 warnings; full `cargo test -p operant-core`.
4. **Commit:** `lint: zero clippy warnings in operant-core + operant-gateway`

### Phase 2 — unwrap()/expect() remediation in operant-core (largest, ~2–3 hr, split into sub-commits)

**Strategy per skill (Chapter 4 — Error Handling):**
- `lock().unwrap()` → `lock().unwrap_or_else(|e| ...)` only where a poisoned lock is a real risk; prefer `expect("poisoned lock — bug")` with justification where the invariant is programmer-error (documented), since poisoning a std mutex is a panic-level bug by definition.
- DB/file-result unwraps → `?` propagation or `.context("...")` via existing `Error`.
- Pure-invariant unwraps (proven unreachable) → `#[expect(clippy::unwrap_used, reason = "...")]` with justification, so the *lint remains enforced* elsewhere.

**Sub-commits (each independently testable):**
1. `operant-core/src/database.rs` (225) — biggest win; convert to `?`/context. **Validate:** `cargo test -p operant-core database`
2. `operant-core/src/gateway_session.rs` (86) + `approval.rs` (63)
3. `operant-core/src/memory.rs` (33) + `skill_usage.rs` (40) + `skills_guard.rs` (34)
4. remaining core files (`skills.rs`, `profile.rs`, `config.rs`, `kanban/`, `patch_tool.rs`, `file_state.rs`)
5. Add `#![deny(clippy::unwrap_used)]` + `#![deny(clippy::expect_used)]` to `operant-core/src/lib.rs` at the end (with `#[expect]` justification escapes) so the count cannot regress.

### Phase 3 — unwrap()/expect() remediation in operant-runtime (~2 hr)

Same strategy on the runtime hotspots (cron/store 131, webauthn 90, cron/scheduler 86, delegate 84, file_read 81, sop/engine 67, agent/loop_ 63, skills/audit 58). Add `deny` attrs on completion.

### Phase 4 — unwrap()/expect() in the remaining crates (~1 hr)

`operant-tools` (18 unwrap, 78 expect), `operant-memory` (6/4), `operant-gateway` (4/100 — 100 expects to convert), `operant-config` (3/44). Add deny attrs.

### Phase 5 — Library error hierarchy: anyhow → thiserror (~1 hr)

1. `operant-memory`, `operant-gateway`: replace `anyhow::Result`/`anyhow::Error` in lib code with `thiserror`-derived crate-local errors (or reuse `operant_core::error::Error` where already depended on). Keep `anyhow` only in `operant-cli` (binary).
2. **Validate:** `cargo check --workspace`; `cargo test -p operant-memory -p operant-gateway`.
3. **Commit:** `refactor(error): thiserror for library crates (memory, gateway)`

### Phase 6 — Documentation & missing_docs (~1–2 hr)

1. Add `#![deny(missing_docs)]` to the small/clean crates first (config, memory, tool-call-parser); add `#![warn(missing_docs)]` to core/tools/gateway to build the habit without a 500-error burst, then escalate to `deny` once clean.
2. Add `///` doc comments for the public surface that the compiler flags (iterative: fix, compile, repeat).
3. Replace `#[allow(dead_code)]` on real-but-unwired code (learning-graph mutations, MCP infra) with `#[expect(dead_code, reason = "...")]` + a tracked TODO, or wire the code (see Phase 7).
4. **Commit:** `docs: missing_docs enforcement + dead_code suppression triage`

### Phase 7 — Operational-loop gaps vs hermes-agent-ultra (2–4 hr, design review first)

These are *capability* gaps, not hygiene — schedule after Phases 0–6 (or as parallel tracks):

1. **Tool-planning layer** (`hermes-tool-planning` parity): a module that lets the agent compose multi-step tool plans (plan → execute → verify) rather than single-shot ReAct calls. Design doc first.
2. **Telemetry crate** (`hermes-telemetry` parity): extract the inline `Observer` into `operant-telemetry` with metrics export (OpenTelemetry-ready).
3. **Eval/parity harness**: `operant-eval` with a small suite of golden task/eval pairs (mirror `hermes-eval`), so operational-loop regressions are caught in CI.
4. **BUGS.md triage**: the doc (last updated 2026-06-19) lists 15+ items; verify each against current code — most (patch tool, SSRF, tool executor, session resume, turn context) are already implemented and should be marked resolved; keep only genuinely-open items.

**Each sub-item gets its own commit + validation.**

### Phase 8 — Enforce and lock in (~30 min)

1. Add `scripts/lint-checks.sh` wrapping: clippy gate, `cargo fmt --check`, `#![deny]` audit (`grep` for the deny attrs), unwrap/expect count check (fail if prod count > 0).
2. Wire into CI alongside the existing test jobs.
3. Update `docs/DEAD_CODE_GAP_ANALYSIS.md` and `docs/TODO.md` with the new status.

---

## 4. Validation Strategy (every phase)

| Phase | Validate |
|-------|----------|
| 0 | gate script passes; baseline committed |
| 1 | clippy 0 warnings (core, gateway); `cargo test -p operant-core` |
| 2–4 | per-crate `cargo test`; deny attrs compile; clippy clean |
| 5 | `cargo check --workspace`; `cargo test -p operant-memory -p operant-gateway` |
| 6 | `cargo build` with deny attrs; `cargo doc --no-deps` succeeds |
| 7 | per-feature `cargo test`; eval harness runs in CI |
| 8 | full `cargo test --workspace` + lint-checks.sh green |

All commits pushed to `github/main` after each phase (per project convention).

---

## 5. Risk Notes

- **unwrap in tests is fine** — the deny attrs (`unwrap_used`) should be applied at crate root with `#[cfg_attr(not(test), deny(...))]` semantics or the lint configured to ignore `#[cfg(test)]` modules so test ergonomics aren't harmed.
- **`lock().unwrap()`**: converting every lock-unwrap to graceful error handling changes panic semantics. Standard-library poisoning is a genuine invariant violation; the recommended pattern is `expect("mutex poisoned")` + `#[expect]` justification rather than propagating, so a poisoned-lock panic remains loud while the lint stays enforced.
- **Phases 2–4 are mechanical but large** (~750 sites). Use `cargo clippy --fix` for the auto-fixable fraction first, then review each manual conversion; do not bulk-`#[allow]`.
- **CI gate swap (Phase 0) is a judgment call**: hard `-D warnings` is stricter but currently blocks the repo. The incremental gate is the hermes-approved pattern and keeps the repo green while Phases 1–6 converge; once all crates are clean, the gate can be tightened back to `-D warnings` (recorded as a follow-up).
