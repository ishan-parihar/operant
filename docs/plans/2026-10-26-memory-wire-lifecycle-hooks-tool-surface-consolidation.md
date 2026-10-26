# Plan: memory-wire lifecycle hooks + tool-surface consolidation

Date: 2026-10-26 · Status: frozen (dated plan) · Owner: operator agent
Source audit: `docs/audit/2026-10-25-memory-sourcehound-integration-gaps.md`
Open rows this plan executes: **RT-191** (missing hooks), **RT-192** (five
duplicate memory tools), **RT-193** (score discarded). Explicitly OUT of
scope: **RT-190** (bank topology — operator decision, recorded separately)
and **RT-181-peer** (on_pre_compress is the peer's row; this plan
implements it as ONE of the four hooks but the row closes under the peer's
number with a cross-ref from RT-191).

Verified facts this plan builds on (all checked in-tree / in the locked
dependency this session):
- `MemoryProvider` trait defaults live at `memory_provider.rs:335-363`
  (`on_turn_start` :335, `on_session_end` :339, `on_session_switch` :343,
  `on_pre_compress` :348, `on_memory_write` :354, `on_delegation` :358,
  `queue_prefetch` :363 — all empty defaults).
- The locked memory-wire rev (fd704c8, from Cargo.lock) exposes
  `MemoryService::retain_doc(bank, content, context, tags, document_id,
  UpdateMode)` at `src/api.rs:521` and its own hook design
  (`src/hooks.rs`: `session_end`/`pre_compact` retain with
  `FLUSH_CHARS=2000` tail-bounded flushes, with tests at hooks.rs:1085-1090
  proving both) — the operant side mirrors those shapes in-process.
- The agent loop already EMITS every hook (run.rs:1619-1663 sync_turn +
  queue_prefetch; run.rs:2735-2758 on_pre_compress insertion site;
  events.rs:351-369 session_end; events.rs:440 delegation).
- The five memory tools: `MemoryStoreTool`/`MemorySearchTool`/
  `MemoryRecallTool` in `tools/memory_tools.rs`, plus the two provider
  twins registered by `memory_provider_tools.rs:66`
  (`register_provider_tools` ← `provider.tool_schemas()`).
- Measured wire cost (audit §5): five tools ≈ 590-650 tokens per request;
  two-tool target ≈ 290; consolidation saves ~300/request.

## Phase 1 — implement the four missing hooks (RT-191 + peer RT-181 half)

One iteration. All in `MemoryWireProvider` (`memory_wire.rs`), in-process,
no serve dependency. Bank = the provider's existing default bank.

1.1 `on_session_end(&self, messages: &[Message])`
     - Fold the conversation's own prose (user/assistant pairs, skip tool
       outputs) into one document; `retain_doc` with
       `document_id = format!("session-{session_id}")`, tags
       `["session", "conversation"]`, `UpdateMode::Replace` (the session-end
       write replaces the running session doc, mirroring hooks.rs
       `session_end`).
     - Tail-bound the content to 2000 chars via the same
       `.skip(len - FLUSH_CHARS)` shape hooks.rs:285 uses — but keep the
       HEAD of each user message (the question is the retrieval key;
       head-anchored truncation is the RT-191 lesson from sync_turn's
       head-drop failure).
     - RED test first: a `MemoryWireProvider` built over a temp-dir store;
       call `on_session_end` with two message pairs; assert a
       `recall_text("session probe")` hit carries both user turns' heads
       and that a second call REPLACES (does not append — same
       document_id, `UpdateMode::Replace`).
1.2 `on_pre_compress(&self, messages: &[Message]) -> String`
     - Same fold+truncate; `retain_doc` with
       `document_id = format!("precompact-{session}")`,
       `UpdateMode::Append` (each compression event adds a bounded gist —
       the middle evictions across one long session are cumulative), tags
       `["session", "pre-compact"]`.
     - Return the `[memory context before compaction]` insert string the
       ladder site (run.rs:2735-2758) already places; today it always
       inserts nothing because the default returns `String::new()`.
     - RED test: evicted-exchange prose (user/assistant pair present in
       `messages`, absent from the post-compaction window) is recallable
       after the hook fires; the returned string is non-empty when any
       pair was folded.
1.3 `on_delegation(&self, task: &str, result: &str)`
     - `retain_doc(document_id = "delegation-{session}-{n}")`, tags
       `["delegation", "task"]`, `UpdateMode::Append`; content = task +
       bounded result head. RED test: delegate → recall by task keyword.
1.4 `queue_prefetch(&self, query: &str)`
     - Mirror of `prefetch` (which already works): store the query in a
       small Mutex<VecDeque> the provider drains on next `prefetch` /
       turn start; bounded (last 8). RED test: queue two, first `prefetch`
       after sees the newest; a third drops the oldest.
1.5 `on_session_switch` / `on_turn_start` stay trait-default EXCEPT one
     line: `on_turn_start` becomes the drain point for `queue_prefetch`
     (1.4) so a queued prefetch actually fires. If the executor does not
     emit `on_turn_start` on the run path (verify: events.rs), the drain
     happens at the top of `prefetch` instead — do not add a new loop
     call site for it.

Acceptance (Phase 1): all four hooks have RED-then-green tests; the live
bank gains a `session-…` document after one real `operant run` conversation
ends (verify via `memory_search` for a phrase from that conversation);
clippy gate + fmt clean; scoped `cargo test -p operant-core --lib -- memory`
green. BUGS.md: RT-191 closed, peer RT-181 cross-referenced as closed by
this work (row edits in the SAME iteration).

## Phase 2 — consolidate five memory tools to two (RT-192)

One iteration. Registry-only change; no engine change.

2.1 `memory_save` + `memory_smart_search` (the provider twins) retire
     from the run-path registry: `register_provider_tools` stops
     registering tool schemas whose names duplicate the built-in trio
     (`memory_save` ≡ `memory_store`, `memory_smart_search` ≡
     `memory_search`). Keep the FUNCTION (other callers may exist —
     verify `operant tools list` + background-review whitelist still see
     the surviving three).
2.2 `memory_recall`'s key recall folds into `memory_search` as an optional
     `key` param (exact-key match path already exists in the provider's
     `recall_text(key)`); `memory_recall` retires. The audit's measured
     target: 5 → 2 tools, ~300 tokens/request saved.
2.3 Update the background-review prompt/whitelist if it names
     `memory_recall` (verify: the review path's tool filter at
     run.rs:446 names the trio — keep the survivors only).
2.4 RED tests: a registry snapshot (get_schemas_all) after registration
     contains exactly `memory_store` + `memory_search` among memory tools;
     `memory_search {query, key}` performs the exact-key recall the old
     `memory_recall` did (port its test).

Acceptance (Phase 2): `operant tools list` shows two memory tools; a live
`operant run` round-trip stores and recalls through the two-tool surface;
wire-schema measurement re-run shows the ~300/request saving (same probe
methodology as the audit); docs updated (README tool table if one exists;
CHANGELOG entry). BUGS.md: RT-192 closed.

Name-retirement is reversible: keep the old names as ALIASES for one
release (a model that calls `memory_recall` gets the folded behavior with
a deprecation note in the result payload) — cheaper than breaking a
pinned prompt elsewhere.

## Phase 3 — carry the fused score (RT-193)

One iteration. Pure plumbing, no behavior change until a gate exists.

3.1 `MemoryWireProvider::format_hits` (memory_wire.rs:252) returns the
     raw `Vec<ScoredMemory>` alongside the formatted text (or the two
     call sites take the scored vec and format at the edge).
3.2 `MemorySearchTool::execute` serializes the REAL `score` per hit
     (replacing the hard-coded `"relevance": 1.0`).
3.3 `prefetch` ranks/caps by score but keeps its text output contract.
3.4 RED tests: a store with a high- and a low-relevance row returns
     ordered, distinct scores through `memory_search`'s JSON; prefetch
     output unchanged (text form).

Acceptance (Phase 3): scores survive to the tool layer; no consumer
changes behavior (the gate remains deferred per memory-wire's doctrine —
RETRIEVAL_EFFICACY_AUDIT §8). BUGS.md: RT-193 closed with the deferral
note intact.

## Sequence + numbering

- Phase 1 FIRST (its own iteration, `feat(iter-N): memory lifecycle hooks`)
  — it grows the corpus, which makes Phase 2/3 verification richer.
- Phase 2 second (`refactor(iter-N): consolidate memory tools`), Phase 3
  third (`feat(iter-N): carry memory-wire scores`). Each phase: RED test →
  fix → scoped test battery → clippy gate → BUGS row closure in the same
  commit → push → deploy (`scripts/build.sh --release`, `~/.local/bin`,
  gateway restart, md5 verify) → live verify through the binary.
- RT-190 (topology) is deliberately NOT sequenced: all three phases work
  identically under any topology decision because they write through the
  provider's bank abstraction. The plan notes this so the operator's
  topology ruling can land whenever without re-doing anything here.

## Risks / open questions for the operator

1. Hook content growth: session-end + pre-compact writes add rows every
   session — ranking consequences on the small `operant` bank (2137
   imported legacy rows vs a handful of live rows). Mitigation: tags
   (`session`, `pre-compact`) keep them filterable; if recall quality
   degrades, the tags are the lever. Watch after Phase 1 via one live
   session's `memory_search` results.
2. `on_turn_start` emission: verify the executor actually emits it on the
   run path before wiring the 1.4 drain there (else drain at `prefetch`).
3. Alias retirement (2.4's alias note) means the wire count drops only
   when aliases are removed — decide one release cadence for the drop.
