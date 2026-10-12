# Telegram agentic-loop UX fixes

Date: 2026-09-30. Status: diagnosed, bug 1 fixed and unit-verified.
Scope: the agentic loop reachable from Telegram, not the TUI/WebSocket paths.

## How to read this

Every item states the observable symptom, the mechanism with `file:line`, and how
it will be proven fixed. Items are ordered by *irreversibility*, not by size: a
leaked credential is worse than a missing feature because rotating it takes time
you may not have.

Two corrections to the investigation itself are recorded in "What was wrong with
the first diagnosis", because both shaped the plan.

---

## Phase 0 — the credential leak (do first)

### P0.1 Rotate both leaked Telegram bot tokens — **owner action, blocking**

**Symptom.** `~/.operant/logs/gateway.log` contains two distinct bot tokens in
plaintext, 30 occurrences total (20 + 10), inside `getUpdates` and `getMe` URLs.

**Mechanism.** Some error paths render the request URL verbatim. Other lines in
the same file correctly show `[REDACTED TELEGRAM TOKEN]`, so a redaction filter
exists and is applied inconsistently. `redact_secrets` (`config.rs:1076`) defaults
to `Some(true)` — the default is not the problem.

**Why first.** The tokens are in a 46 MB file, in shell history, and in agent
transcripts. They must be treated as compromised regardless of any code fix.

### P0.2 Redact tokens embedded in URLs

**Symptom.** The existing redactor misses the credential when it appears inside a
URL path rather than as a standalone value.

**Fix.** Extend the redaction pass to match the Bot API URL shape
(`/bot<digits>:<token>/`) and rewrite it to the same `[REDACTED TELEGRAM TOKEN]`
form already in use, so the two paths agree.

**Test.** A log line built from a real-shaped `getUpdates` URL must render with the
token removed. Assert the raw token does not appear in the captured output — not
that the line "looks redacted".

### P0.3 The log has no rotation

`gateway.log` is live and unrotated. Whatever the retention intent, it is
currently unbounded. Out of scope to fix here; recorded so it is not lost.

---

## Phase 1 — the three reported bugs

### P1.1 A mistyped command is answered as a prompt — **DONE, unit-verified**

**Symptom.** `/arop` was answered by the model with "that isn't a command I have".
Those strings are not in the codebase; the model improvised a plausible rejection,
which made the failure look like a working handler.

**Mechanism.** `resolve_command` returned `Option`. The `?` at
`gateway_commands.rs:495` collapsed "not a command" and "command name not
recognised" into the same `None`. The gateway read `None` as "therefore a prompt".
Root cause is a type that could not express three states, so the caller could not
distinguish them either.

**Fix.** Replaced the `Option` with a three-state `CommandMatch`
(`NotACommand` / `Unknown` / `Found`). The gateway matches all three arms: `Found`
dispatches, `Unknown` replies truthfully and never reaches the model, `NotACommand`
falls through to the agent. No second `starts_with('/')` check was added at the
call site — the type carries the distinction instead.

**Also fixed in passing.** `/stop@my_bot`, the form Telegram sends when a command
is invoked by tapping the bot's name in a group, was also broken: the suffix
defeated the registry lookup. The new parser strips it.

**Tests.** 17 passed / 0 failed. The pre-existing `test_resolve_unknown` asserted
`resolve_command("/nonexistent").is_none()` — it **pinned the bug**. It is replaced
by `test_resolve_unknown_is_not_prose`, which asserts the two cases are not equal,
plus cases for the exact `/arop` text, the `@botname` form, and a bare `/`.

### P1.2 `/stop` reports success and cancels nothing

**Symptom.** `/stop` during a streaming turn neither stopped the stream nor raised
an error. The user was left waiting.

**Mechanism, two independent gaps.**
1. The handler (`gateway_commands.rs:639-654`) does exactly two things: write
   `interrupted=true` session metadata, and return the literal string
   `"⏹️ **Stopping current agent turn."`. That metadata key is written by one line
   and read by **zero** sites in the workspace. It is a stub that reports success.
2. The live turn registry is `turn_inflight: HashMap<String, ()>` — a **unit**, not
   a handle — and the turn is a `tokio::spawn` whose handle is discarded
   (`gateway_runner.rs:2158`). There is nothing to signal even in principle.

**Fix.** The agent already has the right primitive and it is already wired into the
run loop: `Agent.interrupt_flag` is an `InterruptFlag` (`agent/builders.rs:78,143`),
checked at `agent/run.rs:407,549,648`, and it has `reset()`, so a latch cannot kill
the agent permanently. Store the flag in `turn_inflight` instead of `()`, and have
`/stop` trigger it.

**Test.** Start a turn, `/stop` it, assert the run returns an interrupt and that the
next turn still runs — the reset path is the part that would otherwise turn a fix
into a permanent outage.

**Honest reporting.** `/stop` must say what actually happened: "stopped a running
turn" versus "nothing was running". The current code claims the first unconditionally.

### P1.3 Continuation is lost across a restart — the architectural one

**Symptom.** After a gateway restart, a bare `continue` produced an agent with no
memory of the work in progress. The agent itself reported the cause honestly: five
prompts, no replies.

**Mechanism.** Four disjoint session-id namespaces coexist, and the one holding the
real work is unreachable:

| Namespace | Written by | Holds | Survives restart |
|---|---|---|---|
| `gw_<hexhash>` | `gateway_runner.rs:436` | user + final assistant text only, never tool calls | yes, reloaded (last 20, tool rows skipped) |
| `sess_<uuid>` | `turn_context.rs:109` | the **full tool trajectory** | no — orphaned every turn |
| `20260929_164551_08e58525` | `PersistentSessionStore` | nothing; return value discarded at `gateway/mod.rs:383` via `let _ =` | n/a |
| operant-infra's | JSONL/SQLite store | routing metadata | yes |

The lost verification sweep is in `sess_<uuid>`: 163 messages, 39 `undercurrent`
mentions. It was orphaned by this line:

```rust
.unwrap_or_else(|| format!("sess_{}", uuid::Uuid::new_v4()))
```

The Telegram gateway **never sets `persistent_session_id`**. The only production
caller of `with_persistent_session` is `operant-gateway/src/ws.rs:373` — the
WebSocket/TUI path. Confirmed empirically: every `sess_*` session contains exactly
one user message, i.e. one per turn, every turn.

So the gateway's own `gw_*` log *does* persist and *is* reloaded on restart, but it
records only user text and the final assistant text. The reloaded "history" is a
skeleton of unanswered prompts. That is precisely what the agent reported seeing.

**Fix.**
1. Set `persistent_session_id` from the session id the gateway **already** computes
   deterministically, so the tool trajectory lands in the session that survives.
2. Delete the per-turn throwaway uuid, so there is one namespace instead of two.
3. Make the reload path the single source of truth for in-memory history.
4. Have the `PersistentSessionStore` stop writing a third id namespace, or be
   removed — it is currently decorative.

**Data.** The user has stated the current database is disposable and everything in
it is active development, so no migration is required. The orphaned transcripts are
still worth consolidating once so the in-flight sweep is not lost; that is a
convenience, not a correctness requirement, and is not a reason to delay the fix.

### P1.4 `/resume` exists but does not resume

`/resume` is registered with alias `continue`, but it requires the `/` prefix
(command dispatch requires `trimmed.starts_with('/')` at `plugins/mod.rs:227`), it
requires an explicit session id, and it only **prints** a message. So `continue`
with no slash never matches it, and `/resume` never actually resumes.

**Fix.** Make it rehydrate the target session into the agent and continue the turn.
Then decide explicitly whether a bare `continue` should be an alias, because
silently consuming a word users type as ordinary prose is a worse outcome than
making them type the slash.

---

## Phase 2 — why all three felt worse than they were

These are not in the original report. They are the reasons a correct fix will still
leave the experience poor, so they belong in the same plan.

### P2.1 The agentic loop cannot be debugged from its own logs

Across all **879,057** lines of `gateway.log` there is not one turn-started,
inbound-message, or dispatch event. Only tool registration, poll heartbeats and
warnings. The bot demonstrably produced output at 11:57; that turn is invisible.

This is why the first diagnosis was wrong, and it will make the next one wrong too.

**Fix.** Log turn start (session key, message id, character count), turn end
(outcome, duration, whether interrupted), command dispatch, and unknown-command
rejection. Session keys are PII-bearing, so the existing redaction pass must cover
them.

### P2.2 Provider death is silent and slow

47 occurrences of `Stream error, error: Network error: error decoding response
body`, clustered exactly on the failed `continue` turns, in runs of four spaced
60 s apart (12:15:45, 12:16:45, 12:17:45, 12:18:45). The retry ladder burns about
four minutes and then the turn ends with the prompt saved and no reply.

From the user's side: send a message, stare at nothing for four minutes, get
nothing, learn nothing. This is the worst single behaviour in the current system
and it is invisible.

**Fix.** Bound the retry budget in wall-clock terms, and tell the user what happened
when it is exhausted — the provider stream died, here is the error. Silence after a
known failure is the thing that made this look like a hang.

### P2.3 A queued message cannot be followed up on

Ordinary messages go to `turn_queue.push_back` and are answered only after the
running turn finishes, so a question sent mid-turn waits minutes. That is defensible
in isolation, and it is also why the broken `/stop` had no workaround: it was the
only escape hatch, and it did not work. P1.2 is what makes this acceptable.

---

## Phase 3 — proving it

Unit tests are necessary and not sufficient. Every item above that can be observed
from Telegram will be driven over the `tg` CLI against the live bot:

1. `/arop` — must return the unknown-command reply, and the model must not be called.
2. `/stop` during a long streaming turn — stream must end early; a follow-up turn
   must still work.
3. Restart the gateway mid-task, then continue — the agent must reference the actual
   in-flight work, not ask what to do.
4. Kill the provider mid-turn — the user must be told, within a bounded time.

A big test, per the standing instruction, covering complex agentic capacity rather
than single commands. The gateway exposes no HTTP surface, so `tg` is the only way
in and the test driver.

---

## What was wrong with the first diagnosis

Two of three parallel investigations reported directly contradictory conclusions
about cancellation: one that it was entirely absent, one that it was fully wired
with a working `CancellationToken`. Both read real code.

The reason is that `operant-channels/src/orchestrator/` — 14,094 lines, containing a
correct cancellation implementation — is dead code. `run_message_dispatch_loop` and
`start_channels` have no callers outside their own module, and
`operant-cli/Cargo.toml:21` says in its own comment that the crate "never names it".
The repo's `BUGS.md:558` already records this as dead-linked.

The live Telegram path is `operant-cli/src/gateway_runner.rs`, with commands from
`gateway_commands.rs`. Roughly 40% of the second report described unreachable code.

The generalisable lesson, and the reason P2.1 is in this plan: **in this workspace,
confirm which stack is live before analysing a bug in it.** The dead stack is large,
well-commented, and more correct than the live one.
