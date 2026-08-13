# hermes-lcm → operant: Lossless Context Management Integration Design

Status: **Design (Phase 0: P0–P2 planned)**
Source analyzed: `https://github.com/stephenschoettler/hermes-lcm` (978★, cloned at
`/home/ishanp/Documents/GitHub/CLONED-REPOS/hermes-lcm`), commit at time of writing.

---

## 1. What hermes-lcm actually is (from source)

hermes-lcm is a **ContextEngine plugin** for hermes-agent, not a standalone
library. It installs as a hermes-agent plugin (via `plugins.enabled` +
`context.engine: lcm` in the hermes config) and registers:

- **One `pre_llm_call` hook** — the canonical, deterministic injection point.
  Hermes calls it before every LLM call; LCM returns the *final message list*
  to send. This is how it "never loses a message."
- **A set of recall tools** (`lcm_recall*`) registered through the plugin
  schema, so the agent can pull past context on demand.

### Core modules (verified from source)

| Module | Responsibility |
|---|---|
| `dag.py` | `SummaryNode` + `SummaryDAG` — the immutable append-only DAG over the conversation. Every message = a node; edges = parent/sibling ordering. FTS spec + sorted search helpers. |
| `engine.py` | `LCMEngine(CompactionMixin, ResetStateMixin, ReconcileMixin, AuxiliarySessionMixin, PlaceholderLedgerMixin, BypassMixin, ContextEngine)` — message ingest, session lifecycle, rollup maintenance scheduler, the `pre_llm_call` assemble step. |
| `store.py` / `rollup_store.py` / `query_view_store.py` / `assertion_store.py` / `trajectory_store.py` | SQLite stores; `db_bootstrap.py` + `sqlite_util.py` for schema/migrations. |
| `compaction.py` | `CompactionMixin` — **D0 leaves**: the most recent messages are ALWAYS kept verbatim (never summarized away). Compaction only rolls up nodes *below* the frontier. |
| `rollup_builder.py` | `build_day` / `build_week` / `build_month` — LLM-summarized rollups at increasing granularity; `run_rollup_maintenance` for background upkeep; stale/deleted-node invalidation. |
| `adaptive_retrieval.py` | `AdaptiveRetrievalRegistry`, `RetrievalRound`/`RetrievalState`, `EvidenceRequirement`, `ExactEvidence`, `SearchLead` — multi-round, evidence-gated recall instead of one-shot search. |
| `retrieval_core.py` / `vector_store.py` | Hybrid retrieval: SQLite FTS + optional vector embeddings, `_reset_vector_store_pool`. |
| `assertion_extraction.py` / `assertion_state.py` | Extract durable "assertions" (facts) from messages; rebuilt on demand. |
| `message_analysis.py` / `chunking.py` / `tokens.py` | Message-level analysis (importance scoring, regex patterns, token budgeting). |
| `config.py` / `command.py` | `LCMConfig` (SQLite path via `LCM_DATABASE_PATH`, default `HERMES_HOME/lcm.db`), CLI commands for inspection. |

### The key design property: **losslessness**

Instead of *evicting* old messages when the window fills (what most agents do —
and what operant's `context_management::evict_to_budget` currently does), LCM:

1. Keeps the **fresh tail** (D0) verbatim.
2. **Rolls up** everything below into LLM summaries (day → week → month).
3. Keeps the DAG + full message text in SQLite forever, so **nothing is
   deleted** — every token is recoverable via `lcm_recall`.

That is the entire difference from a truncating compressor: the summary is a
*lens*, not a replacement.

---

## 2. Current operant context pipeline (integration points)

| Operant site | File:line | What it does today | LCM replaces/supplements |
|---|---|---|---|
| Message build | `agent/mod.rs:1994` `build_messages()` | Assembles the per-iteration message list | **The `pre_llm_call` hook site.** `build_messages()` is called every iteration — this is where LCM's assemble step runs. |
| Budget eviction | `agent/mod.rs:2138` `evict_to_budget(...)` | **Drops** old messages when over budget (lossy) | Replaced by LCM rollup + D0 tail. Native eviction stays as the *fallback* when LCM is off/disabled. |
| Decay render | `agent/mod.rs:2129` `decay_render(...)` | Soft-attens old messages instead of deleting | Complementary (can stay; applies to the rollup nodes). |
| LLM compressor | `agent/llm_compressor.rs` `LlmCompressor::compress()` | Single-level LLM summarization of overflow | Rollup_builder generalizes this: day/week/month levels with invalidation. |
| Overflow path | `agent/mod.rs:2684` `compress_context_overflow()` | Triggered when the window overflows | LCM compaction replaces this branch when `context.engine = "lcm"`. |
| Token estimates | `context_management.rs` `estimate_tokens` / `estimate_message_tokens` | Used for budget decisions | Reused as the rollup budget oracle. |
| Sessions DB | `database.rs` (SQLite) | Session + message persistence | LCM's own `lcm.db` (SQLite) holds the DAG; native DB unchanged. |

**No new architecture needed** — LCM slots in exactly where `LlmCompressor`
and `context_management` already hook. That is the correct design: a
`ContextEngine` trait + one implementation, chosen by config.

---

## 3. Target architecture in operant

```
operant-core/src/context/                      (new module tree)
├── mod.rs                 # ContextEngine trait + factory (config.agent.context_engine)
├── lcm/
│   ├── mod.rs             # LcmContextEngine : ContextEngine
│   ├── dag.rs             # SummaryNode, SummaryDag (SQLite-backed, append-only)
│   ├── store.rs           # lcm.db schema bootstrap + migrations (dbc)
│   ├── compaction.rs      # D0-fresh-tail frontier, never summarize the tail
│   ├── rollup.rs          # build_day/week/month via the agent's LLM client
│   ├── recall.rs          # hybrid FTS(+vector) search → evidence packs
│   └── config.rs          # LcmConfig { db_path, tail_tokens, rollup_schedule, ... }
```

### The `ContextEngine` trait (mirrors hermes `ContextEngine`)

```rust
#[async_trait]
pub trait ContextEngine: Send + Sync {
    /// Ingest a completed turn (messages + tool results) into the DAG.
    async fn ingest_turn(&self, session_id: &str, turn: &[Message]) -> Result<()>;
    /// Assemble the message list for the next LLM call.
    /// This is the pre_llm_call hook: returns the FINAL list to send.
    async fn assemble(
        &self,
        session_id: &str,
        base: Vec<Message>,
        budget_tokens: usize,
    ) -> Result<Vec<Message>>;
    /// On-demand recall tool backend.
    async fn recall(&self, query: &str, opts: RecallOptions) -> Result<RecallResult>;
}
```

`OperantAgent` gains `with_context_engine(engine: Arc<dyn ContextEngine>)`;
`build_messages()` calls `engine.assemble(...)` as its final step when set.
`LlmCompressor` becomes the *fallback* `ContextEngine` implementation when
`context.engine = "compact"` (current behavior, unchanged default for now).

### DAG schema (SQLite, `lcm.db`)

```sql
CREATE TABLE nodes (
  id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  kind TEXT NOT NULL,            -- 'message' | 'rollup_day' | 'rollup_week' | 'rollup_month' | 'assertion'
  role TEXT,                     -- user/assistant/tool
  content TEXT NOT NULL,         -- full verbatim content (lossless)
  parent_id INTEGER,             -- rollup parent edge
  scope TEXT,                    -- 'main' | 'aux:<id>'
  importance REAL,               -- from message_analysis
  created_at INTEGER NOT NULL
);
CREATE VIRTUAL TABLE nodes_fts USING fts5(content, session_id);
```

Rollups: `rollup_day(scope, day)` etc. aggregate *below the D0 frontier* only.
`assertions` table holds durable facts extracted from messages.

---

## 4. Config surface (hermes parity)

```toml
[agent]
context_engine = "compact"        # "compact" (current) | "lcm"
context_lcm_db = "~/.operant/lcm.db"
context_lcm_tail_tokens = 12000   # D0 fresh-tail budget (kept verbatim)
context_lcm_rollup = true         # background day/week/month rollups
context_lcm_auto_recall = true    # inject recalled evidence when relevant
```

Matches hermes-lcm's `context.engine: lcm` + `LCM_DATABASE_PATH` exactly.

---

## 5. Tool surface (agent-facing)

| Tool | Backend | Purpose |
|---|---|---|
| `lcm_recall` | `recall.rs` (hybrid FTS) | Search the full DAG, return node snippets + parent rollup chain. |
| `lcm_recall_round` | `adaptive_retrieval.rs` port | Multi-round evidence-gated recall: state persists across rounds, returns `ExactEvidence` + `SearchLead`. |
| `lcm_assert` | `assertion_state.rs` port | Store/query durable assertions (facts) extracted from the conversation. |
| `lcm_status` | `store.rs` | DAG size, tail/frontier state, last rollup, db path. |

Registered only when `context.engine = "lcm"` (schema parity with how LCM
registers tools only when enabled — hermes README: "tool list on stock
installs" stays clean).

---

## 6. Phased implementation plan

### P0 — Trait + config + DAG store (foundation, ~1 crate-week)
- `ContextEngine` trait in `operant-core/src/context/mod.rs`.
- `LcmContextEngine::ingest_turn` + `assemble` with the D0 tail only (no
  rollups yet): `assemble` = tail verbatim + summarized-prefix placeholder.
- Config fields (`context_engine`, `context_lcm_*`) + serde defaults +
  `operant.example.toml`.
- SQLite store bootstrap (`dbc` or `rusqlite` — match existing `database.rs`
  choice) with the schema above.
- Unit tests: ingest → assemble budget math; D0 never summarized; FTS insert.

### P1 — Rollups (replace lossy eviction when LCM on) ✅
- `rollup.rs` `build_rollup` with injectable async `Summarizer` (hermes
  `Summarizer = Callable` parity) — day/week/month UTC windows, deterministic
  truncation (200 nodes / 24k chars, char-boundary-safe), idempotent
  `ON CONFLICT` upsert.
- `lcm_rollups` table (hermes `rollup_store.py` shape): session, period_kind,
  period_start, summary, source_count, created_at; PK dedups refreshes.
- CLI: `operant context rollup <session> --period day|week|month --date YYYY-MM-DD`
  (real model client via `client_config`) + `operant context rollups <session>`
  listing + `context status` shows the global rollup count.
- **Compaction wiring**: `assemble()` now injects stored rollups into the
  over-budget context (block after the placeholder, token-budget accounted,
  `context_lcm_rollups_inject` flag, default on). Verified by real-agent
  integration test: stored rollup → `build_messages` carries the summary.
- **On-demand maintenance pass** ✅: `run_rollup_maintenance` (hermes
  `run_rollup_maintenance` dedup semantics) scans DAG sessions and builds
  missing day/week/month rollups over a lookback window, skipping periods
  that already have a rollup; surfaced as `operant context rollup-maintenance
  [--session S] [--lookback-days N]`. Verified live: 2 sessions scanned →
  day/week/month built with correct anchors, re-run fully idempotent.
- **Background maintenance scheduler** ✅: `spawn_rollup_maintenance`
  (hermes `_RollupMaintenanceScheduler` parity, bounded) — one immediate
  pass, then every `context_lcm_rollup_interval_minutes` (0 = disabled,
  default); empty windows skip the summarizer, a bad pass is logged and
  swallowed. Wired in `build_context_engine` with the shared
  `rollup_summarize` LLM helper (also used by the CLI, so prompts can never
  drift). Live-verified: deleting a stored month rollup, a plain `operant
  run` with the scheduler enabled rebuilt it automatically (2 → 3).
- **Deferred (YAGNI until a concrete need):** build leases/generations —
  the lossless DAG already keeps everything; rollups are injected when
  present, built on demand via CLI, the maintenance pass, or the scheduler.

### P2 — Recall tools + assertions ✅
- `lcm_recall` / `lcm_stats` registered through the existing `builtin.rs`
  gate (`config.agent.context_engine`).
- `lcm_assert` ✅ — durable assertion store (`lcm_assertions` table;
  hermes `assertion_store.py` parity, conflict-preserving). Save/query a
  fact; query resolves the active state (latest per unique object) and
  reports contradictions (distinct active objects per predicate).
- `lcm_recall_round` ✅ — multi-round evidence-gated recall (`adaptive.rs`;
  hermes `adaptive_retrieval.py` lightweight parity): in-memory registry
  keyed by `retrieval_id`, TTL-purged, MAX 3 rounds; each round returns
  exact verbatim evidence (cumulative across rounds) + search leads, and
  `complete` flips once the evidence requirement is met.
- `lcm_assert action="extract"` ✅ — **LLM-driven assertion extraction**
  (hermes `assertion_extraction.py` `ModelAssertionExtractor` parity,
  bounded): `assertion_extract.rs` (`AssertionExtractor` trait +
  `LlmAssertionExtractor` over the shared OpenAI-compatible client),
  `recent_message_nodes` engine scan, tolerant payload decode
  (markdown fences / bare array / `{"assertions":[...]}` envelope /
  JSON-salvage out of prose), canonicalized keys matching explicit
  `save` keys, opt-in gate `agent.context_lcm_assertion_extraction`.
  Live-verified end-to-end on the real DAG: a fresh process mined 3
  durable facts (`project` → `stack` = "Rust and SQLite",
  `deploy_cadence` = "biweekly on Wednesdays", `preferred_editor` =
  "Neovim") from prior-session nodes and a follow-up query returned
  them in active state with 0 contradictions. Live testing surfaced two
  production fixes now in place: (1) reasoning models burn their token
  budget on `reasoning_content` first, so the extractor uses 4096
  max_tokens (hermes `max_tokens=4000` parity) with a
  `reasoning_content` fallback and a bounded single JSON-only retry;
  (2) `ToolRegistry` gained per-tool timeout overrides
  (`set_tool_timeout`), and `lcm_assert` carries a 180s window so the
  LLM call isn't killed by the generic 30s tool timeout.
- Deferred (YAGNI): cross-session persisted adaptive state — the DAG +
  explicit save/query/extract surface already covers the durable-facts need.

### P3 — Auto-recall + vector backend (optional, stretch)
- ✅ **Implemented**: on `assemble`, one bounded retrieval round against the
  latest user message injects top evidence as a system block (hermes
  "pre-answer evidence"). Config: `context_lcm_auto_recall` (default true),
  `context_lcm_auto_recall_limit` (3), `context_lcm_auto_recall_max_chars`
  (4000). OR-term FTS query (stopword + FTS5-reserved-keyword filtered),
  majority-overlap dedup vs visible context, evidence token budget reserved
  during compaction. 13 unit tests + real-agent integration test + live E2E.
- ✅ **Vector recall** (`embedder.rs` + `lcm_vector_recall` tool): hermes
  `embedding_provider.py`/`vector_store.py` parity, bounded. `Embedder`
  trait + `OpenAIEmbedder` over the client's new `/embeddings` method;
  `LcmContextEngine::vector_recall` embeds the query and the candidate pool
  (most recent 200 message nodes), caches vectors in `lcm_embeddings`
  (keyed by model, so a model change re-embeds), and returns top-N by
  cosine similarity — surfacing reworded matches with no exact word overlap.
  Registered only when `agent.context_lcm_embedding_model` is set (tool
  surface stays clean otherwise). Config: `context_lcm_embedding_model`
  and `context_lcm_embedding_base_url` (defaults to the chat provider's
  base URL; point it at any OpenAI-compatible `/embeddings` endpoint,
  e.g. `http://localhost:11434/v1` for local Ollama). Tested with a
  deterministic hash-trick mock embedder (ranking + cache-hit behavior).
  **Live E2E verified** against a local Ollama `all-minilm:latest`
  (`/v1/embeddings` returns proper vectors): a phrase stated in process A
  was retrieved by a fresh process B via the reworded query `"special
  numeric startup value"` (no exact-word overlap) — prior-session nodes
  ranked #1 (node 147, sim 0.327) and #2–3 (nodes 139/141) with
  `lcm_embeddings` caching 54 vectors under `all-minilm:latest`. The
  free-tier opencode provider itself has no `/embeddings` endpoint (HTML
  fallback),  which is exactly why the base URL is configurable.
- ✅ **Comprehensive single-session E2E (9/9 PASS)** — one agentic run
  exercised the complete AFT + LCM surface together against the live
  Ollama config: `lcm_stats` (engine=lcm) · `lcm_assert` save id=5 →
  query active status=verified, 0 contradictions · `lcm_recall`
  (5 hits) · `lcm_recall_round` (complete=true) · `lcm_vector_recall`
  (top sim 0.2314, all-minilm) · `aft_status` (degraded=false) ·
  `aft_read` (35 006 bytes) · `aft_outline` (30+ symbols). AFT binary
  auto-resolved from cache `~/.operant/aft/aft-v0.49.4/aft`; scheduler
  active (minutes=60); DB after run: 63 nodes / 7 rollups / 5
  assertions / 58 embedding vectors.
- ✅ **Comprehensive E2E v2 including the extract action (9/9 PASS)** —
  one agentic run over the same live config with
  `context_lcm_assertion_extraction = true`: `lcm_stats` (engine=lcm,
  dag_nodes=84) · `lcm_assert` save (id=17) · **`lcm_assert extract`
  mined 3 facts live** (subject `project` → `stack`/`preferred_editor`/
  `deploy_cadence`) · `lcm_assert` query returned them in active state,
  0 contradictions · `lcm_recall` (5 hits) · `lcm_recall_round`
  (complete=true) · `lcm_vector_recall` (top sim 0.4369) · `aft_status`
  (ready, v0.49.4) · `aft_outline` on a real Rust source (41 symbols
  with line ranges — a `.toml` config file is correctly rejected as a
  non-source extension). DB after run: 95 nodes / 12 rollups / 20
  assertions / 90 embedding vectors; scheduler active (minutes=60).

### P4 — CLI + metrics ✅ (partial)
- `operant context status|sessions|recall <q> [--limit N]` — implemented
  (`cmd_context.rs`). Read-only operator surface over the same DAG file.
- `rollup-maintenance` ✅ — on-demand maintenance pass over all (or one)
  session, building missing day/week/month rollups (hermes `run_rollup_maintenance`
  parity). DAG-stats-in-status-bar remains as stretch — the agent-facing
  `lcm_recall`/`lcm_stats` tools already cover the runtime path; a status-bar
  widget adds UI plumbing with no functional gain yet.

**YAGNI note:** skip the auxiliary-session, placeholder-ledger, and bypass
mixins (hermes-specific multi-session orchestration) until a concrete need
exists; the DAG store's `scope` column already reserves the shape.

---

## 7. Migration / risk

- **Default stays `compact`** — zero behavior change for existing users; LCM
  is opt-in via config (same as hermes).
- **Native eviction stays** as the fallback engine implementation, so
  disabling LCM is a one-line config flip, never a data migration.
- **Losslessness guarantee**: the DAG is append-only and the db is separate
  from `hermes.db`; no destructive ops on existing session data.
- Rollup invalidation must mirror `rollup_builder.mark_stale_for_*` — parent
  summaries are invalidated when a child node is deleted (e.g. `/reset`).

---

## 8. Decisions already made (trajectory)

1. **Trait-first, not crate-first**: `ContextEngine` lives in `operant-core`
   (the existing `llm_compressor`/`context_management` home), matching the
   repo rule "no speculative new crates for minor convenience."
2. **One injection site**: `build_messages()` — identical role to hermes's
   `pre_llm_call`, so parity is auditable.
3. **Opt-in default** preserves the current production behavior until the
   rollup path has live-test evidence (same bar as the AFT integration).
