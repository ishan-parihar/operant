# 04 — Observability and protocol: tracing, metrics, capabilities, the wire tier

**Subsystem family:** what operant records about itself, and whether it has a wire
protocol at all.

**Verdict key:** `IMPORT-DIRECT` · `IMPORT-PATTERN` · `SKIP`.

---

## Row 1 — tool dispatch trace

- **Codex:** `core/src/tools/tool_dispatch_trace.rs:1` (128 LOC) — `ToolDispatchTrace`. The companion `core/src/tools/call_trace.rs:1` (96 LOC) and `core/src/tools/metadata_metrics.rs:1` (47 LOC).
- **Operant today:** operant has a **metric** surface (`crates/operant-core/src/runtime_metrics.rs:34` — `RuntimeMetrics`, `:69` — `MetricsSnapshot`) and an **observer** surface (`crates/operant-core/src/observer.rs:17` — `ObserverEvent`, `:126` — `ObserverMetric`, with a `NoopObserver` at `:211` and a `ConsoleObserver` at `:244`), including `ObserverEvent::ToolCall { tool, duration, success }`. **CORRECTION, measured at implementation time (iter-1505): this row as first written claimed operant had no per-tool-call trace at all. That was imprecise.** The trace exists and is emitted from every turn; what is missing is the **duration value** — the emit site was `duration: Duration::from_millis(0)`, a hardcoded constant, so `ConsoleObserver`'s `duration_ms` was structurally always `0`. The timing was already measured at all three `execute_tools` branches (each feeding `TurnEndBus::record_tool_duration`, a contract `turn_end.rs`'s own test pins) and then discarded one hop before the observer.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** S
- **Value:** closes the gap between "a counter went up" and "here is what happened".

**Why it matters.** operant's `ObserverMetric` is a **counter**. When an operator asks
"why did this turn take 40 seconds and 12,000 tokens?", operant can say the aggregate
but not which tool was slow or how large its result was. codex's dispatch trace answers
that per call. `metadata_metrics.rs` (47 LOC) additionally measures **how many bytes of
tool metadata were shed** — operant has a `max_tool_result_share` knob and a
`guard_tool_result` function (`crates/operant-core/src/context_management.rs:1201`)
that trims, but no measurement of what it trimmed.

**Ponytail version:** operant already has `ObserverEvent` and the observer trait. Add
one `ObserverEvent` variant carrying `(tool_name, duration, input_bytes, output_bytes,
outcome)` and emit it from the one place the tool result is produced. That is a
**single emit site plus one variant** — no new tracing framework, no new dependency.
The 128-LOC codex file is not needed; the *shape* is.

---

## Row 2 — MCP tool exposure policy

- **Codex:** `core/src/mcp_tool_exposure.rs:1` (282 LOC) — `McpHandlerCache`, `SharedMcpDefinitions` (weak-reference cache keyed on `(input_schema ptr, tool_name, agent_plugin, schema_max_bytes)`), plus byte budgets `MAX_AGENT_PLUGIN_MCP_SPEC_BYTES = 8_000` and `MAX_AGENT_PLUGIN_MCP_TOTAL_BYTES = 64_000`. It also applies a per-connector `AppToolPolicyEvaluator` to decide model visibility.
- **Operant today:** `crates/operant-core/src/mcp.rs:1` (3,486 LOC) with `McpNamespacedTool` at `:1982` and `McpManager` at `:2145`. `rg -n "fn.*exposure|expose|filter.*tool|deferred" crates/operant-core/src/mcp.rs` returns **nothing** — operant has no exposure-policy concept inside the MCP layer.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** M
- **Value:** the **byte budgets** are the portable part.

**Why it matters.** codex caps how many bytes of MCP tool schemas may reach the model
(8,000 per spec, 64,000 total). operant has no such cap in the MCP layer — its budget
control is the generic `tools.tool_search` listing budget
(`crates/operant-core/src/config.rs:1804` — `listing_max_tokens`, default 4,000),
which governs the **catalog listing**, not the individual tool schemas that are
injected when a deferred tool is materialised.

So the question operant has not answered is: **when the model calls `tool_call` for a
deferred MCP tool, how large is the schema it gets back?** If a server advertises a
2,000-line JSON schema, operant injects all of it with no cap.

**Ponytail version:** add a byte cap in the one place a deferred tool's schema is
returned (`tool_describe` / the deferred-materialisation path in
`crates/operant-core/src/tools/tool_search.rs`), truncate with a clear marker, and
count the bytes. codex's `schema_max_bytes: Option<NonZeroUsize>` threaded through the
cache is the shape; operant's version needs one constant and one truncation.

**The `SharedMcpDefinitions` weak cache is SKIP** — it exists to avoid keeping
completed sessions alive, a problem operant does not have (its MCP manager is per-process).

---

## Row 3 — MCP capability negotiation

- **Codex:** `protocol/src/capabilities.rs:1` (89 LOC) — `SelectedCapabilityRoot`, `EnvironmentCapabilityRoots`, `CapabilityRootLocation`. It is a **root-selection** model (which filesystem roots an environment exposes), not a general "what can this peer do" handshake. There is also `protocol/src/dynamic_tools.rs` and a capabilities handshake in the MCP connect path.
- **Operant today:** `ABSENT` for a capability model. `rg -n "capabilit" crates/operant-core/src/mcp.rs` → no capability negotiation surface.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** not now.

**Why SKIP, precisely.** I said "MCP capability negotiation" in iter-1484's gap list.
Reading the file, codex's `capabilities.rs` is about **selecting filesystem roots for
an environment** — a concept that exists because codex has an `exec-server` tier with
remote environments. operant has neither remote environments nor a root-selection
problem. The MCP handshake capability flags are a smaller thing and would only matter
if operant connected to servers that gate features on them.

---

## Row 4 — the app-server protocol tier

- **Codex:** `app-server/` (208,928 LOC) + `app-server-protocol/` (37,100 LOC) + `app-server-transport/` (19,127 LOC) + `app-server-daemon/` (12,221 LOC) + `app-server-client/` + `app-server-test-client/`. The `protocol/` crate itself is 33,297 LOC.
- **Operant today:** operant is a **single-process binary**. `crates/operant-cli/src/main.rs` builds the agent and the TUI in one process; the gateway runner (`crates/operant-cli/src/gateway_runner.rs`) runs the agent in the gateway process. There is **no** client/server split, **no** daemon protocol, **no** wire types.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** this is the single biggest codex subsystem and the single **worst**
  value-per-line candidate in the entire ledger.

**Why SKIP, with the reasoning stated so it is not re-litigated.** A protocol tier
exists to let **a client** drive **a server**. operant has no second client. The
gateway is a *different surface*, not a different process talking to the same agent —
and it already shares the core in-process. Introducing an app-server would mean
splitting operant into two binaries that talk over JSON-RPC, adding ~37,000 LOC of
protocol types, and **changing every call site** from an in-process async call to a
serialized round trip.

The only thing codex's app-server buys that operant lacks is the **ability for an
external editor or IDE to drive operant**. That is a real product capability — but it
is a *feature request*, not an architecture import, and it should be scoped as one if
the operator ever wants it.

**One narrow piece worth noticing:** codex's `app-server-protocol/` defines an
**item/event vocabulary** (`protocol/src/items.rs`) that is much richer than operant's
`AgentEvent`. operant's `crates/operant-core/src/agent/events.rs:1` already models
agent events. Not a gap.

---

## Row 5 — OTEL / tracing instrumentation

- **Codex:** `core/src/otel_init.rs`, `core/src/diagnostics/`, the `analytics/` crate (18,368 LOC), `core/src/turn_timing/`, `core/src/direct-tool-timing/`.
- **Operant today:** `crates/operant-core/src/observer.rs` (a `NoopObserver` / `ConsoleObserver` pair), `crates/operant-core/src/runtime_metrics.rs`, `crates/operant-core/src/insights`-style reporting via `crates/operant-core/src/agent/insights.rs` (713 LOC), and the dashboard (`crates/operant-cli/src/dashboard_server.rs`).
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant already has an observability surface and a dashboard.

**Why SKIP:** operant's `ObserverEvent` / `ObserverMetric` / `RuntimeMetrics` /
`MetricsSnapshot` plus the dashboard cover the operator's "what is my agent doing"
need. codex's `analytics/` crate is 18,368 LOC because it serves a fleet with
per-user billing — a scale operant does not operate at.

**The narrow exception:** `core/src/turn_timing/` — per-turn timing instrumentation —
maps onto operant's `AgentEvent` stream, which already carries timing (the TUI renders
durations). Not a gap.

---

## Row 6 — the `observer` seam as the integration point

- **Codex:** codex has **no** single observer trait; observability is spread across `analytics/`, `otel_init.rs`, `turn_timing/`, and per-module metric emitters.
- **Operant today:** `crates/operant-core/src/observer.rs:17-244` — `ObserverEvent` enum, `ObserverMetric` enum, `NoopObserver`, `ConsoleObserver`, and `observer_from_config` (`:225`).
- **Verdict:** **operant is AHEAD here**
- **Effort:** — 
- **Value:** operant's observer seam is the integration point that codex does not have.

**This row is in the ledger to prevent a false import.** When Row 1 (tool dispatch
trace) is implemented, the right move is to emit through operant's **existing**
`ObserverEvent` seam, not to adopt codex's `analytics/` crate or add an OTEL
dependency. operant has the better shape here: one trait, two implementations, a
config-driven constructor.

**Read this row as the ledger's general rule:** for every codex mechanism that is
about *observing*, check operant's `observer.rs` first — it is already the seam
codex lacks.

---

## Family summary

| Row | Mechanism | Codex LOC | Verdict | Effort |
|---|---|---|---|---|
| 1 | tool dispatch trace | 128 + 96 + 47 | IMPORT-PATTERN | S |
| 2 | MCP schema byte budgets | ~40 (of 282) | IMPORT-PATTERN | S |
| 3 | MCP capability negotiation | 89 | SKIP | — |
| 4 | app-server protocol tier | 277,376 (4 crates) | SKIP | — |
| 5 | OTEL / analytics | 18,368+ | SKIP | — |
| 6 | the observer seam | — | **operant is ahead** | — |

**Best in family:** Row 1. One new `ObserverEvent` variant and one emit site closes
the "which tool was slow" question. **Best avoided:** Row 4 — 277,000 LOC across four
crates to serve a client operant does not have.

---

## The ledger's closing argument

Across all four families, the pattern is the same, and it is the reason codex is
importable and a general "read codex and improve operant" is not:

> **codex's portable units are small and separately testable.**
> `tools/src/tool_executor.rs` — 139 LOC, holding the `ToolExposure` enum that governs
> every tool's visibility.
> `execpolicy/src/decision.rs` — 27 LOC, holding the allow/prompt/forbidden model.
> `core/src/compact_token_budget.rs` — 69 LOC, holding a whole compaction mode.
> `core/src/tools/tool_dispatch_trace.rs` — 128 LOC, holding a whole tracing shape.
>
> **operant's equivalents are large modules.**
> `agent/run.rs` — 3,021 LOC. `agent/mod.rs` — 4,333 LOC. `tools.rs` — 1,060 LOC.
> `approval.rs` — 1,951 LOC. `context_management.rs` — 2,493 LOC.
>
> The gap is not "operant is missing codex's features". It is that **operant's
> mechanisms are not decomposed into units small enough to compare against anything.**
> The first three imports in this ledger — `ToolExposure`, `get_context_remaining`,
> the dispatch trace — are also, read another way, the first three **decompositions**:
> they move a decision from a large function into a small named type.

That is why this ledger recommends five imports and not fifteen. The other ten rows
are not wrong about the gap; they are wrong about the *size of the gap* once you
measure the portable unit instead of the subsystem.
