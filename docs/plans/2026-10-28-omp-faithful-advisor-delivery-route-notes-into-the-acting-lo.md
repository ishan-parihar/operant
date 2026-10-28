# Plan: OMP-faithful advisor delivery — route notes into the acting loop

Status: open
Owner: agent-jcode (operator session)
Date frozen: 2026-10-28
References: oh-my-pi `session-advisors.ts` (#routeAdvice, #deliverAdvisorBatch,
resolveAdvisorDeliveryChannel), `advise-tool.ts` (formatAdvisorBatchContent,
ADVISOR_GUIDANCE), `prompts/advisor/boundary-guidance.md`; operant
`advisor.rs` (375 lines), `agent/builders.rs::attach_advisor`,
`tui/adapter_types/tui_app.rs` attach site.

## The defect

The operator's report: "advisor/recap integration into the active agentic
loop" does not work like oh-my-pi. Verified root cause: operant's advisor
DELIVERS NOWHERE. `spawn_advisor`'s `deliver` callback (builders.rs:607-615)
sends each note to the TUI as an `AgentEvent::BackgroundReview` card, and the
module doc (advisor.rs:33-37) says this is deliberate: "never into the acting
agent's context (the omp incident class where advisor output became
model-visible prompt injection)". That was a MISREADING of OMP: OMP's advisor
notes ARE model-visible — that is the feature. The quarantine guards exist
precisely BECAUSE notes reach the model, and every channel in OMP's
`resolveAdvisorDeliveryChannel` delivers into the primary's context:
aside → YieldQueue custom message; steer → sendCustomMessage(deliverAs:
"steer", triggerTurn: true); preserve → preserved card that RE-ENTERS context
on the next user turn. The only model-visible-danger OMP guards against is
advisor output being TAKEN AS INSTRUCTIONS (the `<advisory guidance="weigh,
don't blindly obey">` framing + boundary-guidance header + hazard regexes).

Secondary defects found while mapping the chain:
- The advisor reviews only once per `run()` (TurnEnd fires once, at the
  single `return Ok(result)`); OMP reviews per agent-core turn (iteration).
  Not porting multi-boundary review in this iteration — operant's `run()`
  IS one turn boundary by construction (the emit is on the success path
  only, all early returns are Err). A note lands after the run returns, so
  the delivery policy is the idle/terminal one.
- No severity extraction: operant's note contract asks for `- ` lines with
  no severity vocabulary, so every note is severity-less (a nit) and can
  never steer/continue under any policy.

## OMP's semantics (what we port)

1. **Note contract WITH severity** (advise-tool schema): the advisor replies
   with `severity`-carrying notes. OMP's tool-based advisor emits via a tool;
   operant's prompt-based advisor must ask for it inline. New note contract:
   each line `- [severity] note` where severity ∈ {nit, concern, blocker};
   a bare `- note` is a nit (OMP: "Omit for a plain nit").

2. **Channel policy** (resolveAdvisorDeliveryChannel, simplified to
   operant's single-advisor + idle-post-run reality):
   - The agent is IDLE when notes arrive (run() returned before the side
     call finished — the 3.9s small-stack probe proves the timing).
   - A note at/above `continue_severity` (default `concern`, OMP's
     `advisor.continueSeverity` default) wakes the primary for ONE
     continuation so it addresses the advice.
   - A note below the threshold is PRESERVED as a visible card that enters
     the model's context with the next user turn (OMP preserve channel).
   - Streaming-state routing (aside/steer mid-run) is YAGNI for operant
     until the advisor reviews mid-run — notes structurally cannot arrive
     mid-run today. Not ported.

3. **Batching + boundary guidance** (#flushAdvisorBoundaryNotes): all notes
   from one review deliver as ONE merged message, newest-first, headed by
   the boundary guidance (the reviewed work already finished; treat notes
   critically, act only when valid, silently ignoring wrong feedback is
   acceptable). OMP shows the guidance on the first merged delivery of a
   session, then at most once every 50 turns. Ported as: guidance on every
   continuation (operant delivers at most one batch per run, and each
   continuation is a fresh context entry — cheap, and the 50-turn cadence
   exists to avoid per-delivery noise in a high-volume loop we do not have).

4. **Framing** (formatAdvisorBatchContent): `<advisory severity="…"
   guidance="weigh, don't blindly obey">note</advisory>` per note, XML-
   escaped. The primary's framing that this is advice, not instructions.

5. **Continuation latch** (#advisorContinuation): a batch steers at most ONE
   continuation. After a continuation runs, its own review may NOT trigger
   another (that is the cascade guard). Ported as: the continuation message
   is tagged `[ADVISOR_CONTINUATION]`; the advisor engine skips reviews of
   deltas whose user message carries the tag.

6. **Quarantine stays**: hazard regexes run BEFORE any delivery (they exist
   because notes are model-visible now).

## Where it lands in operant (the seams)

- `advisor.rs`: new note parser (severity + text), batch formatter with
  guidance header, channel decision (preserve vs continue), continuation
  tag. The `deliver` callback gains a second parameter/enum:
  `Delivery::Card(String)` (the old TUI rendering, still the surface for
  preserve) vs `Delivery::Continue(String)` (the formatted batch the model
  must see). The engine calls `deliver` ONCE per review with the merged
  batch, not per note.
- `agent/builders.rs::attach_advisor`: the delivery closure routes:
  - `Card` → `AgentEvent::BackgroundReview` (unchanged TUI path).
  - `Continue` → push the batch into the CONVERSATION as a user-role
    message + set the TUI continuation trigger.
- `run()` admission (turn_context.rs): the `[ADVISOR_CONTINUATION]` tag
  enters through `add_message` like the steer path — no dedupe collision
  with `already_added` (content differs from the last user message), and
  `record_user_turn` must NOT mark it as a user turn for rewind purposes…
    Actually simplest and correct: the continuation enters through
    `run()`'s normal user-query path (the TUI calls
    `submit_user_message(agent, "[ADVISOR_CONTINUATION] <batch>")`), so
    turn_context adds it as the query exactly like a steer-triggered
    follow-up. It IS recorded as a user turn — that is honest: it is an
    agent-initiated turn. Rewind semantics unchanged.
- TUI: `pending_advisor_continuation: Option<String>` on App; the
  BackgroundReview handler sets it when the summary carries the
  continuation marker; the main loop (adapter_types/tui_app.rs, where
  run_complete is drained) fires `submit_user_message` once the run is
  complete and the composer is idle. Cancelled runs (interrupted) do NOT
  auto-continue (OMP: user-stop suppression — a deliberate interrupt must
  not be auto-resumed by advice).

## Config

`[advisor] continue_severity = "concern"` (default; nit | concern |
blocker) — OMP's `advisor.continueSeverity` default. Documented in
`operant.example.toml` in the same change. `suppress_streak`,
`timeout_secs` unchanged.

## Tests (mutation-proven, same discipline as iter-1426)

1. Parser: `- [blocker] x` → (blocker, x); `- y` → (nit, y); `no notes` →
   empty; severity outside the vocabulary → nit (OMP: omit = nit).
2. Channel: nit under continue_severity=concern → Card; concern → Continue;
   continue_severity=nit → Continue for a nit; blocker always Continue.
3. Batch format: severity attribute present, note XML-escaped, guidance
   header on continuation batches only.
4. Latch: a continuation-tagged delta is skipped by the engine (no second
   side call) — the cascade guard.
5. E2E-ish (in-proc): run two turns against a mock client whose second
   reply includes an advisor-qualifying note; assert the conversation
   gained the `<advisory …>` message and `run()` was called twice.
6. TUI: the BackgroundReview handler sets pending_advisor_continuation for
   continuation-marked summaries; the run-complete path fires the
   submission; interrupted runs do not.

## Deliberately NOT ported (YAGNI, documented)

- Multi-advisor roster, WATCHDOG.yml, per-advisor models, quota reporting.
- Mid-stream aside/steer routing (notes cannot arrive mid-run in operant).
- syncBacklog catch-up waits (no concurrent review backlog exists).
- The 50-turn guidance cadence (volume is one batch per run).
- advisor.immuneTurns post-interrupt cooldown window: partially covered by
  "interrupted runs do not auto-continue"; a full cooldown is not earned
  yet.

## Acceptance

- The operator's configured small-stack advisor produces notes that the
  PRIMARY model sees and acts on (continuation), not just TUI cards.
- Live probe: with `[advisor] enabled = true`, a turn that draws a real
  concern from the route renders `⚑ advisor` card AND a follow-up turn
  whose transcript contains the `<advisory>` block.
- All tests green; gates pass; committed as one iteration; deployed.
