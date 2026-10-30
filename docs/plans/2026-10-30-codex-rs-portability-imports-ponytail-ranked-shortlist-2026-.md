# codex-rs portability imports (ponytail-ranked shortlist, 2026-10-30)

Dated-frozen: published `2026-10-30`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

---
## Mechanism

**This plan replaces the work proposed by**
`docs/plans/2026-10-29-codex-rs-architecture-contrast-and-operant-upgrade-plan-2026.md`
(iter-1484). That plan proposed **building** five codex subsystems from scratch. Reading
the codex sources afterwards showed that is the wrong shape of work: codex's
subsystems are 8,000–483,000 LOC each, and codex's own `ToolSpec` is an
OpenAI-Responses-API wire format rather than a capability.

**The evidence for this plan** is the dated audit
`docs/audit/2026-10-30-codex-portability-ledger/` (index:
`00-INDEX.md`), which holds 30 ledger rows across four subsystem families. Each row
records the codex source file and line, the **measured size of the self-contained
portable unit**, operant's current equivalent, and a verdict.

**The rule this plan applies:** a codex mechanism earns an import slot only if its
portable unit is small enough that copying beats rebuilding, AND it closes a gap
operant actually has. That rule turned iter-1484's **15 proposed builds** into
**5 imports** and **2 one-line fixes**.

**The ponytail reading:** read another way, the five imports are also five
**decompositions**. Each moves a decision that currently lives inside a large
function into a small named type — which is why they are cheap to review and cheap to
revert.

---
## The five imports

### 1. `ToolExposure` as a defaulted trait method — IMPORT-DIRECT

- **Codex source:** `tools/src/tool_executor.rs:51-86` (the enum) and `:113-115` (the defaulted trait method). ~36 LOC inside a 139-LOC file.
- **Operant today:** `crates/operant-core/src/tools/tool_search.rs:84` — `pub fn is_deferrable(name: &str) -> bool { name.starts_with("mcp_") || is_sourcehound_composite(name) || name.starts_with("cli_") }`. The `OperantTool` trait (`crates/operant-core/src/tools.rs:270`) has no exposure method.
- **Effort:** M (S for the enum; M because `assemble_tools` takes `Vec<ToolSchema>`, which loses the registry association, so the schema must carry its exposure or the assembler must take the registry).
- **Acceptance:**
  - `pub enum ToolExposure { Direct, Deferred, Hidden }` exists with a defaulted `fn exposure(&self) -> ToolExposure { ToolExposure::Direct }` on `OperantTool`.
  - `McpNamespacedTool` (`crates/operant-core/src/mcp.rs:1982`), the sourcehound composites, and the CLI roster tools each return `ToolExposure::Deferred`.
  - `is_deferrable` no longer inspects tool names.
  - A regression test proves a tool named `mcp_lookalike` that is NOT an MCP tool is **not** deferred.
- **Ledger row:** `docs/audit/2026-10-30-codex-portability-ledger/01-tool-surface.md` Row 1.

### 2. `get_context_remaining` tool — IMPORT-DIRECT

- **Codex source:** `core/src/tools/handlers/get_context_remaining.rs:1` (109 LOC) + `core/src/tools/handlers/get_context_remaining_spec.rs:1` (39 LOC).
- **Operant today:** the arithmetic exists — `crates/operant-core/src/context_management.rs:85` (`estimate_total_tokens`) and `:1283` (`manage_context_reporting` → `EvictionOutcome` at `:802`) — but nothing exposes it to the model. `rg -n "remaining" crates/operant-core/src/agent/` finds only retry-budget and cooldown timers.
- **Effort:** S
- **Acceptance:**
  - A tool registered in `crates/operant-core/src/tools/builtin.rs` returns the remaining context budget as `tokens_left`.
  - The number it reports matches `estimate_total_tokens` on the live message list.
  - A test proves the tool is absent from the tools array when no budget is configured.
- **Why it matters:** operant's eviction is silent. The model cannot tell that operant dropped its context, so it cannot compensate.
- **Ledger rows:** `01-tool-surface.md` Row 2 and `03-session-and-turn.md` Row 2.

### 3. Tool dispatch trace through the existing observer seam — IMPORT-PATTERN

- **Codex source:** `core/src/tools/tool_dispatch_trace.rs:1` (128 LOC), `core/src/tools/call_trace.rs:1` (96 LOC), `core/src/tools/metadata_metrics.rs:1` (47 LOC).
- **Operant today:** `crates/operant-core/src/observer.rs:17` (`ObserverEvent`), `:126` (`ObserverMetric`), `:211` (`NoopObserver`), `:244` (`ConsoleObserver`), `:225` (`observer_from_config`). **No per-tool-call trace.** `rg -n "tool_call_received|tool_result_ready|metadata_bytes" crates/` → empty.
- **Effort:** S
- **Acceptance:**
  - One new `ObserverEvent` variant carrying tool name, duration, input bytes, output bytes and outcome.
  - Emitted from the single place the tool result is produced.
  - No new tracing dependency and no new framework — operant's observer is already the seam codex lacks.
  - The shed-metadata byte count is reported for tool results trimmed by `guard_tool_result` (`crates/operant-core/src/context_management.rs:1201`).
- **Ledger rows:** `01-tool-surface.md` Row 3 and `04-observability-and-protocol.md` Row 1.

### 4. Tool discovery relevance ranking — IMPORT-PATTERN (the 15-line version first)

- **Codex source:** `core/src/tools/handlers/tool_search.rs:148-180` (BM25 embedder + scorer) + `core/src/tools/handlers/tool_search_spec.rs:1` (221 LOC).
- **Operant today:** `crates/operant-core/src/tools/tool_search.rs:394-403` — the ranking is **substring containment sorted alphabetically**, truncated at `limit` (default 10, max 25). A query for "github issue" returns every tool whose name or description contains "github" or "issue", alphabetically, so the most relevant tool can be cut by the limit.
- **Effort:** S for the token-overlap score; M if the `bm25` crate is adopted.
- **Acceptance:**
  - A test proves that for a query matching several deferred tools, the **most relevant** is returned first — not the alphabetically first.
  - The `bm25` dependency is **not** added unless a measurement shows the token-overlap score is inadequate. Record that measurement in the commit body if it is.
- **Ledger row:** `01-tool-surface.md` Row 4.

### 5. Prefix-rule execution policy (TOML, not Starlark) — IMPORT-PATTERN

- **Codex source:** `execpolicy/src/decision.rs:1` (27 LOC — the `Decision` enum), `execpolicy/src/rule.rs:16-64` (`PatternToken` + `PrefixPattern`, 58 LOC), `execpolicy/src/rule.rs:73-83` (the `justification` field). The **parser** (`execpolicy/src/parser.rs`, 473 LOC) is Starlark and is **not** to be imported.
- **Operant today:** `crates/operant-core/src/approval.rs:60-101` (`ApprovalVerdict`, `ApprovalMode`) — the decision model is already equivalent. The **rules** are 47 hardcoded regex patterns plus a 12-category hardline blocklist, compiled into the binary.
- **Effort:** M
- **Acceptance:**
  - A TOML policy file under the operant home can define rules with `program`, `args`, `decision` (`allow` / `prompt` / `forbidden`) and `justification`.
  - `Decision` maps onto `ApprovalVerdict`: allow → `Allowed`, prompt → `RequiresApproval`, forbidden → `Blocked`.
  - **Load-time validation:** a rule may carry `match` / `not_match` example invocations, checked when the file loads, so a typo'd rule fails at load rather than silently never firing.
  - A CLI diagnostic answers "would this command be allowed?" — codex's `codex execpolicy check --rules <path> <command>` shape.
  - The `starlark` dependency is **not** added.
- **Why it matters:** today, adding a safety rule to operant means editing Rust and shipping a binary. It should mean writing a line in a file.
- **Ledger row:** `02-safety-and-policy.md` Row 1 (with Row 2 for the load-time validation).

---
## The two one-line fixes (ship first)

### A. The dangling `operant-runtime` sandbox reference

`crates/operant-core/src/blast_radius.rs:25-26` and `crates/operant-core/src/approval.rs:654-657` both state that real isolation lives in `operant-runtime`'s sandbox backends (landlock, bubblewrap, firejail, docker, sandbox-exec — `SandboxBackend`). **That crate was deleted in iter-892** and is absent from `Cargo.lock`. `crates/operant-os/src/policy.rs` has no `SandboxBackend`. There is no OS-level sandbox in operant: `rg -ci seccomp crates/` → 0 files, same for seatbelt / apparmor / "restricted token".

The `approval.rs` one is inside the **enforced entry point** — the place an engineer debugging an approval decision will read.

- **Acceptance:** both comments state that operant has **no** OS-level sandbox and that the approval guard plus blast-radius classification are the whole safety surface. The historical notes elsewhere (`observer.rs:207`, `tool_guardrails.rs:772`) stay unchanged — they are correct as history.
- **Ledger row:** `02-safety-and-policy.md` Row 3.

### B. The MCP schema byte cap

`crates/operant-core/src/tools/tool_search.rs:464-471` — `tool_describe` returns `s.parameters` with **no size cap**. A server advertising a 2,000-line JSON schema injects all of it. codex caps MCP specs at 8,000 bytes per spec and 64,000 total (`core/src/mcp_tool_exposure.rs:22-23`).

- **Acceptance:** a deferred tool's schema is truncated at a documented byte limit with a visible marker, and the shed byte count is reported through the observer seam from import 3.
- **Ledger row:** `04-observability-and-protocol.md` Row 2.

---
## Explicitly NOT in this plan

Each of these was measured and rejected. The disqualifying fact is recorded so the
decision survives the next reader.

| Not doing | Why | Ledger |
|---|---|---|
| A `ToolSpec` enum | codex's is an OpenAI-Responses-API **wire format**, not a capability | `01-tool-surface.md` Row 6 |
| Code-mode / V8 runtime | operant emits **no TypeScript anywhere** — a code runtime with no code consumer | `01-tool-surface.md` Row 8 |
| The Starlark policy parser | adds a scripting language to the approval path; the rule engine is portable, the parser is not | `02-safety-and-policy.md` Row 1 |
| An app-server protocol tier | 277,376 LOC across 4 crates to serve a client operant does not have | `04-observability-and-protocol.md` Row 4 |
| Sandbox backends | 43,000+ LOC; the operator has not asked for one. Fix the comment instead | `02-safety-and-policy.md` Row 6 |
| Composable permission profiles | needs a sandbox or multiple authorities to be meaningful | `02-safety-and-policy.md` Row 4 |
| Tool-level hooks | needs the richer `ToolContext` first; one change, not two | `02-safety-and-policy.md` Row 7 |
| Parallel tool execution | operant **already has** an 8-worker semaphore pool | `01-tool-surface.md` Row 7 |
| A thread-store KV | operant's session is a transcript, not a sectioned document | `03-session-and-turn.md` Row 8 |

---
## Gates / success criteria

- `cargo fmt --all`
- `./scripts/check.sh check -p operant-core --lib` — 0 warnings
- `./scripts/check.sh check -p operant-cli --bin operant` — 0 warnings
- `cargo test -p operant-core --lib` — green
- `cargo test -p operant-cli --bin operant` — green
- `bash scripts/clippy-warning-gate.sh` — no new warnings
- Every import ships with the regression test its acceptance line names
- No new dependency without a measurement in the commit body
- The two one-line fixes ship **before** any import

---
## Cost

| Import | Codex LOC | Expected operant diff | Effort |
|---|---|---|---|
| 1. `ToolExposure` | ~36 | ~120 | M |
| 2. `get_context_remaining` | 148 | ~150 | M |
| 3. dispatch trace | 271 | ~120 | S |
| 4. relevance ranking | 221 | ~60 (15-line score) | S |
| 5. prefix-rule policy | 745 portable | ~400 | M |
| A. sandbox comment fix | — | ~10 | S |
| B. MCP schema cap | ~40 | ~60 | S |

**Total expected: ~900 lines of operant change**, against the **5 subsystems / 15 builds**
that iter-1484 proposed and the **277,000+ LOC** that a literal port would have cost.

---
## Execution order

1. **A** (the sandbox comment fix) — it is a false promise on a security surface and it is 10 lines.
2. **B** (the MCP schema cap) — small, and it needs the observer seam from 3.
3. **3** (dispatch trace) — establishes the observer variant that B reports through.
4. **4** (relevance ranking) — independent, small, and fixes a correctness gap.
5. **2** (`get_context_remaining`) — independent, and it closes a real capability gap. M, not S: `ToolContext` carries only `metadata` and `session_id`, so reaching a live token count needs a small plumbing addition (`estimate_total_tokens` over the assembled messages already exists at `context_management.rs:85`).
6. **1** (`ToolExposure`) — the largest of the five; do it once the others have landed.
7. **5** (prefix-rule policy) — the largest; last, and only with the operator's agreement on the TOML shape.
