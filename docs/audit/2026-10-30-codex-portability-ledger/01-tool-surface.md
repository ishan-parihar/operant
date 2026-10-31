# 01 — Tool surface: exposure, discovery, context, tracing

**Subsystem family:** how a tool is declared, whether the model sees it, how it is
found, what context it receives, and what is recorded about it.

**Verdict key:** `IMPORT-DIRECT` (lift the unit ~verbatim, adjusting imports) ·
`IMPORT-PATTERN` (copy the idea, reimplement in operant's idiom) · `SKIP` (not worth it).

---

## Row 1 — `ToolExposure` as a defaulted trait method

- **Codex:** `tools/src/tool_executor.rs:51-86` — `pub enum ToolExposure { Direct, Deferred, DeferredModelOnly, DirectModelOnly, CodeModeOnly, Hidden }` with `is_direct()` / `is_deferred()` / `is_available_in_code_mode()` predicates. The trait method is `fn exposure(&self) -> ToolExposure { ToolExposure::Direct }` at `tools/src/tool_executor.rs:113-115`.
- **Size:** the enum + predicates are ~36 lines inside a 139-LOC file.
- **Operant today:** `crates/operant-core/src/tools/tool_search.rs:84` — `pub fn is_deferrable(name: &str) -> bool { name.starts_with("mcp_") || is_sourcehound_composite(name) || name.starts_with("cli_") }`. The `OperantTool` trait at `crates/operant-core/src/tools.rs:270` has **no** exposure method.
- **Verdict:** `IMPORT-DIRECT`
- **Effort:** M (S for the enum alone; see the caveat below for why the full change is M)
- **Value:** the single highest value-per-line row in this ledger.

**Why it matters.** Operant decides deferral by **string prefix**. That has three costs:
1. A tool is deferred or not by a naming convention, so any tool whose name happens to
   start with `mcp_` or `cli_` is deferred whether or not that was intended.
2. `is_sourcehound_composite` (`tool_search.rs:96-101`) scans a table of 23 names to
   decide — a runtime lookup where a type-level answer belongs.
3. There is **no third state**. A tool cannot be "visible but not discoverable" or
   "registered but hidden". Codex's six states give per-tool control with a default that
   requires no change to any existing tool.

**The import is small.** Add the enum to `crates/operant-core/src/tools.rs` (or a new
`tools/exposure.rs`), add `fn exposure(&self) -> ToolExposure { ToolExposure::Direct }`
to `OperantTool`, and change `is_deferrable` to consult the registry. Existing tools
keep working unchanged because the default is `Direct` — but wait, the current default
behaviour is that `mcp_*`/`cli_*`/sourcehound-composite tools **are** deferred. So the
default cannot be `Direct` for those three families; they must opt in to `Deferred`
explicitly. That is still strictly less code than the prefix scan, because the opt-in
happens once at registration rather than on every schema assembly.

**Shape of the change:**
```rust
// crates/operant-core/src/tools.rs
pub enum ToolExposure { Direct, Deferred, Hidden }

pub trait OperantTool: Send + Sync {
    // ... existing methods ...
    fn exposure(&self) -> ToolExposure { ToolExposure::Direct }
}
```
Then `McpNamespacedTool` (`crates/operant-core/src/mcp.rs:1982`), the sourcehound
composites, and the CLI roster tools each return `ToolExposure::Deferred`, and
`assemble_tools` (`tool_search.rs:105`) partitions by `exposure()` instead of by name.

**Caveat (state it, do not hide it):** this changes the deferral decision for any tool
not in those three families that was, by accident, being deferred. The registry is the
single source of truth in codex — `core/src/tools/registry.rs` filters on
`tool.exposure.is_deferred()`. Operant's `assemble_tools` takes `Vec<ToolSchema>`,
which **loses the registry association**. So the import also requires the schema to
carry its exposure, or `assemble_tools` to take the registry instead. That is the real
cost: S becomes M, and the diff touches the registry and the assembler.

---

## Row 2 — `get_context_remaining` tool

- **Codex:** `core/src/tools/handlers/get_context_remaining.rs:1` (109 LOC) + `core/src/tools/handlers/get_context_remaining_spec.rs:1` (39 LOC).
- **Size:** 148 LOC total.
- **Operant today:** `ABSENT`. `rg -n "tokens_left|context_remaining|remaining_budget" crates/` returns exactly one hit, and it is a **test name** in `crates/operant-cli/src/tui/operant_app/info_widget/info_widget_usage.rs:300` (`fn used_wording_does_not_change_remaining_budget_color_thresholds`) — a TUI colour threshold, not a model-visible signal.
- **Verdict:** `IMPORT-DIRECT`
- **Effort:** M
- **Value:** operant's eviction is **silent**.

**Why it matters.** operant implements tiered eviction with a decay curve
(`crates/operant-core/src/context_management.rs`) and auto-compression on overflow.
The model is never told. It observes that earlier turns have vanished from its context
and has no way to distinguish "the system dropped that" from "I never saw that". Codex
solves this with a tool the model can call: `get_context_remaining` returns
`{"tokens_left": N}` and renders a `TokenBudgetRemainingContext` fragment.

This is a genuine capability gap, not a cosmetic one: a model that knows it is near
its limit can summarise early, split work, or stop asking for more context. Operant's
model cannot.

**Why M and not S:** the tool file is small, but the hard part is upstream of it.
operant **already has** the arithmetic — `estimate_total_tokens` (`context_management.rs:85`)
and `manage_context_reporting` (`context_management.rs:1283`) are both `pub` — so the
gap is not a missing accessor. The gap is that no tool can *reach* the live count:
`execute_tools(&self, tool_calls)` (`agent/stream.rs:679`) is called as
`self.execute_tools(tool_calls)` from `agent/run.rs:1813` while `messages` is a **local**
there, and `ToolContext` (`tools.rs:287-298`) carries only `session_id` + `metadata`. So the
port means threading the budget through `execute_tools`/`tool_context` (or a shared cell
updated on `CompactionCompleted`), then writing the tool and registering it in `builtin.rs`.
See the same blocker named for turn-diff tracking in `03-session-and-turn.md` Row 4.

---

## Row 3 — tool call tracing

- **Codex:** `core/src/tools/tool_dispatch_trace.rs:1` (128 LOC) + `core/src/tools/call_trace.rs:1` (96 LOC) + `core/src/tools/metadata_metrics.rs:1` (47 LOC) = 271 LOC.
- **Operant today:** `ABSENT` for the tracing layer. `rg -n "tool_call_received|tool_result_ready|rollout_thread_trace|metadata_bytes" crates/` → empty. operant *does* have `crates/operant-core/src/trajectory.rs` and `crates/operant-core/src/agent/events.rs`, which record tool *events*; what is missing is the **per-call dispatch trace** that says which tool was routed where, with what payload size.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** M for the trace, S for the metrics alone.
- **Value:** operant's `trajectory.rs` records *that* a tool ran; codex records *how it
  was dispatched and how big it was*. The `metadata_metrics.rs` unit (47 LOC) measures
  metadata shedding — how many bytes of tool metadata were dropped. operant has a
  `max_tool_result_share` knob (per the CHANGELOG) but no measurement of what it drops.

**Note:** `call_trace.rs` and `tool_dispatch_trace.rs` are tied to codex's
`Session`/`TurnContext` types. Lift the *shape* (a trace struct with tool name, outcome,
duration, payload sizes) into operant's existing `trajectory.rs`/`events.rs` idiom, not
the code.

---

## Row 4 — BM25 tool discovery ranking

- **Codex:** `core/src/tools/handlers/tool_search.rs:148-180` — the corpus is embedded with `EmbedderBuilder::with_fit_to_corpus(Language::English, &corpus).build()` and scored with `bm25::Scorer`. The spec builder is `core/src/tools/handlers/tool_search_spec.rs:1` (221 LOC, incl. tests) and returns `ToolSpec::ToolSearch { execution: "client", .. }`.
- **Operant today:** `crates/operant-core/src/tools/tool_search.rs:394-403` — the ranking is **substring containment, sorted alphabetically**:
  ```rust
  .filter(|s| query.is_empty()
      || s.name.to_lowercase().contains(&query)
      || s.description.to_lowercase().contains(&query))
  .map(|s| (s.name.clone(), s.description.clone()))
  .collect();
  matches.sort_by(|a, b| a.0.cmp(&b.0));
  ```
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** M (S if the `bm25` crate is acceptable as a dependency; it is a small, pure-Rust crate).
- **Value:** this is a **correctness** difference, not a preference. Operant's ranking
  means a query for "github issue" returns every tool whose description contains
  "github" or "issue", **alphabetically**, truncated at 10 — the most relevant tool may
  be cut by the limit. codex returns BM25-ranked results, so the top-10 is the top-10
  by relevance.

**This corrects iter-1484.** That plan said operant's tool search used a
"threshold-pct/catalog-budget model ... not BM25 ranking" and recommended replacing it.
That was accurate about the *catalog budget* (operant does gate the listing on
`threshold_pct` of context, `crates/operant-core/src/config.rs:1796`) but it **missed
that the search itself is unranked**. The catalog budget decides *whether* the listing
is shown; the ranking decides *which* results the model gets once it searches. Both
need fixing, and they are different fixes.

**The cheap version:** operant does not need BM25. A token-overlap score
(count of query terms present in name × 3 + in description × 1, ties broken by name
length) would beat alphabetical sorting for ~15 lines. Prefer that over adding the
`bm25` dependency unless measurement shows the simple score is inadequate. **Ponytail:
take the 15-line version first.**

**OUTCOME — iter-1510 (RT-200, CLOSED).** The 15-line token-overlap
version shipped exactly as prescribed; `bm25` was NOT taken. Code is
`relevance_score` and `search_matches` in
`crates/operant-core/src/tools/tool_search.rs` (name hit 3,
description hit 1, score-descending sort with an explicit name
tiebreak; query split on whitespace with ANY-term matching, which also
fixes the verbatim-substring gap named in Row 4's opening).
Guard: five tests in `tool_search::tests`.

**Mutation lesson — the fixtures were vacuous before they were green.**
The first `a_name_hit_outranks_a_description_only_hit` used
`mcp_github_issue_create` vs `mcp_project_tools` and passed with every
score forced to 0, because alphabetical and relevance order agreed. It
was rewritten so they DISAGREE (`mcp_zzz_github_issue_create` vs
`mcp_aaa_project_tools`, the relevant tool sorting LAST), plus a
limit-1 case. Forcing scores to 0 now fails exactly that test by name.
Two traps worth checking before writing another fixture here:
`is_deferrable` (`tool_search.rs:80-82`) only admits `mcp_*` and
sourcehound-composite names, so a non-`mcp_` fixture is silently
filtered out and the test sees 0 matches; and a source-pin assertion
whose own literal is the search string matches itself.

---

## Row 5 — tool invocation context

- **Codex:** `core/src/tools/context.rs:1` (626 LOC) — `ToolInvocation`, `ToolPayload`, `ToolEnvironments`, `ToolCallState`, `ToolOutput`.
- **Operant today:** `crates/operant-core/src/tools.rs:288-310` — `ToolContext` is **8 fields**: a `HashMap<String, String>` of metadata plus an `Option<String>` session id. Its own doc comment at `tools.rs:293-297` says the session id is "Currently dormant for all tools that don't read it".
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** M
- **Value:** operant's `ToolContext` cannot carry a **cancellation token**, a **turn
  id**, a **workspace root**, or a **diff tracker**. That is why the `turn_diff_tracker`
  idea (codex `core/src/turn_diff_tracker.rs`, 403 LOC) cannot be ported on its own —
  the tool has nowhere to receive the tracker from.

**Do not import the 626-LOC struct.** Import the *fields operant needs*: a cancellation
token, a turn id, and a workspace root. codex's struct is shaped by codex's Session
model; operant's `ToolContext` should grow by the three fields it actually lacks, and
each addition needs a user.

---

## Row 6 — typed tool-spec layer

- **Codex:** `tools/src/tool_spec.rs:1` (193 LOC) — `pub enum ToolSpec { Function, Namespace, ToolSearch, WebSearch, Freeform }` with serde tagging; `tools/src/tool_definition.rs:1` (30 LOC).
- **Operant today:** `crates/operant-core/src/tools.rs:270` (`OperantTool` trait) + `crates/operant-core/src/schema.rs` (`ToolSchema`). `rg -n "ToolSpec\b" crates/` → empty.
- **Verdict:** `SKIP` (was `IMPORT-PATTERN` in iter-1484)
- **Effort:** — 
- **Value:** none, on operant's terms.

**Why this changed.** Iter-1484 proposed building a `ToolSpec` enum. Reading
codex's actual file shows it is a **Responses-API serialization enum** — its variants
are wire shapes (`#[serde(rename = "function")]`, `web_search`, `namespace`). Its
purpose is to serialize a tool into the OpenAI Responses API JSON. operant speaks
OpenAI-compatible chat-completions and has its own `ToolSchema`. Porting `ToolSpec`
would be porting an OpenAI-specific wire format, not a capability.

The one piece worth noticing is `ToolSpec::ToolSearch { execution: "client", .. }` — a
first-class tool kind for deferred discovery, which operant models with three separate
bridge tools instead. That is a naming difference, not a gap.

---

## Row 7 — parallel tool execution

- **Codex:** `core/src/tools/parallel.rs:1` (1,068 LOC).
- **Operant today:** `crates/operant-core/src/agent/stream.rs:1622` — `Semaphore::new(8)` with `buffer_unordered(8)`.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant **already has** this. Importing codex's 1,068-LOC version would
  be a downgrade in clarity for a capability operant already ships with 8 workers and
  an interrupt-aware drain.

---

## Row 8 — JSON-schema → TypeScript renderer

- **Codex:** `code-mode-protocol/src/json_schema_types.rs:1` (545 LOC) — `render_json_schema_to_typescript`, with expansion caps (`MAX_LOCAL_REF_EXPANSIONS_PER_PATH = 2`, `MAX_TOTAL_LOCAL_REF_EXPANSIONS = 32`, `DEFAULT_INPUT_SCHEMA_MAX_BYTES = 16_000`).
- **Operant today:** `ABSENT`. `rg -ni "typescript|ts_declaration" crates/` → no declaration emitter.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant emits **no TypeScript anywhere**. This renderer exists to describe
  tools to a JavaScript runtime that operant does not have. It is a component of
  code-mode, not a standalone capability.

**Worth stealing as a pattern, though:** the expansion caps. operant's context
assembly has no equivalent bound on schema expansion, and codex's cap design (per-path
depth, total expansion count, byte budget, and a render-work multiplier charged as
strings are built) is a good model for any recursive structure operant might add.

---

## Row 9 — tool lifecycle notifications

- **Codex:** `core/src/tools/lifecycle.rs:1` (177 LOC) — `notify_tool_start` / `notify_tool_finish`.
- **Operant today:** `ABSENT` for this API. `rg -n "notify_tool_start|tool_lifecycle" crates/` → empty. operant has `AgentEvent` streaming in `crates/operant-core/src/agent/events.rs:1` and the TUI drains it, but there is no core-level tool lifecycle hook.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** L
- **Value:** low on its own. codex's lifecycle module notifies **extension
  contributors**, a system operant does not have. The portable idea is small (emit a
  start and a finish event around tool execution) but operant's `events.rs` already
  carries tool events to the TUI — the gap is that they are **not** available to
  hooks or extensions, only to the UI.

**Do not import this now.** It becomes valuable only in combination with Row 11 in
`02-safety-and-policy.md` (tool-level hooks), because both need a common seam.

---

## Family summary

| Row | Mechanism | Codex LOC | Verdict | Effort |
|---|---|---|---|---|
| 1 | `ToolExposure` defaulted trait method | ~36 | IMPORT-DIRECT | S→M |
| 2 | `get_context_remaining` tool | 148 | IMPORT-DIRECT | M |
| 3 | tool call tracing | 271 | IMPORT-PATTERN | M |
| 4 | BM25 (or simple) discovery ranking | 221 (spec) + BM25 core | IMPORT-PATTERN | M / S for the cheap version |
| 5 | tool invocation context | 626 | IMPORT-PATTERN | M |
| 6 | typed tool-spec layer | 538 | SKIP | — |
| 7 | parallel tool execution | 1,068 | SKIP | — |
| 8 | schema → TypeScript renderer | 545 | SKIP | — |
| 9 | tool lifecycle notifications | 177 | IMPORT-PATTERN | L |

**Best in family:** Row 1 (`ToolExposure`). It is the smallest change that fixes a
structural weakness — the deferral decision currently lives in a **string-matching
function** instead of on the **type that owns the tool** — and it is the one row whose
benefit grows as operant's tool count grows.

---

**OUTCOME — iter-1539 (Row 1, `ToolExposure`).** The deferral decision now lives on the type
that owns the tool. `ToolExposure { Direct, Deferred, Hidden }` at
`crates/operant-core/src/tools.rs:279-303` with a defaulted `fn exposure()` on `OperantTool`;
`ToolSchema` carries it (`schema.rs:154-184`, `with_exposure`); the registry stamps it at the
three minting sites (`tools.rs:641,661,700`); `assemble_tools`
(`tools/tool_search.rs:107-171`) partitions by the stamped exposure. `McpNamespacedTool`
(`mcp.rs:2126`) and `SourcehoundCompositeTool` (`sourcehound_composites.rs:450`) return
`Deferred`. Five tests guard it, mutation-proven by reverting `assemble_tools` to
`is_deferrable`, which fails exactly the three new ones by name.

**The trap that cost the most time, recorded because it will cost it again:** fixtures must
reach `assemble_tools` in PRODUCTION'S shape. A hand-built `ToolSchema::new` defaults to
`Direct`, so a fixture that never stamps looks like the code is broken. That is how
`registered_composites_ride_the_bridge_not_the_request`
(`sourcehound_composites.rs:841`) failed 5/5 in isolation — and how the "parallel load flake"
diagnosed as **RT-203 in iter-1523 was WRONG**: it was this defect, deterministic, and that
earlier "delta-proven not mine" compared the wrong thing (the touched modules passed in
isolation; only the whole-file suite surfaced the break). RT-203 is closed with the
correction named in its own row.
