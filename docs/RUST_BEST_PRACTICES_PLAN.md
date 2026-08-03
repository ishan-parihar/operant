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
| Production `unwrap()` (excl. test modules) | **0** across all 6 lib crates ✅ (was ~440: core 268, runtime 141, tools 18, mem 6, gw 4, cfg 3) | 0 (enforced — Phase 8) |
| Production `expect()` | **~310** (gw: 100, core: 86, tools: 78, cfg: 44, mem: 4) | 0 (enforced — Phase 2b/8) |
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

## 2.7 Progress Tracker (updated 2026-08-03)

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 0 — clippy gate | ✅ DONE | `scripts/clippy-warning-gate.sh` + `.ci/clippy-allowlist.txt` (4 entries, all in the user's `prompt_input` WIP). CI wiring deferred per user (workflows out of scope). |
| Phase 1 — 20 lib clippy warnings | ✅ DONE | core + gateway lib targets 0 warnings; `--all-targets` also clean (commits b00b2d1, add4a48). |
| Phase 2 — core unwraps | ✅ **0/268** | database, gateway_session, approval, skill_usage, kanban/*, cronjobs, gateway_markdown, schema, profile, mcp, mcp_oauth, agent/mod, tools/* — all prod unwraps eliminated (b7ebc77, fdf3ec6, 04805ca, 0181db7, 217cf30, 782b9b8). |
| Phase 2b — expect-site checklist | ✅ DONE (0f4f3af0) | `unwrap_used`/`expect_used` denied in gate via `-D` flags (manifest lints unusable in this cargo build — see gate comment). 277 `#[expect]` escapes applied by `scripts/expect-annotate.py` (idempotent, per-category reasons); test targets exempted via `cfg_attr(test, allow)` + `tests/` headers. G2/G3 closed. |
| Phase 3 — runtime unwraps | ✅ **0/141** | security/leak_detector + prompt_guard static-regex unwraps -> rx() helpers (f2c1558); sop/engine, trust/types, identity, doctor, crypto, security_ops, tools/mod, skillforge, model_switch, command_logger, tool_execution, agent/agent, loop_ -> justified expect or graceful if-let (cca00d5). prometheus.rs (25 expects) behind broken observability feature — OPEN. |
| Phase 4 — tools/memory/gateway/config unwraps | ✅ **0/31** | calculator NaN-safe total_cmp sort, jira/linkedin/notion guarded json!, tool_search/web_search locks+regexes, git_operations while-let walk, memory consolidation/snapshot, gateway api_config/sse, config policy (06a5289). Also caught turn_context.rs evolution_state lock in core (missed in Phase 2 sweep). |
| Phase 5 — anyhow → typed errors | ✅ **DONE** | **operant-memory** (2cfaa97e): typed `Error` + `MemoryContextExt` in `src/error.rs`, `MemoryResult<T> = Result<T, MemoryError>` seam in operant-api (Message/Io/Serde/Backend), anyhow removed incl. dev-deps. **operant-gateway**: new `src/error.rs` — typed `Error` (Message/Io/AddrParse/Backend/NeedsOnboarding) + `GatewayContextExt` + `Result<T, E = Error>` alias (defaulted second param preserves two-arg axum usages); tls.rs (5 fns), ws.rs `resolve_session_cwd`, lib.rs chat-dispatch/persist/serve all typed; needs_onboarding marker upgraded from fragile substring match to typed `Error::NeedsOnboarding` variant; boundary seams (Tool/Channel/Provider trait contracts + config `map_prop_error`) kept on anyhow with justification comments; 201 gateway tests pass. |
| Phase 6 — missing_docs | ⛔ not started | `#![deny(missing_docs)]` on config/memory/tool-call-parser first. 0 deny-attrs across all 10 lib crates today. |
| Phase 7 — hermes parity (tool planning, telemetry, eval) | 🔶 design review done | `docs/PHASE7_PARITY_DESIGN.md` covers C1 (tool-planning crate design), C2 (metrics bridge observer — observability substance shipped in Phase 2a), C3 (eval harness), C4 (node_detail). C4 smallest (~1–2h), C1 ~0.5–1d, C3 deferred until a real model endpoint. |
| Phase 9 — audit refresh | ✅ DONE | `docs/HERMES_VS_OPERANT_AUDIT_2026-08-03.md` supersedes the stale July audit (prompt caching + streaming scrubber now implemented; TDG→agentmemory + Obscura→igs migrations done). Gaps G1–G6 recorded; BUGS.md triage (G5) pending. |
| Phase 8 — enforcement | ✅ DONE (85407388) | `scripts/lint-checks.sh` wraps `cargo fmt --check` + clippy gate (`-D unwrap/expect`) + deny-attr audit + gate-flag presence. All 4 checks pass. CI wiring left out of scope per user. |

**`--all-features` inventory — ALL FIXED (2026-08-03):** runtime observability
(otel/prometheus wired to real deps in eb76a5a7), hardware `hardware`
(firmware/ assets committed + vendor-SDK modules gated behind the undeclared
`hardware-vendor` cfg in e32da964), plus tools `probe`, core `anthropic` (Send),
gateway `schema-export` (feature-gated import). `cargo check --workspace
--all-features` is now validated green (0 errors / 0 warnings); AGENTS.md
Local Compilation Protocol updated accordingly (c677831c).

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
5. Add `#![deny(clippy::unwrap_used)]` + `#![deny(clippy::expect_used)]` to `operant-core/src/lib.rs` at the end (with `#[expect]` justification escapes) so the count cannot regress. **Expect escapes needed at these sites (from Phase 2 work):** gateway_session.rs read/write expects (~27), database.rs `conn()` accessor, approval.rs `rx()`, gateway_markdown.rs `rx()` + capture-group expects, agent/mod.rs lock expects (~8), skill_usage.rs (~9), mcp_oauth.rs (~7), file_state/feishu/todo/notify/diagnostics/triage/dispatcher lock expects, computer_use/kanban_tool/mcp serde expects. Do NOT add `#[expect]` before the deny lands (restriction lint not enabled yet → unfulfilled-expectation warnings).

### Phase 3 — unwrap()/expect() remediation in operant-runtime ✅ DONE

**Note:** the plan's original hotspot counts (cron/store 131, webauthn 90, …) were gross counts *including* test modules; the production-only reality was 65 unwraps + 40 expects. All eliminated: static-literal regex unwraps → `rx()` helpers (leak_detector, prompt_guard, loop_), lock-invariant + guarded-by-check sites → justified `expect`, data-dependent `tool_call_id` sites → graceful `if let`. prometheus.rs (~25 expects) remains behind the broken `observability-prometheus` feature (never compiled in default features) — tracked OPEN in the `--all-features` inventory.

### Phase 4 — unwrap()/expect() in the remaining crates ✅ DONE

`operant-tools` (18 unwrap → 0; used `f64::total_cmp` for NaN-safe sort in calculator per Ch.1), `operant-memory` (6 → 0), `operant-gateway` (4 → 0), `operant-config` (3 → 0). The gateway's 100 expects remain (Phase 2b). One missed core site (turn_context.rs `evolution_state` lock) found by rigorous test-boundary scan and fixed in the Phase 4 commit.

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
