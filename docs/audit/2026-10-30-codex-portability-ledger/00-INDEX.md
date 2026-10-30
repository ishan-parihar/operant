# 2026-10-30 codex-rs portability ledger — index

**Ground state:** `operant` `main` @ `cf925f76` (2026-10-30). Line numbers come from that
tree. If they drift, grep the quoted symbol.

**Reference tree:** `../external-repos/codex/codex-rs` @ `588a277b` (2026-10-29),
122 workspace crates, 2,202,563 Rust LOC.

**What this is:** a ledger of codex-rs mechanisms that operant could **import**, with the
exact source file and line, the size of the self-contained portable unit, what operant
has today, and a verdict. It exists because a 2.2M-LOC reference cannot be adopted by
reading it — only by pointing at the ~200 lines that matter and deciding, per row,
whether they earn their place.

**The bar (ponytail):** a mechanism earns an import slot only if it removes a real
operant gap AND its portable unit is small enough that copying beats rebuilding. A
30,000-LOC subsystem is not importable; it is a research project. Rows are sized and
judged on that basis. Every row below is sized by `wc -l` on the unit, not the crate.

**This supersedes the prose in** `docs/plans/2026-10-29-codex-rs-architecture-contrast-and-operant-upgrade-plan-2026.md`
(iter-1484). That plan proposed building a `ToolSpec` enum, an execpolicy DSL, a
code-mode pragma, a protocol tier and a thread-store from scratch. Reading the codex
sources afterwards showed those are the WRONG shape of work: codex's own `ToolSpec` is
OpenAI-Responses-API-specific serialization, codex's execpolicy is a Starlark DSL
(1,251 LOC + a Starlark dependency), code-mode is a V8 runtime, and the protocol tier is
four crates. The ponytail answer is different and smaller: import the ~200-line units
that close real gaps, and skip the rest.

## Files

| # | File | Subsystem family | Rows | Best in family |
|---|---|---|---|---|
| 00 | [00-INDEX.md](00-INDEX.md) | this file — the ranked shortlist | 3 | — |
| 01 | [01-tool-surface.md](01-tool-surface.md) | tool exposure, discovery, context, tracing | 9 | `ToolExposure` as a defaulted trait method |
| 02 | [02-safety-and-policy.md](02-safety-and-policy.md) | approval, permissions, exec policy, sandbox | 7 | execpolicy's `Decision` + prefix matching (838 LOC) |
| 03 | [03-session-and-turn.md](03-session-and-turn.md) | session store, turn metadata, compaction, shell | 8 | token-budget compaction (69 LOC) |
| 04 | [04-observability-and-protocol.md](04-observability-and-protocol.md) | tracing, protocol tier, capabilities | 6 | tool dispatch trace (128 LOC) |

## The shortlist — what to import first

Ordered by value-per-line. Every row is sized; "LOC" is the self-contained unit.

| # | Mechanism | Codex source | LOC | Effort | Why |
|---|---|---|---|---|---|
| 1 | **`ToolExposure` as a defaulted trait method** | `tools/src/tool_executor.rs:51-86` | ~36 | M | operant decides deferral by **string prefix** (`mcp_`, `cli_`, 23 sourcehound names). codex makes it a **6-state enum on the executor trait**. Same effect, no string matching, per-tool overridable. M not S because `assemble_tools` takes `Vec<ToolSchema>`, which loses the registry association.
| 2 | **`get_context_remaining` tool** | `core/src/tools/handlers/get_context_remaining.rs:1` + `..._spec.rs:1` | 148 | M | operant evicts **silently** — the model never learns its budget shrank. codex gives the model a live self-budget signal. |
| 3 | **Tool call trace** | `core/src/tools/tool_dispatch_trace.rs:1` | 128 | S | operant has **no** tool-call tracing at all. codex's is 128 lines. |
| 4 | **Prefix-rule execution policy** | `execpolicy/src/rule.rs` + `policy.rs` + `decision.rs` | 745 | M | operant's 47 regex patterns are **hardcoded in Rust**. codex's are a **data file** the operator writes. The `Decision` enum (27 LOC) and `PrefixPattern` (58 LOC) are directly liftable; the Starlark parser is not. |
| 5 | **Token-budget compaction (no model summarization)** | `core/src/compact_token_budget.rs:1` | 69 | S | operant compresses by **calling the model**. codex can install a fresh window **without** a model call — cheaper and deterministic. |

## What NOT to import (and why)

These are the rows that a naive "port codex" reading would put at the top. Each is
recorded here with its disqualifying fact so nobody re-litigates it.

| Mechanism | Codex size | Disqualifier |
|---|---|---|
| Code-mode (V8 runtime) | `code-mode-runtime/` 12,321 LOC + protocol 2 crates + host | operant emits **no TypeScript anywhere** (`rg -i typescript crates/` → nothing). A code runtime with no code consumer. |
| App-server protocol tier | 4 crates, 208,928 + 37,100 + 19,127 + 12,221 LOC | operant is a single-process binary whose CLI talks to core **in-process**. There is no client to serve. |
| Execpolicy **Starlark** parser | `execpolicy/src/parser.rs` 473 LOC + `starlark` crate dep | the DSL is Starlark, not YAML/TOML. Importing it adds a scripting dependency to the approval path. The **rule engine** is portable; the **parser** is not. |
| Noise-encrypted environment registry | `exec-server/src/environment_registry.rs` | operant has **no remote execution tier** to register with. |
| Sandbox backends (seccomp/bwrap/landlock) | `linux-sandbox/` 12,852 + `windows-sandbox-rs/` 30,369 LOC | operant has **no sandbox** and the operator has not asked for one. `blast_radius.rs:26` still *cites* a deleted crate's sandbox — that comment is the real defect, not the missing sandbox. |
| JSON-schema → TypeScript renderer | `code-mode-protocol/src/json_schema_types.rs` 545 LOC | no consumer (see code-mode row). |
| Parallel tool execution | `core/src/tools/parallel.rs` 1,068 LOC | operant **already has** an 8-worker semaphore pool at `agent/stream.rs:1622`. |

## Method and honesty notes

- LOC counts are `wc -l` on the named unit in the codex tree at `588a277b`.
- "ABSENT" claims carry the grep that proves them.
- Operant-side rows were re-verified against `cf925f76`, not against the iter-1484
  snapshot. Two rows from iter-1484 are **wrong** and are corrected here: codex has
  **14** hook events, not 9; and operant's `tool_search` ranks by **alphabetical
  substring**, not by relevance (see `01-tool-surface.md`).
- The single most valuable structural difference found is that codex's mechanisms
  live in **small, separately-testable units** (`tools/src/tool_executor.rs` is 139 LOC;
  `execpolicy/src/decision.rs` is 27 LOC) while operant's equivalents are **large
  modules** (`agent/run.rs` 3,021 LOC, `agent/mod.rs` 4,333 LOC, `tools.rs` 1,060 LOC,
  `approval.rs` 1,951 LOC, `context_management.rs` 2,493 LOC).
  That is why codex is importable and a general "read codex and improve operant" is not.
