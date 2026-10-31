# Family 08 — observability: where the turn's time actually goes

**Question this file answers:** operant already emits LLM latency, per-tool duration, and turn
totals. codex has a **critical-path calculator that attributes a turn interval to exclusive
categories**. Is there a real gap, or is it the same measurement with a nicer name?

**Ground state:** operant `main` @ `f53907e8`. Reference `../external-repos/codex/codex-rs` @ `588a277b`.

**Scope note:** the operator observer seam (`observer.rs`), `ConsoleObserver`, and the tool-call
duration/trace payload are covered by prior audits
(`docs/audit/2026-10-30-codex-portability-ledger/04-observability-and-protocol.md`, shipped at
iter-1506/2cd1a0e2). This file covers what those did **not**: attribution within the turn.

---

## CODEX-SIDE

### What the mechanism is

`utils/critical-path/src/lib.rs` (181 LOC) computes, for **one observed turn interval**, how the
time divides across mutually exclusive categories. The doc comment (`:1-4`) states the property:

> "Persistent dependency paths and exclusive attribution over the observed turn interval.
> History is depth-limited; formatting and destruction never recurse through the chain."

```rust
// lib.rs:8-24 — the category vocabulary
pub enum WorkKind {
    Inference,      // model inference work on the selected dependency path
    AutoReview,     // automatic review of a tool call before execution
    UserWait,       // time awaiting a human answer or approval
    Tools,          // tool execution, including work after a dependency join resumes
    Overhead,       // time in the interval not covered by a recorded work segment
    /* + work whose category is unavailable, history collapsed at the depth limit */
}
```

The load-bearing word is **exclusive**: every millisecond of the turn belongs to exactly one
category, and `Overhead` is the residual that catches what no segment claimed. The path is a
persistent chain with `MAX_PATH_DEPTH = 16_384` (`:6`), and traversal is iterative so
"formatting and destruction never recurse through the chain" — a stack-overflow guard on a
16k-deep structure.

### How it reaches the operator — and the state model worth copying

`core/src/turn_timing/report.rs:4-25` packages the calculator's output into a **three-state** result:

```rust
pub enum TurnCriticalPath { Disabled, Incomplete, Complete(TurnCriticalPathDurations) }

pub struct TurnCriticalPathDurations {
    pub duration_ms: u64, pub inference_ms: u64, pub auto_review_ms: u64,
    pub user_wait_ms: u64, pub tools_ms: u64, pub overhead_ms: u64, pub unknown_ms: u64,
}
```

and the analytics doc (`analytics/src/turn_critical_path.rs:1-2`) states the invariant that makes
it useful:

> "Missing capture never becomes a zero-duration measurement."

**This is the part of the design that matters most to operant, and it is not the arithmetic.**
operant's own history has this exact defect class, twice:

- **RT-199 (CLOSED iter-1505):** `ObserverEvent::ToolCall` carried a hardcoded
  `Duration::from_millis(0)`, so `ConsoleObserver`'s `duration_ms` was structurally always `0`
  while the real timings were measured and discarded one hop before the observer. The ledger's
  own words: *"An operator asking 'which tool made this turn take 40 seconds?' got an answer with
  no durations in it."*
- **RT-199's lesson applied here:** a single `u64` duration field **cannot distinguish "this took
  zero time" from "nobody measured this"**. That is precisely the ambiguity that let RT-199 live
  its whole life undetected. The three-state enum `{Disabled, Incomplete, Complete}` removes it,
  and `Incomplete` is the state that would have surfaced RT-199 on day one.

So the import is not the 181 lines of interval arithmetic. It is the **state model** — never emit a
duration that was not captured — plus the exclusive-interval construction that answers the
operator's question.

`report.rs:11-13` also records the honest rounding note: sampling and compaction both contribute
to inference, and independent per-category truncation can leave the category sum up to 5 ms
below `duration_ms`. A sum that does not quite add up is documented as expected, not fixed by
fudging.

### The load-bearing units (file:line + LOC)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `critical-path` | `utils/critical-path/src/lib.rs` | **181** | exclusive attribution + depth-limited chain |
| `turn_timing` | `core/src/turn_timing.rs` | — | feeds the turn interval in |
| `direct-tool-timing` recorder | `direct-tool-timing/src/recorder.rs` + `recorder/runtime.rs` | **428** | per-tool timing detail |
| analytics reducer | `analytics/src/reducer.rs` | 4,402 | event → aggregate fold |

---

## OPERANT-SIDE

### What exists today

Three independent timings, all real and all already wired:

| measurement | site | how it is surfaced |
|---|---|---|
| LLM call latency | `agent/run.rs:1234` (`llm_duration`) | `ObserverEvent::LlmResponse { duration }` **and** `ObserverMetric::RequestLatency` (`:1245-1246`) |
| Turn total | `agent/run.rs:1705` (`turn_start.elapsed()`) | `ObserverEvent::AgentEnd { duration }` (`:1706-1712`) |
| Per-tool duration | `agent/stream.rs:1638`, `:1640`, `:1677`, `:1679`, `:1764`, `:1768` | `ObserverEvent::ToolCall*` duration fields |
| Stream retry budget | `agent/stream_retry_budget.rs:176-220` | `elapsed` / `exhausted` / `remaining` |

And the aggregate reporting side is genuinely capable:
`agent/insights.rs` (`InsightsReport`, `:19-31`) already computes overview totals, per-model
token/cost breakdowns, platform breakdowns, per-tool counts with percentages, activity patterns
(busiest day/hour, streaks), and session highlights. `tui/stats_dialog.rs` (183 LOC) renders it.

### The gap, stated precisely

**It is a real gap, but it is not the one the name suggests.**

operant has **three numbers** — how long the model took, how long each tool took, how long the
turn took — and they are all *inclusive*: tool durations overlap the LLM duration, which overlaps
the turn total. **Nothing answers "of this turn's wall clock, how much was the model, how much
was tools, how much was me waiting for the operator, and how much was unaccounted?"**

That is exactly the question an operator asks when a turn feels slow. Today the answer has to be
assembled by hand from overlapping numbers, and the residual is invisible.

Two things codex measures that operant has **no field for at all**:

1. **`UserWait`** — time spent blocked on a human answer. operant has `user_question.rs` and the
   `ask_user` dialog, but no timing that attributes turn wall-clock to it.
2. **`Overhead`** — the residual. Without it, an unaccounted 20 seconds in a 45-second turn is
   simply not visible.

And the depth-limit recursion guard (`:6`, `:4`) is a pattern worth noting: a 16,384-deep chain
traversed recursively is a stack overflow waiting for a bug report. operant has no such structure
yet, so it has no such hazard — this is a note for the *implementation*, not a gap.

### What makes this a *smaller* import than the LOC suggests

The `WorkKind` vocabulary maps onto categories operant already times separately:

| `WorkKind` | operant today |
|---|---|
| `Inference` | measured (`run.rs:1234`) |
| `Tools` | measured per tool (`stream.rs:1638+`) |
| `UserWait` | **not measured** |
| `AutoReview` | not applicable — operant's guardian is a call classifier, not a pre-execution review step with a duration |
| `Overhead` | **not measured** — the residual |

So the *computation* is the new part. The inputs are all already being recorded; what is missing
is the **union into exclusive intervals** over one turn, and one field (`UserWait`) that needs a
new timer around the ask-user wait.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| Exclusive turn attribution (`WorkKind`) | `critical-path/src/lib.rs` 181 | three inclusive timings, no union | **med** — the operator's "why is this slow" question has no answer | yes, pure arithmetic over existing timestamps | **IMPORT-SMALL** |
| `UserWait` category | `WorkKind::UserWait` | `user_question.rs` exists, no timer | low | ~10 LOC | IMPORT-SMALL |
| `Overhead` residual | `WorkKind::Overhead` | none | low (falls out of the union) | free | IMPORT-SMALL |
| Depth-limited iterative traversal | `lib.rs:6`, `:4` | n/a | none | pattern | note for the implementation |
| `direct-tool-timing` recorder | `recorder.rs` 428 | `record_tool_duration` already exists | none | — | **ALREADY-HAVE** |
| Aggregate insights | `analytics/reducer.rs` 4,402 | `agent/insights.rs` 713 + `stats_dialog.rs` 183 | none | — | **ALREADY-HAVE** |

**The ponytail shape:** the honest version of this import is **not** porting a 181-LOC crate. It is
recording four `Instant`s operant already has (plus one new one around the ask-user wait) into a
single per-turn struct, computing `Overhead` as `turn_total - known_segments`, and exposing the
four numbers on the existing `<turn_context>` tail — the same mechanism the context-budget line
already rides. **~60 lines, no new dependency, no new crate.**

**The trap to avoid, recorded:** the naive implementation double-counts. If `Tools` is recorded as
the sum of per-tool durations *plus* the gap between them, and `Inference` is the sum of LLM
latencies, and the two are then added to the turn total, the result exceeds the turn interval and
the operator sees a percentage over 100%. The exclusive-interval construction — every millisecond
owned by exactly one category, residual in `Overhead` — is the entire value of the codex design,
and it is the part a quick port would skip.
