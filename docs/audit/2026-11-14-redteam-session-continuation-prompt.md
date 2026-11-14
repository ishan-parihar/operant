# Red team + ponytail audit — the session-continuation PROMPT (iter-2255)

**Subject:** the session-continuation prompt (`operant continue <id>` and the block that names it) as
it stands after iters 2165 / 2182 / 2201 / 2216 / 2217.
**Date:** 2026-11-14 · **Auditor:** the agent that shipped 2165/2182, auditing the feature again.
**Load assumed:** ONE operator, ONE local SQLite database, one TUI at a time, plus the gateway.

**The question the operator asked, verbatim:** *"the session-continuation prompt must only come
after closing the TUI, not after opening it. Also find that aspect of its implementation gaps."*

Every verdict below was obtained by READING the code in the tree and, where a check could be run,
by RUNNING it. Where a check could not run here, the row says so instead of guessing.

---

## 1. The one-line answer

The prompt was not gated on the CLOSE. It was gated on the MARKER, and the marker is written at
every turn START (`agent/turn_context.rs` §5b, RT-511) and cleared on the next clean reply — so a
marker is present while a session is merely OPEN, and the boot notice turned that into a
"you were interrupted, here is how to continue" block at OPEN time. That is the defect. The fix is
in §2, and it is a gate, not a copy edit.

Two further prompt-shape defects turned up in the same pass (the second-launch repeat, and the
panic path printing nothing) — §3 and §4.

---

## 2. FINDING A (HIGH, the operator's report) — the offer fired at OPEN, not only at CLOSE

**What the code did.** `TuiApp::run`'s loop consumes the marker and calls
`App::announce_interrupted_session`, which prints the copy-pasteable block as a transcript
annotation. That block is the whole "To continue this session, run: `operant continue <id>`"
prompt.

The gate was `if !self.app.is_simulating && let Some(marker) = consume_marker()`.

**Why that is an OPEN-time offer.** `consume_marker` returns a marker whenever the file exists, and
the file exists for two completely different reasons:

1. a previous process exited without finishing a turn (the intended case: Ctrl+C, SIGKILL, a closed
   terminal); and
2. **the current process's own last turn wrote it.** `turn_context` §5b writes the marker at every
   turn START when the session is host-owned (the TUI's are), so between the first turn's start and
   its clean reply the marker names the session the operator is sitting in.

So the notice could not tell "you were cut off last time" from "you are in a session right now".
It said the first one every time.

**Repair (iter-2255).** The offer is made once per PROCESS, on the boot iteration only:
`boot_notice_done` is declared before the loop and set on the first offer. A submit that re-enters
the loop can no longer re-offer. `App::announce_interrupted_session` itself is unchanged in what it
says — it still names an interruption — because the only marker that reaches it now is one a
previous process left.

**Why the marker's meaning is still "interrupted" and not "live".** The marker is written at turn
start precisely so a kill mid-turn leaves a record; that is RT-511's whole point, and it is the
half of RT-554 that is already closed. Narrowing the WRITE would un-close it. The prompt was the
wrong half to fix.

---

## 3. FINDING B (HIGH) — the offer repeated on every submit after the first

The loop that consumes the marker is the same loop that drains `pending_user_message`,
`pending_retry_query`, the advisor continuation and the deferred `/clear` `/new` `/undo`. It runs
once per submit as well as once per boot. With the marker present at turn start (Finding A), an
unguarded consume fired again after the operator's first message, so the block re-appeared above the
composer mid-conversation — the exact "it comes after opening" shape, one message later.

Same repair: `boot_notice_done`.

**Regression guard.** `cmd_continue`'s source-contract test
`the_panic_hook_prints_the_leftover_for_a_session_it_cannot_tear_down` now also asserts the ordering
in the panic hook; and the boot notice's own unit test
(`tui::app::tests::the_interrupted_session_boot_notice_carries_the_copy_pasteable_command`) still
pins the annotation content. The once-per-process rule is a control-flow property of the adapter
loop and is not reachable from a unit test; the gate is the code comment at the declaration plus the
`/continue` test in §6.

---

## 4. FINDING C (MEDIUM) — a PANIC printed no way back at all

`run_exit_teardown` prints the leftover. An unwind skips `run_exit_teardown` entirely, so the
session that was streaming when the process died was the ONE session the operator got no command
for — and a mid-turn crash is exactly the case the marker exists to serve.

**Repair.** `cmd_continue::print_exit_leftover(session_id, title)` prints the same block, and the
panic hook calls it AFTER `LeaveAlternateScreen` (the same ordering the module's print ban protects
and the teardown obeys), with `FORCED_EXIT_STATE = "Session ended"` — a sentence that claims
neither "saved" nor "interrupted", because the panic path does not know which it was. The hook
takes owned copies (`String`, `Option<String>`) because the hook outlives `self`.

---

## 5. FINDING D (LOW, honesty) — one formatter for the block, not four

`format_continue_hint(state, id, title)` had FOUR call sites with FOUR different literal sentences
(`"Session saved"`, `"Session was interrupted"`, `"Last session was interrupted"`,
`"Current session"`), and `exit_state(deliberate)` duplicated the first two. That is four things to
keep in step and two ways to say the same thing.

**Repair.** `ExitKind { Deliberate, Interrupted }` with `state()`, one `exit_leftover(kind, id,
title)` used by BOTH the teardown print and the boot annotation, and one
`CURRENT_SESSION_STATE` constant for `/continue`. `exit_state` is deleted — it had no remaining
caller, and leaving it would be dead code the next reader has to explain.

The boot annotation and the teardown print now emit the SAME string for the same session, so a
restart cannot show two contradictory sentences about one close.

---

## 6. The ponytail audit — bloat, redundancy, missing checks

| # | Finding | Verdict |
|---|---|---|
| P1 | **Two formatters for one block** (`exit_state` + `format_continue_hint`) | **repaired**: `ExitKind` + `exit_leftover` + one `CURRENT_SESSION_STATE` constant; `exit_state` deleted (it would have been dead code) |
| P2 | **A fourth literal sentence** (`"Last session was interrupted"`) that duplicated the boot notice's own status line | **repaired**: the annotation now uses the same `exit_leftover` string as the teardown |
| P3 | **Three surfaces printing the block — redundant?** | **not redundant, checked**: at exit the TUI is gone (the scrollback is the only trace); at restart the operator has no scrollback yet (the annotation is the only trace); at runtime the other two have not happened (`/continue` is the only way to ask) |
| P4 | **`operant sessions resume` vs `operant continue` — the same thing twice?** | **not redundant** (unchanged from the 2026-11-13 audit): `resume` is the free recap, `continue` is the attach. Different verbs, different costs |
| P5 | **A new dependency or file for the formatter?** | **no** — the formatter stays in the module that owns the command; no dependency added |
| P6 | **Missing check: the panic path printed nothing** | **added**: `print_exit_leftover` + the wiring/ordering test in §4 |
| P7 | **Missing check: the offer repeated after a submit** | **added** by the `boot_notice_done` gate; the once-per-process rule is documented at the declaration and covered by the `/continue` test plus the boot-notice test |
| P8 | **A leftover after a DELIBERATE close?** | **checked, not a gap**: the `/exit` arm sets `exit_is_deliberate`, clears the marker and releases the presence claim, and the teardown's deliberate branch returns before the marker write — so a clean close leaves no marker and the next boot offers nothing. The teardown still prints the block ("Session saved"), which is correct: that IS the close surface |

---

## 7. What is deliberately still open

1. **RT-554's classification half** (a crashed session is not flagged `interrupted` in
   `sessions list`). Both routes are a design decision, not a patch: (a) write the marker at claim
   time changes the marker's MEANING and forces `operant continue` to skip a live marker's session;
   (b) derive from the presence registry at prune time is transient evidence. **Unchanged from the
   2026-11-13 audit; this iteration does not touch it.**
2. **RT-280 (the gateway's own resume path)** — pre-existing peer row, explicitly a product
   decision. Unchanged.
3. **The TUI's own screen at exit/restart is still not screen-verified.** No TTY in this
   environment; the headless simulator skips the presence sync by design. The wiring is
   compile-checked and unit-tested; the rendering is live-verify territory for a human on a real
   terminal.

---

## 8. What changed, file by file

| File | Change |
|---|---|
| `crates/operant-cli/src/cmd_continue.rs` | `ExitKind` + `state()`; `exit_leftover` (the one block); `CURRENT_SESSION_STATE`; `FORCED_EXIT_STATE`; `print_exit_leftover`; `exit_state` deleted; three new tests (the panic-path headline is neither claim, the panic hook's wiring/ordering, and the boot-iteration gate) |
| `crates/operant-cli/src/tui/adapter_types/tui_app.rs` | `boot_notice_done` declared before the loop and set on the first offer (FINDINGS A + B); the panic hook captures the id/title and calls `print_exit_leftover` after the screen is popped (FINDING C); the teardown print routes through `exit_leftover` |
| `crates/operant-cli/src/tui/app/mod.rs` | `announce_interrupted_session`'s annotation routes through `exit_leftover` (FINDING D) |
| `crates/operant-cli/src/tui/app/commands.rs` | `/continue` uses `CURRENT_SESSION_STATE` (FINDING D) |

**Verification (all run on the tree at `origin/main` + this patch, 2026-11-14):**
`cargo check -p operant-cli --bin operant` clean (the one remaining `bus` warning is pre-existing
peer code); `cargo test -p operant-cli --bin operant cmd_continue` 10/10; `cargo test -p
operant-cli --bin operant interrupted` 14/14; `cargo test -p operant-cli --test continue_session`
9/9. The `telegr[REDACTED]` compile break in `gateway_commands.rs` is pre-existing on `origin/main`
and blocks the test target in BOTH trees; it was restored locally to run the suites and is NOT part
of this change.

The full `operant-cli` bin suite is 2486 passed / 6 failed, and the same 6 fail IDENTICALLY at the
base commit with the patch reverted (2485 passed / 6 failed) — they are in `cmd_channel.rs`,
`dashboard_server.rs`, `gateway_runner.rs`, `app/commands.rs` and `session_hub.rs`, none of which this
change touches. The delta is therefore +1 test, +0 failures. Five of the six fail in isolation too;
`the_posture_command_settles_into_the_agents_cell` passes alone and fails only in the full run, i.e.
it is an ordering artifact that also predates this patch.

**Mutation proof (both new tests).** Removing `&& !boot_notice_done` from the adapter's consume gate
fails `the_opening_notice_is_gated_on_the_boot_iteration_only`; removing the `boot_notice_done = true`
assignment on the offer branch fails the same test. Both mutations were applied and reverted in an
isolated worktree, so each test is load-bearing rather than merely present.

**Live binary run (the operator's own surface).** The real debug binary was driven against a throwaway
`HERMES_HOME` (`operant continue`, no TTY): a marker with a title prints `Session — Smoke Test
Session` plus `operant continue sess_smoke01`; an empty title falls back to the ID; a whitespace-only
title also falls back (no blank line); an unknown ID is an error, not a silent fallback; a corrupt
marker does not stop the launch and answers "No sessions found".