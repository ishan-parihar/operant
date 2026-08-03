# Phase 7 — Hermes Parity Design Review

**Date:** 2026-08-03 · **Status:** design review (implementation pending per-owner
prioritization) · **Cross-ref:** [`HERMES_VS_OPERANT_AUDIT_2026-08-03.md`](HERMES_VS_OPERANT_AUDIT_2026-08-03.md) §7.2 (gaps C1–C4)

The refreshed audit confirms the **core agentic loop is at parity** with
hermes-agent-ultra (prompt caching, streaming scrubber, TDG→agentmemory and
Obscura→igs migrations all landed). The four remaining gaps are *capability*
features, not loop-architecture defects. This document reviews each against the
current codebase and the upstream implementation (`hermes-agent-ultra/crates/`),
and proposes a concrete design so implementation can proceed incrementally,
one commit per item.

---

## C1 — Tool-planning layer (Medium — feature)

### Upstream reference
`hermes-tool-planning` (838 LOC, single `lib.rs`): runtime policy for **which
tools a given platform may call**:
- `normalize_platform_key("tg" | "dc" | "local")` → canonical config key
- `default_platform_toolsets()` / `configured_platform_toolsets(config, platform)`
  → `Vec<String>` toolset tokens (e.g. `["hermes-telegram"]`)
- `canonical_toolset_token(...)` — fuzzy token canonicalization
  (`"browser-use"` → `"browser"`, `"code"` → `"code_execution"`, …)
- `resolve_platform_tool_names(...)` / `resolve_platform_tool_schemas(...)` —
  filter a `ToolRegistry`'s definitions down to the platform's allowed set
- `tool_definition_summary(defs)` — compact `{name, description}` list for
  hooks/transcript metadata

### Operant current state
- `operant-tool-call-parser` covers **parsing** only (canonicalize JSON args,
  detect malformed calls) — no platform-scoped tool policy exists.
- The gateway exposes the full tool registry to every channel; there is no
  per-platform allow-list, no toolset config key in
  `operant-config` schema, and no `tool_definition_summary` helper.
- `operant-api::ToolSpec` (`crates/operant-api/src/tool.rs`) is the schema
  equivalent of `hermes_core::ToolSchema` (name + description + JSON params).

### Proposed design — `operant-tool-planning` (new small crate)
Pure-function module, no async, no I/O (mirrors upstream, so it is trivially
testable and dependency-light):

1. `normalize_platform_key(&str) -> String` — alias map (`local→cli`,
   `tg→telegram`, `dc→discord`, `whatsapp`, `sms→sms_twilio`, …) aligned with
   the 7 supported gateway platforms.
2. `canonical_toolset_token(&str) -> String` — alias folding for tool names so
   config can say `browser`, `browser-use`, `web` interchangeably.
3. `resolve_platform_tool_names(config, platform, all_names: &[&str]) -> Vec<String>`
   — `HashSet` filter with empty→all fallback (upstream semantics).
4. `tool_definition_summary(specs: &[ToolSpec]) -> Vec<serde_json::Value>` —
   `{name, description}` extraction for gateway hooks + session transcripts.
5. `default_platform_toolsets() -> HashMap<String, Vec<String>>` — built-in
   defaults per platform (telegram/discord/slack get the safe subset; CLI gets
   everything).

**Integration points** (each behind the existing gateway feature):
- `operant-gateway` chat-dispatch: filter `ToolRegistry` defs per channel
  platform before building the system prompt + tool list.
- `operant-config`: add optional `platform_toolsets: HashMap<String, Vec<String>>`
  to `GatewayConfig` (default empty → all tools, preserving current behavior).
- `operant-gateway/src/api.rs` / `ws.rs` transcripts: use
  `tool_definition_summary` for the tool-call metadata instead of ad-hoc maps.

**Validation:** unit tests for normalization/canonicalization/filtering (pure
functions), one gateway integration test asserting a discord session sees only
its toolset. Deps: `serde`, `serde_json`, `operant-api`.

**Effort:** ~0.5–1 day. **Alternative:** fold into `operant-tool-call-parser`
as a `platform` module (keeps crate count flat); recommended if the parser
already gains a `ToolSpec` dependency.

---

## C2 — Telemetry crate (Low — feature)

### Upstream reference
`hermes-telemetry` (lib.rs + otlp.rs): `TelemetryConfig`, `init_telemetry` /
`init_telemetry_from_env` (tracing-subscriber + optional OTLP),
`MetricsRegistry` (atomic counters: llm_request, tool_call, error, http,
prompt-cache hit/miss), `prometheus_text()`, `langfuse_trace_config_from_env`.

### Operant current state
- `operant-runtime/src/observability/` now has **real** `prometheus.rs` and
  `otel.rs` (opentelemetry 0.27 + prometheus 0.14, wired in Phase 2a/G4 with
  tests). The Phase 2a work effectively *delivered C2's substance*.
- `operant-core/src/observer.rs` (`Observer` trait + `ConsoleObserver`) is the
  inline event hook used by the agent loop.

### Recommendation
**No new crate.** The observability module already covers OTLP export + metrics;
the remaining gap is only that the `Observer` trait's event stream is not
counted into the metrics registry. Minimal follow-up (still worth doing):
- In `operant-runtime/src/observability/mod.rs`, add a `MetricsBridgeObserver`
  implementing `operant-core::Observer` that increments the
  `MetricsRegistry` counters on `on_llm_request`, `on_tool_call`,
  `on_error`, `on_prompt_cache_hit/miss`.
- Wire it into `operant-runtime`'s observer stack when the
  `observability-prometheus` feature is on.

**Effort:** ~2–3 hours. Defer unless dashboards need tool/LLM call counts.

---

## C3 — Eval / parity harness (Low — CI hardening)

### Upstream reference
`hermes-eval` (runner/verifier/reporter/tblite + `agent_rollout` + `hermes-bench`
binaries) — full offline/online benchmark harness with golden task pairs.

### Operant current state
- `operant-runtime/src/agent/eval.rs` is **internal-only**: complexity-tier
  heuristics + eval config plumbing. No runner, no verifier, no CLI.
- No golden task/eval pairs; no CI gate that catches loop regressions.

### Proposed design — `operant-eval` (new small crate, default feature off)
Keep it **much smaller than upstream** — the goal is regression-catching, not a
benchmark suite:
1. `src/task.rs` — `EvalTask { id, prompt, golden_actions: Vec<String>,
   expect_keywords: Vec<String> }` (serde YAML-loadable from
   `operant-eval/tasks/*.yaml`).
2. `src/runner.rs` — drive `operant-core` agent loop headlessly (query-only,
   bounded iterations, no TUI), capture tool-call sequence + final text.
3. `src/verifier.rs` — assert golden tool names appear in order (subsequence)
   and/or keywords present in the final answer; one assertion per check,
   deterministic.
4. `src/reporter.rs` — print per-task pass/fail + summary; exit non-zero on
   failure so CI/`lint-checks.sh` can gate.
5. `tasks/` — 5 golden pairs covering: tool-call ordering (e.g.
   `file_read` → `file_edit`), multi-step planning (search → extract →
   summarize), error recovery (unknown tool name), safety (no tool call for
   "ignore previous instructions"), and prompt-cache correctness marker.

**Integration:** optional `operant-eval` binary `operant-eval run`; wired into
`scripts/lint-checks.sh` as step 5 only when the binary is built (feature-gated,
so the default dev loop stays fast).

**Effort:** ~1 day. **Defer note:** requires a working model endpoint to be
useful; pure-function verifier/reporter can ship and be tested with a
`MockRunner` before real endpoints are configured.

---

## C4 — `node_detail()` (Low — UX)

### Upstream reference
`learning_mutations.py` exposes per-node inspection (read a learning-graph node
before editing it) so the agent doesn't blindly overwrite.

### Operant current state
`operant-core/src/agent/learning_graph.rs` has `delete_node` and `edit_node`
mutations but **no read-before-edit** — `learning_mutation_tool.rs` cannot show
the current node contents when the agent wants to edit one.

### Proposed design
1. `learning_graph.rs`: add
   `pub fn node_detail(node_id: &str, skills_dir: &Path, memory_dir: &Path) -> Option<NodeDetail>`
   returning `{ id, title, body, tags, mtime, sources }` parsed from the same
   markdown store the mutators use (share the existing parse helper to avoid
   drift).
2. `learning_mutation_tool.rs`: when the tool receives an `edit` action,
   automatically prepend the current node detail to the result so the agent
   sees what it is about to change (no new tool name, no schema change).

**Effort:** ~1–2 hours. Pure-function, unit-testable with a temp dir (same
pattern as `lib.rs` hardware context tests).

---

## Prioritization

| # | Item | Effort | Value | Do now? |
|---|------|--------|-------|---------|
| C4 | `node_detail()` | ~1–2h | Low (UX safety) | Yes — smallest, fully local, no new deps |
| C2 | metrics bridge observer | ~2–3h | Low–Med (dashboards) | Optional — observability substance already shipped |
| C1 | tool-planning crate | ~0.5–1d | Medium (feature parity) | Yes — pure functions, well-tested upstream shape |
| C3 | eval harness | ~1d | Low (CI hardening) | Defer until a real model endpoint is configured |

**Guardrail (per user):** CI/GitHub-workflow wiring stays out of scope; all
enforcement is local (`scripts/lint-checks.sh`) until explicitly requested.

---

## Validation checklist (per item)

- C1: `cargo test -p operant-tool-planning`; gateway integration test for
  discord toolset filtering; `cargo check --workspace --all-features` clean.
- C2: `cargo test -p operant-runtime --features observability-prometheus`; a
  fake `Observer` event bumps the matching counter.
- C3: `cargo test -p operant-eval` with `MockRunner`; `operant-eval run`
  passes golden tasks against a configured endpoint.
- C4: unit tests in `learning_graph.rs` (temp dir), same pattern as existing
  hardware-context tests; `cargo test -p operant-core`.
