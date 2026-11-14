# Red team + ponytail audit — the continuation prompt's EXECUTION (iter-2324)

**Subject:** the session-continuation prompt as it stands after iters 2165 / 2182 / 2201 / 2216 /
2217 / **2255** / **2256**, audited for feature gaps, operational gaps and redundancy.

**Why this exists.** iter-2255 fixed the timing (the prompt fired at OPEN, not only at CLOSE) and
iter-2256 pinned it with a mutation-proven test. This pass asks the other question: now that the
block prints at the right MOMENT, is what it prints actually TRUE and USABLE? The answer was no in
two places, one of them on the operator's own live machine.

---

## What this repo does

operant is a Rust coding agent with a terminal UI, a CLI, and a multi-platform gateway. It persists
chat sessions in SQLite. When the process ends without a clean goodbye (a Ctrl+C, a killed terminal,
a panic), the next launch is supposed to say "that session was cut off — here is the command that
reopens it."

Load assumed: one operator at a terminal, plus a gateway daemon and cron jobs on the same home. Not
a multi-tenant server. Every finding below is judged against that.

The prompt is printed by four surfaces that must agree:

| surface | where | sentence |
|---|---|---|
| TUI exit leftover | `tui/adapter_types/tui_app.rs` teardown, after the screen is popped | `Session saved` / `Session was interrupted` |
| next-launch transcript annotation | `App::announce_interrupted_session` | `Session was interrupted` |
| `/continue` while the TUI is open | `tui/app/commands.rs` | `Current session` |
| `operant continue` with no TTY | `cmd_continue::handle_continue_command` | `Session` |

---

## MUST FIX

### 1. `continue` offers a command that cannot work (`cmd_continue.rs:215-242`)

- **What this is:** `resolve_target_session` read the interrupted-session marker and trusted its
  session id as an answer, without checking that the session still exists in the store.
- **Problem:** the marker is a single-slot FILE that outlives the DB row it names. Four ordinary
  operator actions leave a marker pointing at nothing:
  - `operant sessions delete <id>` (`cmd_sessions.rs:254`) deletes the row and does not clear the marker;
  - `operant sessions prune` (`cmd_sessions.rs:405`) same;
  - `merge_sessions` (`database.rs:3365`) deletes the source row;
  - a gateway socket session (`bind_slot_to_session`, `gateway_runner.rs:3195`) ignores the
    `save_session` result, so a failed write leaves a host-owned id with no row — and `host_owned`
    is exactly the condition under which `turn_context.rs:287` writes the marker.

  So `continue` printed `operant continue socket-session`, and running that command answered
  `Error: Session not found: socket-session`. An offer the operator cannot accept, repeated on
  every launch until some other surface consumed the file.

  **This is not hypothetical: it was the operator's live state.** `~/.operant/sessions/interrupted-session.json`
  held `{"session_id": "socket-session", "title": "hello"}` and `operant sessions list` did not
  contain that id. `continue` printed the dead command; the fix below now answers with the most
  recent real session instead.

- **Fix applied:** the marker is a HINT, not an authority. It wins only when `list_sessions`
  contains its id; otherwise the answer degrades to the same recency fallback the no-marker case
  uses, and a `warn!` records why. The marker's own TITLE still wins over the store's when the row
  exists (an existing test pins that, and it is right: the marker's title is what the operator was
  looking at).
- **If we skip it:** every operator who deletes or prunes a session keeps a permanently broken
  resume offer, and the one command the whole feature exists to print is the one that fails.

### 2. Two label rules named one session two ways on one screen (`tui/app/mod.rs:1784-1788`)

- **What this is:** the boot status line and the transcript annotation under it label the same
  session with two different fallback rules.
- **Problem:** with no title, the status line used `marker.session_id.chars().take(8)` while
  `format_continue_hint` used the whole id. One screen showed:

  ```
  Last session was interrupted: sess_177 — /session to resume it.     <- status line
  Session was interrupted — sess_177efb5f-1713-46d5-904b-168ab793052a  <- annotation
    operant continue sess_177efb5f-1713-46d5-904b-168ab793052a
  ```

  Measured with a standalone reproduction. The operator comparing the line they read to the command
  they are about to copy cannot tell whether those are the same session.

- **Fix applied:** one function owns the rule — `cmd_continue::session_label(session_id, title)` —
  and both surfaces call it. The 8-character truncation is gone from this path: it existed for a
  narrow status bar, and this label also feeds a sentence the operator reads in full.
- **If we skip it:** the same class of drift the module's own header warns about ("two copies of the
  sentence are two things to keep in step") stays in place for the label.

---

## SHOULD FIX

### 3. The panic leftover goes stale after an in-TUI session switch (`tui_app.rs:301-306`)

- **What this is:** the panic hook captures `(session_id, title)` ONCE, in `TuiApp::enter`, before
  the loop. The hook closure is `move` and the id is an owned `String`, so it never refreshes.
- **Problem:** the session can change while the TUI is open (the session hub's Enter sets
  `core_session_switch`, and `apply_pending_switch` retargets). A panic after a switch prints the
  PREVIOUS session's leftover, so the operator is sent back to the session they deliberately left.
- **Fix:** re-capture on the switch. The switch already runs `apply_pending_switch`, which knows
  both the outgoing and the incoming id; the hook needs a shared `Arc<Mutex<Option<(String, String)>>>`
  written there and read by the closure. Not applied in this pass — it changes the hook's capture
  shape, and the operator's report was about timing, which is fixed.
- **If we skip it:** rare (a panic after a switch), but the failure mode is the worst kind — a
  confident wrong answer.

### 4. Four `consume_marker` callers, one of them guaranteed to lose (`cmd_sessions.rs:616`, `cmd_gateway.rs:617`)

- **What this is:** the marker is a single-slot file, and four independent places call
  `consume_marker()` (which DELETES it): the TUI boot notice, the gateway boot notice,
  `sessions resume --last`, and nothing else.
- **Problem:** first caller wins, silently. `operant gateway run` on a home where the TUI also boots
  will consume the marker the TUI was going to offer, or vice versa. Each site has a comment
  explaining why ITS consume is correct, and none of them mentions the others.
- **Fix:** the marker is an OFFER, so let every interested surface READ it and let exactly one
  surface retire it — the surface that actually acted on it. `cmd_sessions.rs` already does the
  right thing (`read_marker`) and its comment says why. The TUI and the gateway should match.
- **If we skip it:** a restart of both surfaces announces nothing, and the operator believes the
  session ended cleanly.

### 5. `resolve_target_session` lists up to 10,000 sessions to check one id (`cmd_continue.rs:228`)

- **What this is:** the fix for finding 1 needed "does this id exist", and the available API is
  `list_sessions(10_000)`.
- **Problem:** it is O(n) in the session count with a database read and a `Vec` of every row, to
  answer a yes/no about one id. On the operator's store today that is 30+ rows, so this is a
  correctness-neutral slow path, not an outage.
- **Fix:** `Database::session_exists(&self, id) -> Result<bool>` as an indexed `SELECT 1`, used by
  both the marker check and the explicit-id path (which currently does the same 10,000-row scan).
  The API addition belongs to whoever next touches `database.rs`.
- **If we skip it:** the cost grows with the store, and there are now two sites doing the same scan.

---

## NICE TO HAVE

### 6. `FORCED_EXIT_STATE` earns one use (`cmd_continue.rs:77`)

`pub const FORCED_EXIT_STATE` has exactly one real reader (`print_exit_leftover`, same file) plus
two assertions that only check it differs from the other two sentences. Inlining it deletes a public
constant and 2 lines, and the two assertions can compare the printed block instead. 4 mentions -> 1.

### 7. `CURRENT_SESSION_STATE` has one caller (`cmd_continue.rs:114`)

One caller (`tui/app/commands.rs:769`). It is a cross-module constant, so keeping it named is
defensible as documentation of the three-state contract — but it is not pulling weight the way
`ExitKind` does. Leave it; noted so the next reader does not assume it is load-bearing.

---

## What is NOT a finding (checked, found correct)

- **The boot gate is still load-bearing.** Both mutations re-run after the origin sync: removing
  `&& !boot_notice_done` fails `the_opening_notice_is_gated_on_the_boot_iteration_only`, and
  removing `boot_notice_done = true` fails the same test. The iter-2256 test is not vacuous.
- **The panic hook's ordering is right.** `LeaveAlternateScreen` precedes `print_exit_leftover` in
  the closure body, and the module's print ban is exempted with a reason on every `println!` in the
  teardown path.
- **The deliberate/interrupted split holds.** `/exit` sets `exit_is_deliberate` and calls
  `clear_marker()`; the teardown's `write_interrupted_marker` returns early on that flag and clears
  the DB flag instead, so the two halves of the record stay quiet together.
- **`ExitKind` is one type, not four strings.** `exit_leftover` is the only block; `exit_state` is
  gone from the tree.
- **`is_simulating` gate is right.** `operant tui debug simulate` must not leave real state, and the
  marker consume is behind `!is_simulating`.

---

## Verification

- `cargo test -p operant-cli --bin operant cmd_continue` — **11/11** (was 10; +1 for the label rule).
- `cargo test -p operant-cli --test continue_session` — **11/11** (was 10; +1 for the phantom marker).
- `cargo test -p operant-cli --bin operant interrupted` — **14/14**.
- `cargo check -p operant-cli --bin operant` — clean apart from the pre-existing `cronjobs/scheduler.rs`
  and `gateway_runner.rs` peer warnings.
- **Live binary run** (`operant continue`, real debug build):
  - before the fix: `Session — hello / operant continue socket-session`, and running that printed
    `Error: Session not found: socket-session`;
  - after: `Session — Capacity retry probe: … / operant continue emp-cron_fa1e44c`, and that command
    succeeds on the operator's own home.
- Full bin suite with this patch: 2542 passed / 7 failed. The same run at `origin/main` with the
  patch reverted: 2542 passed / 6 failed, and the one extra failure
  (`gateway_runner::tests::a_queued_message_waits_for_the_next_turn`) passes in isolation and is in
  a file the peer has dirty in the shared tree (`gateway_runner.rs`, `gateway_runner/tests.rs`).
  Delta attributable to this patch: **+1 test, +0 failures.**

## Verdict

Healthy on timing (iters 2255/2256 hold), unhealthy on truth: finding 1 was live on the operator's
machine and is fixed; finding 2 was two labels for one session and is fixed. Finding 3 is the one to
do next — a stale id in a panic path is a confident wrong answer.

**Lean: -6 lines possible** (finding 6; finding 7 is a keep).

## Not checked

- The gateway and cron boot paths were read, not run (they need a live platform credential).
- `apply_pending_switch`'s full retarget path was read, not executed under a panic.
- The `database.rs` API addition proposed in finding 5 was not written.