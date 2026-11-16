# Turn attribution — where a turn's wall clock went

Owns: the four exclusive spans a completed turn is classified into
(`inference`, `tools`, `user_wait`, `overhead`), the capture that fills them,
and the post-turn `TurnEnd` seam that delivers the classified value. It also
owns the turn-end heuristics that decide whether a model's stop was really a
truncation and whether a tool batch was usable at all.
Not for: the turn context that rides the request tail
([context-injection.md](context-injection.md)), the context-eviction ladder
([context-management.md](context-management.md)), or the observer feed
([observability.md](observability.md)).

Source: `crates/operant-core/src/turn_attribution.rs` (the spans, the
breakdown, the three-state classifier), `crates/operant-core/src/turn_end.rs`
(the `TurnEndBus` and the delivered `TurnEnd` event),
`crates/operant-core/src/turn_end_heuristics.rs` (truncation detection and the
tool-loop anomaly detectors), `crates/operant-core/src/agent/run.rs`
(`turn_attribution()`, the emit site, `TurnTiming`),
`crates/operant-core/src/agent/stream.rs` (the per-call tool-duration map),
`crates/operant-core/src/agent/mod.rs` (the map's field).

## Contents

- [The rule](#the-rule)
- [The four spans](#the-four-spans)
- [The three-state classifier](#the-three-state-classifier)
- [Where the capture happens](#where-the-capture-happens)
- [The seam and its consumer](#the-seam-and-its-consumer)
- [Configuration](#configuration)
- [The turn-end heuristics](#the-turn-end-heuristics)
- [Failure modes](#failure-modes)

## The rule

> `Unmeasured` is a different answer from `0`, and conflating them is the bug
> this whole family exists to avoid.

A `Duration` of zero cannot distinguish "the tool took no time" from "nobody
timed the tool". A `u64` of milliseconds cannot either. `SpanDuration` is an
enum so it can: `Measured(Duration)` carries a real reading (which may
legitimately be zero) and `Unmeasured` is a named absence that renders as
`n/a`, never as `0ms`. RT-199 was that bug; the fix was the per-call map in
`agent/stream.rs`, filled by `execute_tools` at the same three sites that feed
`TurnEndBus::record_tool_duration`, and read back by the `ObserverEvent::ToolCall`
emit — before it, `ConsoleObserver`'s `duration_ms` was structurally always
zero.

## The four spans

`TurnSpan` has four variants and `ALL` lists them in render order:

| Span | What it measures |
|---|---|
| `inference` | time inside a provider call, waiting on the model |
| `tools` | time inside a tool, waiting on the effect |
| `user_wait` | time the operator held the turn (the pause/estop gate) |
| `overhead` | the residual: message building, context management, redaction, persistence, the emit awaits |

`TurnBreakdown::build(total, inference, tools, user_wait)` derives the fourth as
`total.saturating_sub(sum)`, and when ANY input is `Unmeasured` the derived
residual is `Unmeasured` too — a precise residual derived from partial inputs
would be a fabricated number.

## The three-state classifier

`TurnAttribution::of(breakdown)` returns one of three states:

- **`Disabled`** — capture is off. The kill-switch (`[agent]
  show_turn_attribution = false`) returns `None` before anything is built, so
  this variant never reaches the classifier from the production path. It exists
  because the delivered field is an `Option<TurnAttribution>` over a seam that
  can carry it.
- **`Incomplete { measured, missing }`** — at least one span was never
  captured. It carries the spans that WERE measured AND a `missing` list naming
  the ones that were not. That list is the whole point: a subscriber that
  renders "tools: n/a" without being told WHICH spans are absent cannot tell a
  capture gap from a model that used no tools.
- **`Complete(TurnBreakdown)`** — every span was measured.

### `Complete` means measured, not exclusive (RT-431)

The classifier checks measurement, never arithmetic. A turn whose measured
spans exceed its wall clock (the 8-worker pool runs tools concurrently, so a
turn's tools span can exceed its wall clock) is `Complete`, while its spans do
NOT sum to the total. `is_exclusive()` is the separate question: it sums every
span and compares, and reports false there. A subscriber matching on `Complete`
is not being told the spans partition the turn, and the module's own guard test
pins that the docs do not claim otherwise.

## Where the capture happens

`OperantAgent::turn_attribution(turn_start, inference, saw_inference)`
(`agent/run.rs`) builds the breakdown from three sources:

1. `inference` is `Measured` only when `saw_inference` is true. A turn that exited
   before any model call leaves it `Unmeasured` — a real zero would say the
   model answered instantly.
2. `tools` comes from `self.turn_tool_duration()`, which sums the per-call map
   keyed by tool-call id. `None` from an empty map is `Unmeasured`, not zero.
3. `user_wait` is honest `Unmeasured` at this seam: the approval gate is a
   synchronous decision and the guardian LLM's time is model time already
   counted in `inference`. Nobody fabricates a zero for it.

The result is `TurnAttribution::of(breakdown)` — the classified form, which is
what the event carries.

## The seam and its consumer

`TurnEndBus::emit(session_id, iterations, tool_calls, result, attribution)`
delivers one `TurnEnd` per completed turn on a tokio broadcast channel
(`SUBSCRIBER_QUEUE = 64` slots per subscriber). The event is deliberately
history-free: counters, the bounded `tool_durations_ms` vec
(`MAX_TOOL_DURATIONS = 256`), the attribution, and a `result_summary` cut at
`RESULT_SUMMARY_LIMIT = 512` bytes on a char boundary — so a clone costs a few
hundred bytes, never a `Vec<Message>`.

`attribution` is `None` when nothing was captured (the kill-switch, or a turn
that exited before any model call), and that is a distinct state from a span
left `Unmeasured`: `None` means no attribution was attempted, `Unmeasured`
means attribution ran and that span is honest about having no number.

**What actually renders it, as of the commit this page is written against:**
`TurnEndBus::log_attribution` prints one `debug!` line per turn at the emission
site — `render_one_line()` for `Complete`, and for `Incomplete` the missing
span names alongside the same line. That is the consumer RT-209 deferred for
its whole life, and a line in `gateway.log` is a real observable. What is
still absent is a SUBSCRIBER that reads the field: both existing subscribers
(`advisor.rs`, `recap.rs`) read only `end.session_id`, so the value the event
carries is dropped after the log. `TurnBreakdown::render()` (the multi-line
block) and `TurnBreakdown::is_exclusive()` have no production caller — the seam
uses `render_one_line`.

This module is no longer an orphan. It was one (iter-1631 wired `TurnSpan`,
`TurnDuration` and `TurnBreakdown` and left `TurnAttribution` unused), and NO
gate caught it: the types are `pub` in a `pub mod`, so zero users emits no
`dead_code` warning, tests are green, and clippy is silent. That is why the
module carries an orphan guard that scans the four production seams per public
type and requires `TurnAttribution::of` to be called from OUTSIDE this module —
a guard that passes silently is the failure this module already shipped twice.

## Configuration

`[agent] show_turn_attribution` (`crates/operant-core/src/config.rs`, default
`true`). Reachable as `[agent] show_turn_attribution` in `operant.toml`. The
iter-1631 note is load-bearing: the field was added and filled with a literal
`true` at every construction site, which made the kill switch unreachable from
configuration — a flag that cannot be set is documentation, not a switch.

## The turn-end heuristics

`turn_end_heuristics.rs` answers two other questions a turn's ending raises,
and both are pure string analysis so they are unit-testable without a model.

### Was this stop really a truncation?

`should_treat_stop_as_truncated(model, finish_reason, content, has_tool_messages,
has_tool_calls)` catches a provider that reports `finish_reason="stop"` on
output the length limit actually cut. It gates on the model name containing
`glm` (the Ollama-hosted GLM family misreports truncation as a stop) AND on the
content having no natural ending after thinking blocks are stripped. The
natural-ending check is the real guard against false positives on well-behaved
proxies, which report `finish_reason` correctly.

`has_natural_response_ending` accepts terminal punctuation, a closing code
fence, a caret, and the emoji/symbol ranges. `thinking_exhausted` is the
sibling case: the response burned the entire output budget on reasoning with
nothing visible left, so a continuation retry would be pointless and a targeted
error is the right answer.

The continuation itself reuses the existing shape in `run()` (append a user
`Message`, refund the iteration, `continue`) — there is no second loop.
`continuation_prompt()` is the bare instruction, and
`continuation_prompt_with_partial(partial)` carries the text that WAS produced,
because a bare "continue" makes the model restart the answer from scratch: it
cannot continue what it cannot see.

### Were the tool results usable?

`detect_anomalies(tool_bodies, response_text)` runs three deterministic
predicates over the trailing tool batch plus the response:

- `is_empty_tool_result` — the body is empty or whitespace only;
- `is_malformed_tool_output` — the body is shaped like JSON (starts with `{`
  or `[`) but its LEADING value does not parse. Only the leading value is
  checked, because the agent itself appends annotations after a tool's body
  (`[Tool loop warning: …]`) and a whole-body parse flagged those valid results
  as malformed;
- `has_error_signal` — the first non-empty line starts with an error prefix
  (`error:`, `fatal:`, `panic:`, `traceback`, …) or contains one of the
  mid-line markers (`command not found`, `no such file`, `permission denied`).

The feedback message rides the same continuation shape, capped by
`MAX_ANOMALY_RETRIES = 3`, and the cap is counted by SCANNING the message list
for the `ANOMALY_RETRY_PREFIX` marker rather than by a parallel counter —
`count_injected_retries` for the whole list, `count_turn_injected_retries`
scoped to the current turn, because a persistent session that spent its budget
in an earlier turn used to get no anomaly retry for the rest of its life.

## Failure modes

| What breaks | What you see |
|---|---|
| Capture off | `attribution` is `None`; nothing is rendered and nothing is logged |
| No model call happened | `inference` is `Unmeasured`; the turn classifies as `Incomplete` naming it |
| Tools ran but were never timed | `tools` is `Unmeasured`, never `0ms` |
| The measured spans exceed the wall clock | `Complete` while `is_exclusive()` is false — concurrent tools, honestly reported |
| A tool batch is unusable | one bounded injected retry naming the anomaly, capped at three, never an infinite loop |
| The model misreports a truncation as a stop | the GLM-gate plus the natural-ending check decide; a well-behaved proxy is not affected |

## Intention

The family exists because "why did this turn take 40 seconds" is a question the
operator asks, and nothing in operant answered it. The three things that were
already measured (the LLM call duration, each tool's duration, the turn total)
are all inclusive and overlapping: the tools run inside the turn and the LLM
call that follows them runs inside it too, so summing them tells the operator
nothing and double-counting tells them a lie over 100%.

The emission at turn END rather than in the prompt tail is a measured
architectural constraint, not a preference: the `<turn_context>` tail is built
exactly once per turn, before the loop starts, when no model call has happened
and no tool has run, so a block placed there would render `n/a` for every span
on every turn. Emitting it on the last iteration instead is worse — it would
break the frozen-prefix byte-stability the provider cache depends on.

This is deliberately NOT a TUI panel. A panel needs new TUI surface, which is a
peer's area and a far larger iteration; the seam is the one place that already
holds the classified enum AND the guarantee that the turn is over.