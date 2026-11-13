# Red team + ponytail audit — the session-continue feature (iter-2165)

**Subject:** `operant continue`, the TUI exit leftover, the restart notice, the `/continue`
runtime command, and the presence/deliberate-exit repairs — shipped as `745b4991`
(`feat(iter-2165)`), audited and repaired in `fix(iter-2182)`.
**Date:** 2026-11-13 · **Auditor:** the agent that shipped the feature, auditing its own work.
**Load assumed:** ONE operator, ONE local SQLite database, a TUI or a gateway, one process per
session in the common case.

Every verdict below was obtained by **running** the code, not by reading it. Where a check could
not be run here, the row says so instead of guessing.

---

## 1. What I claimed in iter-2165, and what is true

| Claim | Verdict |
|---|---|
| "Closing the TUI prints `operant continue <session-id>`" | **true** — the PATH binary prints it; the formatter is unit-tested |
| "`operant continue <id>` re-enters the TUI attached to that session" | **true by construction, not screen-verified** — `LaunchMode::Resume` → `pending_switch_session` → the frame loop's retarget → `set_session_id` rehydrates from SQLite. No TTY exists in this environment, so the screen was never seen |
| "A restart carries the command" (the gap the operator reported) | **true** — the boot notice is now a transcript annotation with the WHOLE id, not a status line with a truncated one |
| "`/continue` prints the command while the TUI is open" | **true** — unit-tested |
| "A deliberate `/exit` no longer reads as an interruption" | **true** — the branch and its regression test |
| "The session this process is attached to is CLAIMED, so `operant continue` refuses a live one" | **FALSE as shipped** — see finding 1 |
| "A refused claim keeps two writers off one session" | **FALSE as shipped** — see finding 2 |
| "The leftover says what happened" | **FALSE as shipped** — see finding 3 |
| "The verification is green" | **true for the targeted tests** (6 unit + 7 integration + 4 TUI). The full suite's single failure is `dashboard_server`'s "no daemon means not running" — a real `operant gateway run` daemon is live on this host, so that test's premise is false here (the file is clean and untouched) |
| "The deployed binary carries the change" | **true** — `operant continue --help` on the PATH binary answers; the binary is now a peer's rebuild (which contains iter-2165) |

---

## 2. The red team — where my own execution was wrong

### Finding 1 — the presence sync covered ONE of the two retarget paths (SEVERITY: HIGH)

**What I shipped:** the presence claim/release sat in the frame loop's switch drain
(`tui/app/mod.rs`), with a comment saying it covered "every hub switch and every `operant
continue` attach".

**What is true:** a retarget has TWO drains, and they alternate on whether a turn is streaming:

* the frame loop's drain (`app/mod.rs`, `!is_streaming`), and
* the outer drain (`tui_app.rs`, `run_pending_agent_actions`), which owns every retarget that
  could race a live turn.

The outer drain is where `/new` mints a fresh id and where a switch made *during* a turn lands.
Neither claimed the incoming session nor released the outgoing one. Consequences, both live:

* **the outgoing session stayed claimed by a process that no longer wrote there** — so
  `operant continue <old-id>` refused a session nobody was in, which is the exact command the
  feature exists to offer;
* **the incoming session was unclaimed** until its next turn (iter-2150's turn-start claim), so
  a second `operant continue` could attach to it in that window.

I read the frame loop's own comment — *"the outer drain owns every retarget that could race a
live turn"* — and patched the frame loop anyway. The comment was the warning.

**Repair:** one `App::release_session_claim` helper and one `App::claim_session` primitive, called
from **every** retarget site: the frame loop's drain, the outer drain's switch, and `/new`. The
seam is `App::apply_pending_switch`, extracted from the frame loop so the rule is testable.

### Finding 2 — the claim came AFTER the retarget, so a refusal was only a log line (SEVERITY: HIGH)

**What I shipped:** `switch_to(...)` (the retarget) ran first, then `claim_current_session()`
logged a refusal if the claim was refused.

**What is true:** the retarget IS the dangerous act — it is what puts a second writer on a
session's rows. A refusal logged after it changes nothing. The claim now comes **first** and is
authoritative: a refused claim **stops** the switch and says so.

**Repair:** the claim precedes the retarget at every site; the refusal is reported and the switch
is DROPPED (not retried — retrying would fight the other process for the same session).

### Finding 3 — the leftover said "Session saved" after an INTERRUPTED exit (SEVERITY: MEDIUM)

**What I shipped:** the exit leftover always opened with `Session saved — <title>`.

**What is true:** Ctrl+C is not a save. The session row says `end_reason = 'interrupted'` (the
marker writer already distinguishes the two — `App::exit_is_deliberate`), so the leftover and the
row disagreed about the same exit. An operator deciding whether to continue a half-finished turn
reads the sentence.

**Repair:** `cmd_continue::exit_state(deliberate)` names the headline — `Session saved` for
`/exit`, `Session was interrupted` for Ctrl+C.

### Finding 4 — the no-TTY fallback claimed a save that never happened (SEVERITY: LOW)

`operant continue` in a shell with no TTY printed the exit leftover, i.e. `Session saved — …` —
but nothing was closed: the session is at rest in the store and the command below is what
continues it from a terminal that has one. The headline is now `Session`.

### Finding 5 — the fleet renamed my iteration twice while it was in flight (SEVERITY: PROCESS)

`iter-2161` and `iter-2164` were both published by peers between my lookup and my push. The
commit was renumbered twice (`iter-2161` → `iter-2164` → `iter-2165`) and the collision is noted
in its body, per the "duplicate labels are append-only" rule. `origin/main` carries exactly one
`iter-2165`. A peer has since rebuilt and redeployed the binary; it contains my change (checked
on the PATH binary), but it is THEIR artifact, not mine.

---

## 3. The ponytail audit — bloat, redundancy, missing checks

| # | Finding | Verdict |
|---|---|---|
| P1 | **Two formatters for one output.** `format_exit_leftover` was a wrapper around `format_continue_hint` — a second name for the same string, one edit away from drifting | **repaired**: the wrapper is deleted; `exit_state` names the headline and `format_continue_hint` formats it. One formatter, four call sites (exit, restart, runtime, no-TTY) |
| P2 | **Two copies of the claim block** — iter-2150's `claim_current_session` and my inline `match claim_session_here(...)` | **repaired**: one primitive (`App::claim_session`, returns a verdict) and one release helper; the peer's helper delegates |
| P3 | **Three surfaces printing the same command — redundant?** | **not redundant, checked**: each is the only one visible at its moment. At exit the TUI is gone (the leftover is the only trace); at restart the operator has no scrollback yet (the annotation is the only trace); at runtime the other two have not happened (`/continue` is the only way to ask) |
| P4 | **`operant sessions resume` vs `operant continue` — the same thing twice?** | **not redundant**: `resume` is the free re-orient (a recap), `continue` is the action (attach). Different verbs, different costs |
| P5 | **A new dependency or file for the formatter?** | **no** — the formatter lives in the module that owns the command, and no dependency was added |
| P6 | **Missing check: the claim's refusal must stop the retarget** | **added** (`a_switch_to_a_session_another_window_owns_is_refused`): a live foreign owner (pid 1) refuses the claim, and the test asserts the agent was NOT retargeted |
| P7 | **Missing check: the exit headline must distinguish the two exits** | **added** (`the_exit_headline_distinguishes_a_deliberate_close_from_an_interruption`) |
| P8 | **The retarget rule was untestable** — it lived inline in the frame loop | **repaired**: `App::apply_pending_switch` is the rule, and the frame loop calls it |
| P9 | **A stale claim left behind by a crash** | **not a gap**: `prune_dead_presence` drops records whose pid is gone, so a SIGKILLed process releases its claim on the next claim |

---

## 4. The operational gaps that remain open (and why)

1. **The TUI's own screen at exit and at restart is not screen-verified here.** This
   environment has no TTY, and the headless simulator drives `run_headless`, which has no
   terminal and (by design) skips the presence sync. The leftover's TEXT is covered by the same
   formatter the integration test asserts, and the attach reuses the hub's existing retarget; the
   wiring is compile-checked and unit-tested, not screen-checked. **A human on a real terminal
   should confirm the annotation renders above the composer.**
2. **The presence WRITE side is only reachable from an interactive TUI.** The read side is proven
   end to end (a live record makes `operant continue` refuse to fork the session).
3. **The gateway's interrupted-session notice names the in-gateway path** (`/sessions`, `/resume
   <n>`) and never the shell command. That is deliberate — a gateway session id continued from a
   shell is the same row, but the gateway's own resume is the shorter path from where the operator
   is. **Not changed.**
4. **`operant continue` refuses a live session but does not say WHICH window holds it.** The
   registry records a pid, not a window title. Cosmetic; the operator can find the other process.

---

## 5. What the repair changed

| File | Change |
|---|---|
| `crates/operant-cli/src/tui/app/herdr_lifecycle.rs` | `claim_session(id) -> bool` (the primitive, with a verdict) + `release_session_claim(id)`; iter-2150's `claim_current_session` delegates |
| `crates/operant-cli/src/tui/app/mod.rs` | the frame loop's switch drain extracted into `App::apply_pending_switch()` — claim BEFORE the retarget, release the outgoing one, refuse and report when the claim is refused |
| `crates/operant-cli/src/tui/adapter_types/tui_app.rs` | the OUTER drain's switch claims before the retarget and releases the outgoing session; `/new` releases the outgoing session; the exit leftover's headline follows the exit's nature |
| `crates/operant-cli/src/cmd_continue.rs` | `exit_state(deliberate)`; `format_exit_leftover` deleted (one formatter); the no-TTY fallback's headline |
| `crates/operant-cli/src/tui/app/tests.rs` | the refused-switch regression test |
| `crates/operant-cli/src/cmd_continue.rs` (tests) | the deliberate-vs-interrupted headline test |

**Verification:** `cargo check -p operant-cli` clean · `cargo clippy -p operant-cli --all-targets`
clean · 6 unit tests (cmd_continue) + 7 integration tests (`tests/continue_session.rs`) + 5 TUI
tests green · the full binary suite green except the environment-bound dashboard test named above.
