# Context management / compression — long-horizon effectiveness audit (2026-10-25)

**Scope.** `operant-core/src/context_management.rs` (2,493 lines), the agent's
build path (`agent/run.rs` `build_messages`, the preflight ladder, the bust
gate), the compressor family (`agent/compress.rs`, `agent/llm_compressor.rs`),
tool-result spend guards, workspace/AGENTS.md injection
(`context_files.rs`), and the optional engines (`context/lcm.rs`,
`context/adaptive.rs`, `context/rollup.rs`). Everything below was read at
commit `7df1fa5e` (iter-1279); line references are to that tree.

**Method.** No claims from memory: every mechanism below was located, its
caller graph checked (`grep` for live call sites outside its own module and
tests), and its constants cross-checked against the wiring in
`operant-cli/src/main.rs` and `operant.example.toml`.

---

## 1. The architecture as it actually runs

Per turn, `build_messages` assembles the request in this order
(`agent/run.rs:2479–2860`):

1. **Frozen prefix** (system prompt + skills) — byte-stable across turns
   (iter-39/911 prompt-cache design). Everything volatile rides the TAIL.
2. **Bust-permission gate** (plan 06 P1, `run.rs:2479–2504`):
   `bust_permission(estimated, window, natural, explicit)` — `Explicit`
   (/compact, clear) > `Force` (≥85% of window) > `Natural` (model changed /
   system prompt changed / cache idle past TTL). Under `None`, NO rewrite
   runs at all — the request is the previous one plus appends, so the
   provider prompt cache keeps hitting. This is the load-bearing decision:
   **all lossy machinery is gated behind cache economics.**
3. **Micro-compaction** (opt-in, off by default: `OPERANT_MICRO_COMPACTION`
   env or `[agent] micro_compaction`): at 40% of window, folds the oldest
   completed exchange (head keep 4 / tail keep 8) into a 240-char marker.
   This is the ONLY mechanism that rewrites the *store*.
4. **Preflight ladder** (fires ≥80% of window, only under a bust):
   rung 1 `fast_trim_tool_results` (re-trim historical tool results to
   2,000 chars; protect first 3 / last 4) → rung 2 `decay_render`
   (H50=100 tokens, D=20 — deliberately more aggressive than the standard
   200/30) → rung 3 `preflight_llm_summarize` (LLM summarize-before-evict,
   **only when `[agent] context_compression = true`, which defaults
   false**) → rung 4 `evict_to_budget` (or the context-engine `assemble`).
   A wrap-up rung appends a final-call copy when the ladder compacted.
   Above 95% after all rungs: typed `ContextExhausted` instead of a doomed
   400.
5. **Eviction** (`evict_to_budget_reporting`): tiered — T3 tool results
   first, T2 long/reasoning-ish assistant messages, T1 user/assistant
   last — FIFO within tier, recency reserve `(budget/4096).clamp(6,50)`
   messages always kept, whole tool groups as the atomic unit (a
   `tool_use` is never orphaned from its `tool_result`), and a final
   reserve-trim pass when the protected set alone exceeds the budget.
   `EvictionOutcome.within_budget` reports the impossible case upward.
6. **Reactive rescue**: a provider `context_overflow` classifies →
   `compress_context_overflow` (LLM-first when attached, else
   `manage_context` = decay 200/30 + evict) → one retry; a second overflow
   fails typed. Cooldown state persists in the DB (anti-thrash).
7. **Tool-result spend guards**, three layers composing: source cap
   (`truncate_tool_result` 4,096 bytes), share guard
   (`guard_tool_result`, default 5% of window, `[agent]
   max_tool_result_share`) with an exact-prefix body + explicit
   "N tokens withheld" footer, and the ladder's re-trim.
8. **AGENTS.md / workspace files** (`context_files.rs`): 7 filenames
   (AGENTS.md, CLAUDE.md, .operant.md, HERMES.md, .cursorrules, case
   variants), 20,000-char cap with 70%/20% head/tail truncation + a
   `[...truncated]` marker, prompt-injection pattern scan
   (`ignore previous instructions` etc.), invisible-character strip.
   Injected per turn into the **tail** volatile suffix as
   `<workspace_context>` — after the ladder, so it never evicts and never
   busts the frozen prefix.
9. **Optional lossless engine** (`context_engine = "lcm"`): SQLite DAG +
   fresh-tail assembly instead of lossy eviction, day/week/month LLM
   rollups, P3 adaptive recall (off by default). Default config attaches
   nothing.

**What is genuinely good here** (verified, not flattery): the store is never
silently destroyed — the ladder rewrites only the *built* list, so a long
session's full history survives on disk and reloads (only micro-compaction
and /compact rewrite the store); `estimate_message_tokens` counts tool-call
arguments and reasoning, not just content (the RT-116 fix —
`estimate_current_tokens` takes max(reported, heuristic)); eviction keeps
tool pairs whole at every level; `repair_tool_pairs` fixes the
tail-leading-tool-run malformed history; the bust gate is a real
cost-mechanism, not decoration; `guard_tool_result` never edits output
beyond prefix-drop + authored markers.

---

## 2. Gaps and fragmentations

Ranked by long-horizon effectiveness impact.

### G1 (HIGH) — The default long-horizon experience is silent lossy eviction

`context_compression = false` by default (`operant.example.toml:92`,
`config.rs:801`). With defaults, a long session crossing 80% of the window
gets: tool re-trim → decay truncation → tiered eviction. No summary is ever
produced; old turns are simply absent from the prompt. The entire
summarizer (`llm_compressor.rs`, 1,214 lines: head/tail protection,
iterative previous-summary merge, structured task/resolved/pending output,
cooldown persistence) is dark unless opted in. The machinery is complete and
tested; the default just doesn't use it. This is a defensible cost decision
(an extra LLM call per compaction), but it should be a *documented* decision
— today nothing in README/CHANGELOG tells an operator that long sessions
degrade to eviction unless they flip one flag.

### G2 (HIGH) — `on_pre_compress` is a no-op: memory never sees what eviction throws away

The hermes plugin behavior ("extract insights from messages about to be
compressed and prepend them as context") is wired in the ladder
(`run.rs:2735–2758`), but the trait method's only implementation is the
default returning `String::new()` (`memory_provider.rs:348`). Neither
`MemoryWireProvider` nor `BuiltinProvider` overrides it. So the insertion
code is live, reachable, and always inserts nothing. The store keeps the
bytes (G1's mitigation), but nothing distills them: across a restart the
same ladder re-evicts the same middle turns with no memory assist. This is
the single cheapest high-value gap: memory-wire `retain`-ing a short gist of
evicted exchanges would make long-horizon sessions recall what context
management removed.

### G3 (MEDIUM) — Three compaction surfaces, three different algorithms

- Manual `/compact` → `compact_history_now` → **micro-fold** (marker
  strings, lossy, 240 chars/exchange) — never the LLM summarizer.
- Reactive overflow → **LLM-first**, deterministic fallback.
- Preflight rung 3 → **LLM summarize-before-evict**, no fallback (later
  rungs own it).

Three entry points, three behaviors. `/compact` while
`context_compression = true` still uses the cheap marker fold — an operator
who enabled the summarizer gets surprise lossy output from the one command
whose name promises compaction. Either `/compact` should prefer the
summarizer when attached, or the docs must say it's a fold.

### G4 (MEDIUM) — Proactive compaction is invisible to the user

`AgentEvent::CompactionStarted/Completed` fire only on the reactive overflow
path and `/compact` (`compress.rs:313–321`, `events.rs:308–318`). The
preflight ladder emits `info!`/`debug!` logs — nothing the TUI renders. A
long session can have its middle evicted (or its tool history halved) with
zero user-visible signal. The events exist; the ladder just doesn't emit
them. (Rung outcome data — trimmed count, evicted tokens — is already
computed for the logs.)

### G5 (MEDIUM) — The semantic compaction stage is dead code

`SemanticCut` + `semantic_compaction_cutoff` (`context_management.rs:299–547`)
— the embedder-scored, goal-similarity *group selection* with an excellent
doc comment explaining why prefix-argmax is degenerate — has **zero live
callers**. Complete, tested (degradation-to-recency paths pinned), unwired.
Either wire it as a preflight rung / engine mode, or record it as a shelved
capability; today it is a trap for the next reader ("where is this used?"
— nowhere).

### G6 (LOW) — Summarizer model is not configurable

`LlmCompressorConfig::summarizer_model` defaults `"gpt-4o-mini"` and the
CLI wiring spreads `..Default::default()` over it
(`main.rs:2111–2115`) — no TOML key exists. On an omp-gateway or local
endpoint that doesn't serve that model name, enabling
`context_compression = true` makes every compaction fail (then cooldown)
and the feature silently degrades to the deterministic path. A
`[agent] summarizer_model` key (with empty = reuse the route model, which
`PreflightConfig.summary_model` already spells as its convention) would
close it. Related: the compressor default `summary_max_chars` is
`usize::MAX` while `PreflightConfig` (the same knob family) defaults
4,000 — the LLM summary is unbounded in the default attach path.

### G7 (LOW) — Threshold interplay is undocumented

The ladder gates at 80% (`PREFLIGHT_THRESHOLD_PERCENT`), rung 3 then asks
`should_compress(estimated > window × threshold_percent)` where
`threshold_percent` comes from `[agent] context_compression_threshold`
(default **0.5**, `config.rs:802` — not the compressor struct's 0.80
default, which the same wiring overrides). So defaults compose coherently
(ladder 80% > compressor 50%), but an operator who sets
`context_compression_threshold = 0.9` silently no-ops rung 3 inside an
80%-triggered ladder. Two thresholds, one gate, no doc.

### G8 (LOW) — Tool-result limits: three numbers, no one page

4,096-byte source cap · 2,000-char ladder re-trim (hardcoded
`PreflightConfig::default()` inline at `run.rs:2766`, not TOML-tunable) ·
5%-of-window share guard. They compose correctly (source → re-trim → share)
but the interplay lives in three files; a "tool results in context" doc
section would prevent the next person re-deriving it.

### G9 (LOW) — CJK estimation asymmetry (accepted, worth restating)

`estimate_tokens` is chars/4 and admits CJK underestimates (~1 token/char);
`decay_render` converts back with tokens×4 chars. Internally consistent, but
a CJK-heavy history can overflow while every estimate says it fits. The
bust `Force` gate and the 95% exhausted check use the same heuristic, so the
safety nets share the blind spot. (The `estimate_current_tokens` max-rule
mitigates by adopting the provider's exact count after the first request.)

### G10 (INFO) — AGENTS.md is re-read and re-scanned every turn

`load_context_file_prompt()` runs inside `build_messages` per turn — disk
read of up to 7 files, regex threat scan, invisible-char strip, every
build. Tokens are charged to the budget and it rides the tail (cache-safe),
so the cost is latency-only (~sub-ms warm), but a stat/mtime cache would
make long sessions free here. (This repo's own AGENTS.md is 84 KB → every
operant-run agent sees a 70%/20% truncation of the file the repo tells
agents to "read top-to-bottom" — a deliberate, correct bound; the note is
that our own dogfood is the heaviest case.)

### G11 (INFO) — Decay truncates with no marker (deliberate)

`decay_render` hard-truncates without "[truncated]" by documented choice
("the model doesn't need to know"). The counter-position: a model that
doesn't know content was amputated may quote half a line as fact. The choice
is defensible and documented; listed because every future auditor asks.

---

## 3. What I recommend, in order

1. **Wire `on_pre_compress` into memory-wire** (G2): retain a bounded gist
   of the evicted middle before rung 4 drops it. Smallest change, biggest
   long-horizon win — recall replaces what eviction removed.
2. **Emit `Compaction*` events from the preflight ladder** (G4) with the
   per-rung numbers already computed for logs. TUI shows "context
   compacted: −12,400 tok" once, not never.
3. **Decide the default for `context_compression`** (G1): either document
   "long sessions degrade to eviction unless enabled" in README + example
   TOML comment, or flip the default on. Do not leave it implicit.
4. **Unify `/compact`** to prefer the LLM summarizer when attached (G3).
5. **Wire or shelve the semantic stage** (G5). My read: wire it behind the
   lcm engine as an assembly mode, since it already needs an embedder.
6. **Add `[agent] summarizer_model`** + bound `summary_max_chars` (G6), and
   one doc paragraph on the two thresholds (G7).

---

## 4. Verdict

The compression architecture is *fragmented but coherent*: five mechanisms
(micro-fold, ladder, reactive, share guards, engine) share one token
accounting and one cache-economics gate, and the correctness invariants
that matter most (tool pairs whole, store never silently destroyed, frozen
prefix never busted, eviction outcome reported) are all real and tested.
The long-horizon *effectiveness* gap is not in the mechanisms — it is that
the two mechanisms that make long sessions actually work (summarization,
memory distillation of the evicted middle) are respectively off-by-default
and a no-op. Everything else is tuning.

— audit by the operator agent, iter-1279 session. Rows G2/G4/G5 filed as
RT-181..RT-183 in BUGS.md.
