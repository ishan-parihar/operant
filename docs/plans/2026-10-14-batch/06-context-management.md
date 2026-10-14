# 06 — Context management modeled on magic-context, with memory through memory-wire

## Requirement (user, 2026-10-14)

"Implement context-management like magic-context without the memory tools, because we
already have memory-wire. Integrate that memory aspect through the memory-wire."
Standing constraint: no functional regressions in the agent core; it must stay robust for
long-horizon tasks.

## Reference: magic-context (`parent-projects/magic-context/ARCHITECTURE.md`)

Core ideas, ranked by value for operant:

1. **Cache-stable prompt layout.** Frozen `system` + `m[0]` (a frozen baseline of history
   and memories) + `m[1]` (a volatile delta) at the head, then the raw conversation tail.
   Most passes replay all three byte-identically (`SOFT+`), so the whole prefix stays
   cached.
2. **One bust permission per pass.** Every mutation lane (drops, heuristics, m[1]
   refresh, decay re-tier) applies *only* on a pass that is already busting the cache:
   model change, system change, idle beyond the cache TTL, the ≥85% force band, or an
   explicit flush. Deferred work rides the next bust and never causes its own
   (invariants 1–4 in the protected section).
3. **Historian.** Background summarization of older raw history into *compartments*, each
   with four detail tiers, an importance score and facts. A deterministic renderer picks
   one tier per compartment from age, importance and budget. It never runs on the turn
   path, and publishing never forces a bust.
4. **Tags and `ctx_reduce`/`ctx_expand`.** Each part is tagged `§N§`. The agent marks parts
   it no longer needs; drops land on the next bust as the single fixed placeholder
   `[dropped §N§]`. `ctx_expand` recovers the raw original.
5. **Pressure.** A force band at about 85% applies queued work. At 95% with nothing left to
   fold, the turn is refused rather than sent over the limit. Provider limits learned from
   overflow errors are persisted.
6. Memory tools (`ctx_memory`, `ctx_note`, `ctx_search`) and the dreamer are **out of
   scope**, per the user. Memory-wire covers that role.

## What operant already has (don't rebuild)

| magic-context | operant | Where |
|---|---|---|
| raw history + recovery | LCM lossless DAG + `lcm_recall`/`lcm_recent`/`lcm_load_session`/`lcm_vector_recall` tools | `context/lcm.rs` |
| decay tiers | `decay_render(messages, h50, decay)` (by age only) | `context_management.rs:809` |
| eviction | `evict_to_budget`, `safe_compaction_cutoff`, `tool_group_starts` (never splits tool pairs) | `:688`, `:210`, `:164` |
| summarization | `llm_compressor` (synchronous, rung 3 of the preflight ladder) | `agent/llm_compressor.rs`, `run.rs:2426-2600` |
| continuous fold | `micro_compact` at 40%: folds the oldest exchange into a ≤240-char marker **in the store** (lossy in the store; LCM keeps the raw) | `:546-650`, `run.rs:2383-2415` |
| cache guard | `prompt_cache_guard` (verifies the frozen system prefix is unchanged by the ladder) | `:650` |
| frozen/volatile split | system prefix frozen; a "volatile suffix" system message | `run.rs:2280-2370` |
| memory | `MemoryProvider` trait; in-process memory-wire provider (`provider = "memory_wire"`) | `memory_provider.rs:421`, `memory_wire.rs` |
| aux model slot | `[auxiliary] compression` | `config.rs:1523` |

## Defects found while planning (fix in phase 0)

1. **The volatile suffix sits before the history**, so every change busts the cached
   history. In `build_messages` (`run.rs:2310-2370`), memory prefetch (`<memory_context>`,
   recomputed per turn from the last user message), `<long_term_memory>`, the kernel
   harness block and `<workspace_context>` go into a system message *between* the frozen
   prefix and the conversation. Any per-turn change there invalidates the provider cache
   for the entire conversation that follows. The code's comment admits it ("only the
   volatile suffix + conversation history are re-processed each turn"). On long sessions
   this is most of the cost and latency.
2. **The preflight ladder inserts a user message at index 0**
   (`[memory context before compaction]`, `run.rs` right after the ladder's info log): a
   user-role message *ahead of the system prompt*, and a guaranteed full-prefix bust.
3. **`micro_compact` fires whenever the estimate crosses 40%**, on any turn, so each fold
   busts the cache at the fold point. That is the "own bust" magic-context forbids.

## Phases (each is one or more iterations; all mutations behind the flag except P0)

`[context] engine = "classic" | "magic"` (default `classic`). P0 applies to both engines,
because it fixes defects.

### P0: cache-stable layout and telemetry (low risk, ships first)

- Move every per-turn volatile block to the **tail**. Render it as one `system` message
  placed immediately *before the latest user message* of the current turn, or appended to
  it as a `<turn_context>` block if a provider rejects mid-conversation system messages
  (Anthropic: convert to a user-content prefix). The head becomes `system(frozen)` +
  history, byte-stable across turns. Within a turn (tool iterations), the tail block must
  be computed once per turn and reused verbatim.
- Ladder pre-compress insights: insert them after the system prefix, inside the region
  being summarized, never at index 0.
- Telemetry: record `cached_prompt_tokens` per request (OpenAI-compatible:
  `usage.prompt_tokens_details.cached_tokens`; Anthropic: `cache_read_input_tokens`,
  already parsed at `clients/anthropic.rs:563`) into session stats. Show the hit ratio in
  `/context` and the F12 overlay. Log `prefix_bytes_stable` (the length of the common
  prefix with the previous request's serialized messages).
- Tests:
  - two consecutive turns with different prefetch results: the serialized request prefix
    up to the previous turn's last message is byte-identical;
  - the ladder never produces a non-system message before the first system message.

### P1: bust-permission scheduler (both engines)

- Add `BustPermission` computed once per request in `build_messages`:
  - `Natural(reason)` for a model/provider change, a system-prompt hash change, or idle
    longer than `[client] prompt_cache_ttl` since the last response;
  - `Force` when the estimate is ≥85% of the window;
  - `Explicit` for `/compact` and `/ctx-flush`;
  - `None` otherwise.
- `micro_compact`, decay re-tiering, historical tool-result re-trimming and queued drops
  run **only** when permission ≠ `None`. Under `None`, the request is the previous request
  plus appended messages, with no rewrites.
- ≥95% with nothing foldable: return a typed `Error::ContextExhausted`. The TUI offers
  `/compact`; the gateway auto-compacts and retries once. This replaces a provider 400.
- Persist learned limits: on an overflow 400, record `(provider, model) → limit` in the
  DB and use it as `context_window` next time (`error_classifier.rs` already detects
  overflow).
- Tests: a 200-turn mock session below 85% shows 0 rewrites. Crossing 85% causes exactly
  one rewrite, and the following requests are append-only again.

### P2: tags, `ctx_reduce` and `ctx_expand` (magic engine)

- Tag ids: monotonically increasing per session, assigned when a message (or a tool
  result) enters the store. Persist them in a new DB table `context_tags(session_id, tag,
  conv_position, content_hash, kind, state)`, where state is `live|queued|dropped`. The
  model sees tags as a short prefix on tool results and assistant messages: `§17§ `. Keep
  it compact, because every byte costs.
- Tool `ctx_reduce { tags: [u64], reason: String }`: validates the tags (never the last
  user message or the open tool pair) and queues them. It returns "queued; drops apply at
  the next context refresh".
- Tool `ctx_expand { tag }`: returns the raw original, from LCM or from the tag row's
  content hash.
- On a pass with `BustPermission ≠ None`, queued tags are replaced by exactly
  `[dropped §N§]`, a pure function of N. Never derive placeholder bytes from content.
- Nudge: at ≥70%, append (in the tail block) one line listing the largest live tags
  ("§12 read_file 18k tok, §15 …") so the model knows what it can reduce.
- These are not memory tools; they manage the window. The user only excluded memory
  tools.

### P3: historian and m[0]/m[1] head (magic engine)

- **Trigger** (after each turn, off the turn path, in a `tokio::spawn`): the raw history
  outside the protected tail exceeds `historian_chunk_tokens` (default 12k), or pressure
  ≥ 60%.
- **Boundary:** reuse `safe_compaction_cutoff`/`tool_group_starts`. Keep the protected
  tail (last 8 messages, or the current turn, whichever is larger).
- **Produce:** call the `[auxiliary] compression` model (fall back to the main model) with
  a bounded prompt: the chunk, the last 2 compartments and the session goal
  (`compaction_goal`, `context_management.rs:265`). The output is JSON:
  `{start, end, title, tiers:{full, detailed, brief, oneline}, importance, facts[]}`.
  Validate the ordinals; discard the last compartment when it has no lookahead.
- **Publish:** store it in the DB `context_compartments`, queue drops for the summarized
  raw range (they ride the next bust), and send facts to memory-wire (P4). Never force a
  bust.
- **Render:** `m[0]` holds the compartments rendered with one tier each, chosen by
  `decay_curve(age, importance, budget)`, plus baseline memories, frozen at
  materialization. `m[1]` holds new compartments at full tier plus new memories since
  `m[0]`'s watermark. Both are synthetic user messages placed right after the frozen
  system message.
  - **HARD fold**, where m[0] re-materializes and m[1] resets: model/provider change,
    system hash change, idle > TTL, m[1] > 20% of the history budget, or `Force`.
  - Otherwise m[0] bytes never change.
- Replaces rung 3 (synchronous LLM summarize) when `engine = magic`. Rungs 1, 2 and 4
  remain as the emergency path.
- Tests: concurrency, where the historian runs while a turn appends; the published
  compartments are valid and the turn's request is unaffected (disjoint-snapshot
  invariant). Determinism: same inputs give the same m[0] bytes.

### P4: memory through memory-wire (both engines; no new memory tools)

- Historian facts go to `MemoryWireProvider::on_memory_write` / `retain_tagged`, with tags
  `project:<repo>`, `session:<id>` and `category:<kind>`.
- Memory reaches the prompt in exactly two places:
  - the `m[0]` baseline at materialization, and the `m[1]` delta by memory-id watermark
    (magic engine);
  - the bounded per-turn recall in the tail block (P0), for both engines.
- The `<long_term_memory>` block and the `builtin` provider are removed, and memory-wire
  is the only memory engine (plan 11 phase C3). Facts are never injected twice.
- **Decided (user, 2026-10-14): in-process memory-wire only**, compiled from upstream
  `main` on every build (plan 11 phase A). There is no HTTP/daemon backend option. Bank
  selection (user / project / org / emp) comes from plan 11 phase E's `bank_for`.

### P5: long-horizon soak and rollout

- `tests/context_soak.rs` (operant-core, `#[ignore]` by default, run in CI nightly): a mock
  client records every request across a scripted 300-iteration session. Tool outputs range
  from 200 B to 200 KB; there are 3 user goals and 2 model changes. Assert:
  - no request is over 95% of the window;
  - tool pairs are always intact (`message_safety` repairs == 0);
  - the bust count is at most expected (computed from the common-prefix length between
    consecutive requests);
  - the first user goal stays reachable (in m[0] or via `ctx_expand`);
  - the historian never blocks a turn (turn wall time is unaffected by historian
    latency, using a mock that sleeps 5 s).
- Dogfood `engine = "magic"` for a week, then flip the default. Keep `classic` for one
  release.

## Order

P0 (two iterations: layout, telemetry), then P1, then P2, then P3, then P4, then P5.
P0 and P1 alone remove most of the cost; ship and measure them before P2/P3.

## Docs

`operant.example.toml` (`[context]` section, new memory fields), `README`
(context-management section), `CHANGELOG`, `docs/` design note linking to magic-context's
ARCHITECTURE for the invariants.
