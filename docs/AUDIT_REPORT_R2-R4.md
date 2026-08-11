# Operant Audit Report — Rounds 2-4 (2026-08-06)

## Summary

Shipped 9 fixes across 4 audit rounds, each verified live against the running binary (0.1.4). The core `operant run` path uses `OperantAgent` (operant-core), NOT `RuntimeAgent` (operant-runtime) — the runtime agent stack is compiled via default features but is dead-linked in the CLI's core path (only its personality/tools/cron submodules are consumed by the gateway).

## Fixes Shipped (all committed + deployed)

| # | Fix | Commit | Live-Verified |
|---|-----|--------|--------------|
| Bug #1 | Session aggregate counters dead (message_count, tool_call_count stayed 0) | 9eb8f4a4 | `sess_980\|27\|13` in DB |
| Fix #5 | Doctor false-positives on removed providers (tdg/hindsight/mem0/etc) | 5b91ea82 | `✓ tdg provider (legacy/removed)` |
| Fix #1-3 | Review cadence/surface/config (evolution split, event, TOML keys) | 5b91ea82 | Skill nudge fires at iter 10 |
| R2-1 | LLM compressor fully dead (zero callers all binaries) | c394c517 | Compressor attaches when enabled |
| R2-2 | Real token usage never drives compression | c394c517 | `estimate_current_tokens` uses last_reported |
| R2-3 | Memory bifurcation / dead JSON store tool_memories.json | c394c517 | memory_store→MEMORY.md, JSON stays 0 |
| R2-1 follow-up | Config-gate compressor on context_compression | 7960e614 | Skipped when disabled, attached when enabled |
| R3 | Credential pool dead-wired (zero callers) | 1da115a9 | "Attached credential pool, provider: openai, creds: 2" + rotation |
| R4-1 | Empty-response retry ladder | 7960e614 | `WARN Empty assistant response — retrying (1/3)` + recovery |

## Key Architectural Divergences vs hermes-agent

1. **Two agent stacks**: hermes has one agent loop; operant has OperantAgent (CLI run path) + RuntimeAgent (operant-runtime, dead-linked in CLI, used by gateway). RuntimeAgent (loop_detector, context_analyzer) is never invoked by `operant run`.

2. **Credential pool**: hermes lazy-loads pool at runtime (`load_pool`); operant had it config-wired but never attached — FIXED. Now mirrors hermes `_recover_with_credential_pool`.

3. **Empty responses**: hermes retries empty turns (`conversation_loop.py`); operant accepted as final — FIXED (R4-1).

4. **Context compression**: hermes `ContextEngine` is a per-turn citizen tracking real API token usage, with LLM summarization; operant had the compressor fully dead — FIXED (R2-1/R2-2).

5. **Memory**: hermes has one memory tool writing to injected MEMORY.md/USER.md; operant had 3-layer decoupled store (naive JSON tools, Builtin provider, agentmemory server) — memory tools now route through the injected MemoryManager (R2-3).

## Known/Deferred (YAGNI-flagged)

- `session_events` table: `record_event` has zero runtime callers, never read by CLI. Dead scaffolding. Removal skipped pending direction.
- `api_call_count` column: never populated (hermes tracks per-accumulated-run; operant's budget resets per-run). YAGNI skip.

## Live Verification Summary (2026-08-06)

- Version: `operant 0.1.4` deployed at `~/.cargo/bin/operant`
- Tool dispatch: 10-20 file creations per run, all landed
- R2-3: memory_store → MEMORY.md (2 hits), tool_memories.json stays empty (0)
- Bug #1: `sess_980|27|13`, `sess_5bb|41|20` in sessions table
- Fix #5: doctor shows `✓ tdg provider (legacy/removed)` with correct guidance
- R4-1: retry ladder fired `WARN Empty assistant response — retrying (1/3)` on real empty turn, task recovered
- R2-1: compressor attaches when `context_compression=true`, skips when `false`
- Skill nudge: fires at iteration 10 (`Skill nudge triggered — spawning background review, iters: 10, interval: 10`)
- ACP server: boots, listens on stdio, shuts down gracefully
- Sessions list: populated counts (41, 36, 10, 54, etc.)
- Doctor: `Found 2 issue(s)` — missing API keys + uninitialized skills hub (expected on fresh config)
