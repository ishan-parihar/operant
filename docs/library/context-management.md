# Context management — eviction, decay, and the compaction ladder

Owns: how operant keeps a long conversation inside the context window —
tiered eviction, the decay curve, the four-stage compaction ladder, the
LCM lossless engine, and prompt-cache stability.
Not for: what gets injected into a turn ([context-injection.md](context-injection.md)),
the memory store ([memory.md](memory.md)).

Source: `crates/operant-core/src/context_management.rs`,
`crates/operant-core/src/context/` (mod, lcm, rollup, adaptive, embedder),
`crates/operant-core/src/agent/llm_compressor.rs`,
`crates/operant-core/src/session/autocompact.rs`,
`crates/operant-core/src/tools/lcm_tools.rs`.

## Why this exists

`build_messages` concatenates system + memory + skills + context_files +
the full history every iteration — any long-running session eventually
exceeds the window. Operant ports the techniques of
`cortexkit/magic-context` into native Rust, then extends them with a
lossless engine (hermes-lcm parity) and a scheduled autocompact.

## The three mechanisms (and why there are three)

| # | Mechanism | When it runs | Lossy? |
|---|---|---|---|
| 1 | Tiered eviction + decay (`context_management.rs`) | during a request, when the assembled prompt approaches the window | yes |
| 2 | LCM DAG engine (`context/lcm.rs`) | per-call assembly, opt-in via `agent.context_engine = "lcm"` | **no** (append-only DAG) |
| 3 | Scheduled autocompact (`session/autocompact.rs`) | after a turn completes, on schedule | yes, with a floor |

They are deliberately three: the reactive path (1) runs mid-request where
latency matters, the LCM path (2) replaces lossy eviction wholesale, and
the scheduled path (3) compacts between turns where a compaction floor
protects recent context. `session/autocompact.rs`'s module doc spells out
why a third mechanism was not folded into the second.

## 1. Tiered eviction + decay curve

When the assembled prompt approaches the window, content is evicted
oldest-first **by tier**:

- **T3 (tool results)** evicted first — bulky, re-derivable.
- **T2 (reasoning / thinking blocks)** second.
- **T1 (user/assistant messages)** last.

The **decay curve** scores what survives: H = H50·2^((I-50)/D)/max(p,0.10)
(iter-38), where importance decays with age but recency keeps a floor.
The **recency reserve** scales with the budget: `budget/4096` clamped to
[6, 50] (iter-43) — a bigger window keeps more fresh turns unconditionally.

### The compaction ladder (2026-10-25 audit: one ladder, four rungs)

When eviction alone is not enough, a preflight ladder runs in order, each
rung cheaper than the one below it:

1. **Decay** — deterministic evict-to-budget (above).
2. **Micro-compaction** (`agent.micro_compaction`, env
   `OPERANT_MICRO_COMPACTION`) — inline trimming of tool-result bulk.
3. **Semantic compaction** (`agent.context_semantic_compaction`, default
   off) — goal-similarity group selection via the embedding model
   (`context_lcm_embedding_model`, `"local:hash"` = zero-dependency):
   keep goal-relevant exchanges over the oldest-prefix recency cut.
4. **LLM summarize** — `agent/llm_compressor.rs` calls the auxiliary
   compression model (`[auxiliary_models.compression] model`; the default
   is no longer hardcoded — G6) to produce a structured summary of the
   middle turns, bounded by `context_compression_summary_max_chars`.

The ladder emits `CompactionStarted` / `CompactionCompleted` events the
TUI already renders (RT-182). The semantic stage's degrade path is its own
recency fallback — a failed embedding never kills the compaction.

## 2. The LCM lossless engine (hermes-lcm parity)

`agent.context_engine = "lcm"` swaps the whole assembly for an
**append-only SQLite DAG**: every message is stored verbatim
(losslessness), and `assemble()` keeps the D0 fresh tail (most recent
messages within the tail budget) and compacts everything older into a
placeholder block. Temporal **rollups** (`context/rollup.rs`) summarize DAG
content below the D0 frontier into day/week/month summaries stored in the
`lcm_rollups` table — the summarizer is injected so the core is testable
with a fake.

Two agent-facing tools ship with the engine (`tools/lcm_tools.rs`):
`lcm_recall` (FTS5 phrase search over the DAG, session-scoped by default,
global when the session arg is omitted — returns verbatim nodes, so
nothing lost to compaction is truly gone) and `lcm_stats`.

`context/adaptive.rs` is the bounded port of hermes's adaptive
evidence-gated retrieval: multi-round recall that terminates once enough
distinct evidence is gathered. `context/embedder.rs` collapses the
embedding-provider contract to one `embed` call (`"local:hash"` for tests
and zero-dependency setups).

## 3. Scheduled autocompact (`session/autocompact.rs`)

After a turn completes, a scheduled compaction runs with a **compaction
floor** (§4.5): recent context is protected by a floor the compaction
cannot cut past. The operator CLI surface is `operant context status /
recall / sessions` (`crates/operant-cli/src/cmd_context.rs`).

## Prompt-cache stability (the constraint everything respects)

The provider prompt cache only helps when the prefix is byte-identical
between calls. Two invariants hold it:

- The **system prompt is a frozen prefix** (base + skills guidance);
  anything that changes it (skills load, ponytail posture change,
  context-file change) invalidates the cache — a documented consequence,
  not a defect.
- All volatile per-turn data (memory recall, workspace facts, harness
  state, MoA advice) rides the TAIL — a `<turn_context>` block on the
  latest user message — never the prefix (iter-911).

Anthropic `cache_control` breakpoints are set on the system prompt
(iter-54); OpenRouter-compatible endpoints get the `prompt_cache_ttl`
config (`"5m"` default, `"1h"` available).

## Guardrail: the tool-result share cap

One tool result may occupy at most `agent.max_tool_result_share` (default
0.05 = 5%) of the context window before its bulk is withheld behind an
explicit marker stating the kept head, token count, price, and removed
tokens (iter-357). The withheld part is NOT stored anywhere the agent can
read back — the marker tells the model to re-run the tool with a narrower
slice if it needs the rest: a silently shortened result is worse than a
failed one.

## Intention

Every mechanism here exists because a silent truncation produced a wrong
answer somewhere: FTS5 special-character escaping (iter-46) because a
search silently missed rows; the marker instead of silent withholding
because the model could not tell it was not reading what the tool returned;
the compaction floor because scheduled compaction cut live context. The
ladder's order (cheap deterministic first, LLM summarize last) is cost
discipline: the LLM rung only fires when the cheap rungs demonstrably
cannot fit the window.
