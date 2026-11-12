# Audit — the poke that never fired on a 120s loop ceiling

**Date**: 2026-11-12 · **Scope**: `crates/operant-core` (agent loop, error taxonomy, cron seam,
the auto-poke), `crates/operant-cli` (gateway error class, config wiring) · **Trigger**: the
operator reported that the operant agent's poke "does not work" on interruptions like
`Error: Agent error: request timed out after 120s (loop request_timeout ceiling)` and that it
"hangs the agentic-loop instead of retrying it effectively".

**Method**: (1) read the live logs for the failure signature; (2) walked the failure backwards from
the operator-visible string to the branch that produced it; (3) extracted the two DECISION
functions that decide "is this failure retryable?" — `Error::is_transient` and `Failover::classify`
— verbatim out of the repo and executed them under `rustc` against the exact error text the loop
produces, so the verdict is measured rather than asserted; (4) grepped for the callers of every
recovery primitive (the poke's own seam included); (5) fixed, then pinned each pair with
regression tests.

> **Base note.** The audit ran against a checkout 448 commits behind `origin/main`; the fix was
> rebased onto `origin/main` (iter-2118) before it was committed. The rebase CHANGED THE SUBJECT:
> upstream already ships the poke (`crates/operant-core/src/agent/auto_poke.rs`, ported from
> jcode's auto-poke), and the rebase exposed the seam the operator's report is actually about —
> **G6**, the poke's unreachable error path. Both halves are fixed here.

---

## 1. The symptom, as it happened

From `~/.operant/logs/gateway.log.2` (2026-10-16, the clearest specimen — the same two lines repeat
every few minutes for hours):

```
12:01:34  WARN  No response headers in time, retrying, model: claude-sonnet-4-20250514, attempt: 1, idle_secs: 60
          at crates/operant-core/src/client.rs:507
12:02:34  WARN  LLM request exceeded loop request_timeout ceiling, budget: 120s
          at crates/operant-core/src/agent/events.rs:101
12:04:38  WARN  No response headers in time, retrying, model: claude-sonnet-4-20250514, attempt: 1, idle_secs: 60
12:05:38  WARN  LLM request exceeded loop request_timeout ceiling, budget: 120s
          ... and again at 12:07/12:08, 12:11 ...
```

And the cron seam's own verdict, minutes earlier (2026-10-09):

```
10:03:44  WARN  socialization: turn 1 for `crew-chief` failed: Agent error: request timed out after 120s (loop request_timeout ceiling)
          at crates/operant-core/src/cronjobs/scheduler.rs:750
10:03:44  INFO  Starting agent run            ← the NEXT job starts 16ms later
```

**Read the pair.** The provider needed more than 60s to send headers, the client's own ladder
re-issued once, and at exactly 120s the agent's outer ceiling cancelled the whole request. The
scheduler did not wedge — it moved straight to the next job. **The seat did.** Nothing re-issued
that seat's request, so it stayed dark until its next cadence tick (a daily cast: up to 24h).

## 2. The discrepancy, stated exactly

- **E**: a slow provider is a retryable condition. The turn re-issues the request while the turn's
  retry budget lasts; if the turn still fails, the harness's bounded poke gives the turn one more
  pass with its tools; if that fails too, the cron seam's bounded transient retry re-arms the job
  within 30s/120s. The seat's cadence survives one slow provider.
- **O**: the first timeout ends the turn; the poke never fires (its error path is unreachable); the
  cron seam's retry gate does not see the failure at all; the seat waits for its next scheduled run.

## 3. The gap — six defects, one causal chain

### G1 · The outer ceiling eats the inner retry ladder

`agent/events.rs` races the WHOLE `chat_streaming` future against `loop_request_timeout()` =
`max(request_timeout_secs, reasoning floor)` = 120s. Inside, `client.rs` runs its own ladder:
`max_attempts` attempts, each with a header wait of `effective_timeout(model)` =
`[client] timeout_secs` = 60s. So attempt 1 burns 60s, attempt 2 starts, and the outer 120s ceiling
cancels the ladder mid-attempt-2 — exactly the 60s + 60s pattern in the log.

The two budgets come from two knobs that are never reconciled (`operant.toml:10` = 60s,
`operant.toml:25` = 120s), and the outer one wins by construction: `120 < 3 × 60 = 180`.

### G2 · The ceiling's error was UNTYPED, so every recovery gate read it as "give up"

The ceiling produced `Error::Agent(String)`. Measured, by executing the repo's own decision
functions against the exact error text:

| Gate | Before | After this fix |
|---|---|---|
| `Error::is_transient()` (`error.rs`) | **false** — `Error::Agent(_)` is not in the arms | **true** (typed variant) |
| `Failover::classify(...).reason` (`agent/fallback.rs`) | **`Unknown`** — the catch-all `_` arm | **`Timeout`** |
| `.retryable` | true | true |
| `FailoverDecision` | `GiveUp` | `FallBack` (the model chain advances) |
| cron's `should_arm_transient_retry` (`cronjobs/scheduler.rs`) | **false** → the bounded retry (30s/120s, `MAX_TRANSIENT_RETRIES = 2`) NEVER armed | **true** → the seat re-arms |

The typed rung existed and was unreachable: `FailoverReason::Timeout` ("Connection/read timeout —
rebuild client + retry") was constructed only for `Network` / `StreamStalled`, and **no production
code branched on it** — grep finds only the classifier's own tests.

The same untyped string also defeated the THINKING-TIMEOUT annotation: `annotate_thinking_timeout`
re-wrapped every annotated error into `Error::Agent`, so the one model class that most needs a
raised ceiling was also the one whose retry was silently disabled.

### G3 · The turn had no retry arm for a failure before the first byte

`agent/run.rs` — the streaming establishment path and the non-streaming path — branched only on:
context overflow → compress + retry; reasoning-param rejection → retry without the field;
auth/rate-limit → rotate credential + retry. Everything else → `return Err`.
`classified.retryable` was consulted ONLY in the mid-stream consumption loop, never on the
establishment path — so the turn's retry budget (`max_retries = 3`) was never spent on a timeout.
Two existing re-issues (the reasoning-param retry and the rotate retry) also bypassed the loop
ceiling entirely.

### G4 · The core's re-prompt primitive is dead code

`OperantAgent::run_with_healing` is the only re-prompt primitive in the core and it has **zero
callers** — grep finds the definition and one comment. `max_healing_attempts = 3` in the operator's
config is inert. It is also the wrong remedy as written: it re-prompts with "the previous attempt
encountered an error", which cannot help a slow provider — a RE-ISSUE can.

### G5 · The reasoning floor never applied to the models actually in use

`get_reasoning_stale_timeout_floor` matched the slug after the last `/` with a right-boundary
whitelist of `- . _ /`. `:` was NOT a boundary, so every aggregator id with a variant suffix missed
the floor:

| Model id | Before | After |
|---|---|---|
| `dots-studio/dots-3-note-preview:free` | **None** (the `dots-3-note-preview` entry — added because "cron seat 120s kills" — was dead for this id) | 300s |
| `deepseek/deepseek-r1:free` | **None** | 600s |
| `openai/o3-mini:free` | 600s — matched the shorter `o3` needle, not `o3-mini` | 300s (`o3-mini`) |
| `claude-sonnet-4-20250514` (the id in the live log) | **None** — the table only carried 4.5/4.6/5 | 240s |

Worse, the floor is looked up by `self.model()` — the RAW config string. The operator's live config
runs the aliases `large-stack` / `small-stack`, which the vendor-slug table can never match, and
there was no per-model timeout knob at all despite the doc promising one.

### G6 · THE POKE'S ERROR PATH WAS UNREACHABLE (the named gap)

Upstream's poke (`agent/auto_poke.rs`) is the bounded synthetic continuation: when a turn's
wrap-up produced nothing, the harness gives the turn one more pass WITH its tools, bounded by
`MAX_AUTO_POKES = 2`. Its guard is explicit — `PokeState::record_error` is called at BOTH provider
error sites (the establishment path and the mid-stream path) so the poke can tell a deterministic
rejection (context length, invalid model, quota — a second identical request cannot help) from a
transient one.

**But nothing ever asked.** The error paths ended in `return Err(...)`: the poke's recorded error
was read by nobody, the turn died with an error card, and the seat's work died with it. The poke
was reachable only from the early-exit seams (iteration budget, wall clock, seat cap, an empty
wrap-up) — never from an interruption. That is precisely the operator's report: the poke does not
work on an interruption, and the agentic-loop hangs.

The upstream test suite pinned the poke's four happy-path behaviours and nothing about a provider
error — so the missing seam was invisible to the tests, not protected by them.

## 4. What landed

1. **The ceiling is typed.** `Error::RequestTimeout { budget, guidance }`: transient, NOT
   self-healing, and it keeps the operator-visible text ("request timed out after 120s (loop
   request_timeout ceiling)"). `Failover::classify` maps it to `FailoverReason::Timeout` with
   `retryable: true`, and the model chain advances on it.
2. **The annotation preserves the type.** `annotate_thinking_timeout` now carries the guidance in
   the variant's own field instead of re-wrapping into `Agent`.
3. **The turn re-issues.** `prepare_retry_request` (the budget bookkeeping) and
   `reissue_with_budget` (the loop that spends it) back both establishment paths: while the turn's
   retry budget lasts, the turn refunds its iteration, consumes one retry, emits
   `AgentEvent::RetryScheduled`, rebuilds the request and RE-ISSUES it under the ceiling. Each
   re-issue's own failure is classified again, so a failure that changes class is re-read rather
   than assumed. The two pre-existing re-issues (the reasoning-param and the rotate ones) now run
   under the ceiling too.

   **Only a CLASSIFIED failure earns a re-issue.** The classifier's catch-all arm marks EVERY
   unclassified error `retryable`, and a retry on a coin flip turns a deterministic failure into a
   slow one — the first run of this fix broke `review_discipline`'s
   `error_exit_skips_the_armed_review` (a scripted `Error::Agent("provider exploded")` used to end
   the turn and now kept it alive). The arm therefore skips `FailoverReason::Unknown`.
4. **THE POKE REACHED THE ERROR PATH (G6).** The poke decision is factored out of
   `wrap_up_or_poke` into `maybe_poke`, and BOTH establishment error paths now call it when the
   turn's re-issue budget is spent: a CLASSIFIED transient failure gets the bounded poke and the
   turn continues; a deterministic or UNCLASSIFIED one still ends the turn. The error path
   deliberately does NOT make the grace call — the provider has just failed, so asking it for a
   tool-less summary would spend another request on a provider that is already refusing.

   **The unclassified case ends the turn on purpose.** Upstream pins "an error ends the turn"
   (`review_discipline::error_exit_skips_the_armed_review`: the operator already knows why the
   turn stopped, and there is no final answer for the review lane to evaluate). A re-issue or a
   poke on a failure nobody could classify is a retry on a coin flip. The classified transient
   failures — the loop ceiling, a 503, a transport error — are the ones the operator's report is
   about, and those are the ones that now recover.
5. **The client is polite.** The header-timeout branch now backs off before its next attempt
   instead of re-issuing immediately.
6. **The floor matches real ids.** `:` and `+` count as boundaries; the dated Claude 4 generation
   is in the table; the guidance text no longer claims a single global floor.
7. **The operator gets the knob the doc promised.** `[client] model_request_timeouts = { "large-stack" = 600 }`
   — an explicit per-model budget that wins over both the global timeout and the built-in floor, and
   that bounds BOTH the transport's header wait and the agent's loop ceiling (the agent's copy is
   threaded in `main.rs::agent_config`).

## 5. The regression tests

- `crates/operant-core/tests/poke_after_provider_error.rs` (new) — the E/O pair for the poke: a
  CLASSIFIED transient failure (an overloaded 503) after real progress is re-issued while the
  turn's budget lasts, and once that budget is spent the bounded poke gives the turn its last
  pass — the poke's answer is the turn's answer, and the synthetic continuation really reached the
  model. The CONTROL: a deterministic failure (a 400 billing rejection) ends the turn with no poke
  and no extra request.
- `crates/operant-core/tests/request_timeout_retry.rs` (new) — drives the REAL race (the client
  really sleeps past the agent's `request_timeout`): the turn completes and the request is
  re-issued exactly once; the retry is visible to the host; a provider that never answers fails
  with the TYPED transient error; and with `max_retries = 0` the same provider is fatal (the
  control that proves the arm is load-bearing and bounded).
- Unit tests alongside the code: the typed error's transience (`error.rs`), the `Timeout`
  classification (`agent/fallback.rs`), the cron seam's retry gate (`cronjobs/scheduler.rs`), the
  floor matcher and the explicit override (`reasoning_timeouts.rs`), and the agent's ceiling
  honouring the per-model budget (`agent/mod.rs`).

## 6. Residual risk / what was NOT changed

- The two budgets (`[client] timeout_secs` vs `[agent] request_timeout_secs`) are still two knobs.
  With the fix the cancellation is no longer fatal, but an operator who sets the client's ladder
  larger than the agent's ceiling still burns the first attempt before the re-issue. Aligning them
  automatically was rejected as a larger change than the gap needed.
- `run_with_healing` is still uncalled (G4). The typed re-issue is the remedy a timeout needs; the
  self-healing re-prompt is a different mechanism and wiring it is a separate decision.
- The per-model knob is keyed by the model string as the config names it; an alias that resolves to
  a different id at runtime would need the resolved id in the key.
- The mid-stream give-up path (a stream that died after the retry budget) still ends the turn with
  a typed `Error::StreamDied` rather than reaching the poke. It has its own ladder and its own
  typed error; widening the poke to that path is a separate decision.
- The worst case is now SLOWER on purpose: a provider that times out on every attempt burns the
  turn's whole re-issue budget (4 attempts) and then the poke's own budget before the turn ends —
  minutes, not seconds. That is the trade the operator asked for ("retry it effectively"), and the
  per-model budget (`[client] model_request_timeouts`) is the lever that removes the need for it:
  a model that legitimately needs 600s to answer then never reaches the ceiling at all.
