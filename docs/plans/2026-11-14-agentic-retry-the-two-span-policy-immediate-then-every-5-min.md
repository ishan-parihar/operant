# Agentic retry: the two-span policy (immediate, then every 5 minutes, infinite by default)

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it in
genome.toml `core.living_plans` and keep a 'Resume here' section).

**Question this answers.** "How does the agentic loop recover from a provider interruption
without hanging — and how is that recovery configured, documented, and self-healing?" The
config-resilience contract it depends on: `docs/plans/2026-11-14-config-resilience-architecture-...`.

## Mechanism — one policy, two spans, one opt-in ceiling

```
[agent.retry]                       # every key documented, healed, describable
small_span_retries = 3              # immediate re-issues of the SAME request, no sleep
large_span_secs    = 300            # once the small span is spent: retry this often, forever
max_span_secs      = 0              # 0/omitted = INFINITE (the default); set it to stop there
announce_large_span = true          # heartbeat event on every large-span retry
```

- **Small span** (`1..=small_span_retries`): immediate re-issues, no delay — "span directly".
- **Large span** (`> small_span_retries`): one request per `large_span_secs`, **indefinitely** —
   the 5-minute span is the large span and it retries endlessly by default.
- **Ceiling** (`max_span_secs`): opt-in; `0`/omitted = infinite. When set and the elapsed span
   reaches it, stop and surface the typed error ("gave up after Xm").
- **Rate-limit aware**: a `RateLimit` (429) failure uses the provider's own `retry_after`
   instead of the flat large span.
- **Visible**: every large-span retry emits a heartbeat (the TUI retry pill + the gateway log
   line). An infinite silent retry is the "hang" this is meant to kill.

### Where it applies (the one authority, and the others)

| Ladder | Today | With the policy |
|---|---|---|
| L1 transport (client) | 3 × 60s + exp(5s→60s) | unchanged — wire-layer retries, a different failure |
| L2 mid-stream (body drop) | 45s budget | unchanged — a different failure, typed `StreamDied` |
| L3 agent re-issue (`reissue_with_budget`) | 3 immediate, **no backoff** | **the policy**: 3 immediate, then 300s, infinite unless capped |
| L4 cron re-arm | `[30s, 120s]` × 2 | **becomes the large span**: 300s, infinite unless capped; the 30/120s pair is deleted |

The stop is `max_span_secs` (a named knob), not the turn's wall clock — the wall clock stays the
*turn* bound; the retry policy is independent of it.

## Gates / success criteria

1. A scripted provider that fails the first 3 calls then answers → the turn completes with the
   answer, and the client saw exactly 4 calls (3 immediate, then the 4th after `large_span_secs`).
2. A provider that never answers → the turn retries at `large_span_secs` indefinitely when
   `max_span_secs = 0` (asserted by elapsed-time bounds, not by waiting 5 minutes: inject a
   fake clock / a tiny `large_span_secs` in the test config).
4. With `max_span_secs` set, the turn stops at that span and surfaces the typed error.
5. A 429 with `retry_after` uses the provider's hint, not the flat large span.
6. Self-healing: a missing `[agent.retry]` block is healed to the defaults and written back once;
   an invalid value (negative, zero where it must be positive) is healed and logged.
7. The heartbeat event fires on every large-span retry (`announce_large_span`).

## Open decisions

- The exact default numbers (`small_span_retries=3`, `large_span_secs=300`, `max_span_secs=0`)
   are the operator's to set; the plan's defaults are the healing values.
- Whether the large span applies to every seat (TUI/CLI/cron/interactive) or only cron — the
   plan assumes every seat (the seat is the unit that "hangs").

## Cost

One `RetryPolicy` module (with its tests), the `[agent.retry]` block on the config-resilience
contract, the wiring in `reissue_with_budget` and the cron seam, and the heartbeat event.
Estimated at two iterations: the policy + wiring, then the self-healing + describe integration.
