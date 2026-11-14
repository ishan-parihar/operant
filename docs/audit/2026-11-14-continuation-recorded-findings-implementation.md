# Implementation of the iter-2321 audit's recorded findings (iter-2348)

**Subject:** the three SHOULD FIX / NICE TO HAVE findings that
`docs/audit/2026-11-14-redteam-continuation-execution.md` recorded and did not fix, plus the lean cut.

**Why this exists.** The audit fixed the two MUST FIX findings (RT-564 / RT-565). This pass executes the
three that were written down and left: the panic leftover (3), the marker's ownership (4), the O(n) scan
(5), and the one-use constant (6).

---

## 1. The panic leftover was ALWAYS empty — iter-2255's panic fix never fired (RT-569)

**What was recorded.** "The panic hook captures `(session_id, title)` ONCE, in `TuiApp::enter`, so it
goes stale after an in-TUI session switch."

**What was actually true — worse than recorded.** The capture was not stale, it was *always `None`*:

- `TuiApp::run` installs the panic hook at line ~308.
- `self.app.core_session_switch` is set at line ~502 (interactive) — **after** that.
- So `self.app.core_session_switch.as_ref().and_then(|s| s.session_id())` evaluated to `None` every
  time, `panic_leftover` was `None`, and the hook printed **nothing at all**.

The panic path that the whole mechanism exists for — a session streaming when the process dies — had
never once produced the way back. iter-2255 shipped it, and its test passed, because the test only
asserted that the hook *calls* `print_exit_leftover`. A wiring test cannot tell that the value being
printed is empty.

**Fix.** A shared `Arc<Mutex<Option<(String, Option<String>)>>>`:
- created empty where the hook is installed (the honest state — there is no session yet),
- written the moment the agent is attached and an id can exist,
- refreshed on every frame-loop pass from the agent's live `session_id()`.

That third write also fixes the finding as originally recorded: after an in-TUI session switch the cell
holds the session the operator is **in**, not the one they left.

The hook takes the lock only for the instant it reads, and treats a poisoned lock as the intended
recovery (the panic is why we are here) — a hook that deadlocked would replace a panic message with a
frozen terminal.

**Pinned by** `the_panic_hook_resolves_the_session_id_lazily`. Mutation-proven: restoring the exact
original shape (eager read of `core_session_switch`, `panic_leftover` frozen, hook reading the frozen
copy) makes it fail.

## 2. The gateway daemon ate the TUI's resume offer ~every 1.4 minutes (RT-570)

**What was recorded.** "Four `consume_marker` callers, one of them guaranteed to lose."

**What was actually true — a measured production failure.** The gateway runs under `Restart=always`.
Measured on this host:

```
journalctl --user -u operant-gateway --since "24 hours ago" | grep -c "Started Operant"
1017          # start/stop events in 24 hours  =>  one every ~1.4 minutes
NRestarts=0   # (the service is restarted by deploys and restarts, not counted here)
```

Each boot called `consume_marker()`, which **deletes** the file. So after a crash, the gateway's next
restart — on average about 84 seconds later — silently took the marker. The TUI's boot notice, which
is the surface a returning operator actually reads, found nothing and printed nothing.

**The operator-visible consequence:** the TUI boot notice from iters 2255/2256 was almost always dead
code on this machine. The operator's original report ("the prompt must only come after closing the
TUI") was fixed in a code path that had usually already been emptied.

**Fix.** Both boot surfaces read instead of consume:
- `tui/adapter_types/tui_app.rs`: `read_marker()`
- `cmd_gateway.rs`: `read_marker()`

The marker is still retired — by whoever **acts** on it: a clean turn's `clear_marker_for`, or `/exit`'s
`clear_marker`. A passive notice no longer has the power to silence every other notice.

**Pinned by** `no_boot_notice_consumes_the_marker`, which asserts both surfaces read and neither
consumes. Mutation-proven both halves: flipping either back to `consume_marker()` fails it.

## 3. An O(n) scan to answer a yes/no about one id (RT-571)

**What was recorded.** "`resolve_target_session` lists up to 10,000 sessions to check one id."

That was my own doing: the RT-564 fix needed "does this id exist" and reached for
`list_sessions(10_000) + find`, twice. It reads every row and builds a `Vec` to answer a question
`sessions.id` (the `PRIMARY KEY`) answers directly.

**Fix.** `Database::get_session_id_and_title(&self, id) -> Result<Option<(String, Option<String>)>>` —
an indexed point lookup returning the title as well, because both callers want it. Both call sites in
`resolve_target_session` use it.

**Pinned by** `get_session_id_and_title_is_a_point_lookup` (hit and miss).

## 4. `FORCED_EXIT_STATE` earned one use (RT-572, lean)

A `pub const` with exactly one real reader in the same file. Inlined as a local `const PANIC_EXIT_STATE`
inside `print_exit_leftover`, carrying the reasoning that was on the public item; the two assertions
that compared against it now compare against the literal. 4 mentions -> 2, and one less public item.

---

## What did NOT change

- The marker's **meaning** is untouched: it is still written at every turn start (RT-511) and cleared
  on the next clean reply. Only *who may delete it* changed.
- `ExitKind`, `exit_leftover`, `CURRENT_SESSION_STATE` and the four surfaces' sentences are as
  iter-2321 left them.
- `is_simulating` still gates the TUI's boot notice, so `operant tui debug simulate` leaves no state.

## Verification

- `cargo check -p operant-cli --bin operant` — **clean** (0 errors).
- `cargo test -p operant-cli --bin operant cmd_continue` — **13/13** (was 11; +2: the lazy-capture and
  boot-read tests).
- `cargo test -p operant-cli --test continue_session` — **11/11**.
- `cargo test -p operant-cli --bin operant interrupted` — **14/14**.
- `cargo test -p operant-core --lib get_session_id_and_title` — **1/1**.
- `cargo clippy -p operant-cli -p operant-core --all-targets` — no finding in any file I touched.
- `./scripts/genome lint` — **0 errors**.
- Full `operant-cli` bin suite with this patch: **2558 passed / 7 failed**. At `origin/main` with the
  patch stashed: **2556 passed / 7 failed** — the same seven names. Delta: **+2 tests, +0 failures.**
  All seven are in `cmd_channel.rs`, `dashboard_server.rs`, `gateway_runner*` and `app/commands.rs`,
  which this change does not touch; `advisor_card` passes in isolation and fails only in the full run
  (an ordering artifact), `cmd_channel` fails in isolation too and is pre-existing.

**Not checked:** the panic hook was not exercised under a real panic — the process-global hook cannot
be driven from a test, which is exactly why iter-2255's wiring test could not catch this. The lazy read
is pinned structurally (the capture must not name `core_session_switch`) rather than behaviourally.