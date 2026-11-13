# Task Ledger — TUI exit leftover + `operant continue`

End-state test: `operant continue <id>` re-enters the TUI attached to that session with its
transcript rehydrated, AND closing the TUI prints that exact command with the session id.

## Ledger

1. [x] U1 Intent table + ledger — done when: `_kernel/intent.md` and this file exist.
   — depends on: - — premises: C1,C2,C3
2. [x] U2 `cmd_continue.rs` — done when: `cargo test -p operant-cli cmd_continue` passes and
   the module exposes `continue_command_line`, `format_exit_leftover`,
   `resolve_target_session`, `handle_continue_command`.
   — depends on: U1 — premises: C1,C2
3. [x] U3 CLI wiring — done when: `operant continue --help` lists the command and
   `operant continue <bogus>` exits non-zero with "Session not found".
   — depends on: U2 — premises: C2
4. [x] U4 `LaunchMode::Resume` — done when: the TUI boots attached to the given session id
   (agent's session id == the requested id before the first turn).
   — depends on: U2 — premises: C2
5. [x] U5 Exit leftover — done when: closing the TUI prints
   "To continue this session, run:\n  operant continue <id>" containing the live id.
   — depends on: U2 — premises: C1
6. [x] U6 Presence claim fix — done when: after one turn, `session_is_live(id)` is true for
   this process, and false after the process exits.
   — depends on: U4 — premises: C2 (safety: `continue` must not fork a live session)
7. [x] U7 Build + tests — done when: `cargo build -p operant-cli` and the CLI test suite are
   green (no new warnings).
   — depends on: U3,U4,U5,U6 — premises: C1,C2
8. [x] U8 Gap report — done when: `operant-testing/kb/` carries the session-management gap
   register with file:line evidence.
   — depends on: U7 — premises: C3

## Parallel groups
- U3, U4, U5 share U2 and touch different files (main.rs / tui_app.rs / tui_app.rs) — the
  tui_app.rs edits are serialised by hand; U3 is independent.
- U6 depends on U4's plumbing (the id must be observable) but its edit is in
  `tui/app/mod.rs` + `init.rs`.

## Unknowns front-loaded
- Does `agent.set_session_id(<existing id>)` rehydrate from SQLite for a session written by
  a PREVIOUS process? Verified: `SessionStore::acquire` → `load_from_db`
  (operant-core/src/session/mod.rs:279,460). ✔
- Does the TUI's frame loop drain `pending_switch_session` at boot? Verified:
  tui/app/mod.rs:1756 drains it when `!is_streaming`, which is true at boot. ✔
- Is the presence registry ever populated? Verified NO: the only two `claim_session_here`
  call sites read the agent's session id at boot, when it is still None (the id is minted
  on the first turn). → U6.


### U9 — the deliberate-close defect (found while implementing U5)

The user's scenario is "after CLOSING the TUI". Closing it on purpose (`/exit`) turned out to
be indistinguishable from being cut off: the `/exit` arm cleared the interrupted marker and the
exit teardown wrote it straight back, so the next launch announced an interruption for a session
the operator had closed on purpose, and `operant sessions list --interrupted` listed it.

Fix: `App::exit_is_deliberate`, set by the `/exit` arm, makes the teardown skip the marker write
and clear the stale DB flag instead. The distinction the marker module exists for now survives the
most common exit path.

- [x] U9 Deliberate-close fix — done when: `cargo test -p operant-cli --bin operant deliberate_exit`
  passes, and the row's `RowStatus` is `Idle` after a deliberate close (it was `Interrupted`).

## Verification record (this session)

| check | command | result |
|---|---|---|
| type check | `cargo check -p operant-cli` | exit 0 |
| lint | `cargo clippy -p operant-cli --bin operant --no-deps` | exit 0, no warnings |
| unit | `cargo test -p operant-cli --bin operant cmd_continue` | 6 passed |
| integration | `cargo test -p operant-cli --test continue_session` | 7 passed |
| deliberate close | `cargo test -p operant-cli --bin operant deliberate_exit` | 1 passed |
| full bin suite | `cargo test -p operant-cli --bin operant` | 2307 passed, 1 failed — `tui::keybindings::tests::every_remaining_vim_context_chord_either_works_or_says_why_not`, a PRE-EXISTING failure in another agent's uncommitted keybinding WIP (`keybindings/defaults.rs`, `keybindings/tests.rs`, `app/key_handling.rs` are all `MM`); untouched by this change |
| end-to-end (manual) | the built binary against a temp `HERMES_HOME` | explicit id → that session; marker → the marked session; no marker → most recent; unknown id → exit 1 `Session not found`; live session → exit 1 `still running in another process`; marker survives two runs |

### NOT verified here
- The TUI's own exit path (the leftover print and the `pending_switch_session`
  attach) needs a real terminal: the headless simulator (`operant tui debug
  simulate`) drives `run_headless`, which has no terminal and skips the
  presence sync by design. The leftover's TEXT is covered by unit tests and by
  the integration test (same formatter), and the attach reuses the hub's
  existing retarget path; the wiring is compile-checked but not screen-checked.
- The presence CLAIM (the write side of `session_is_live`) is only reachable
  from an interactive TUI. The READ side is proven end-to-end: a live record
  makes `operant continue` refuse to fork the session.

## Ledger — the second request (runtime + restart surfaces)

10. [x] U10 `format_continue_hint(state, id, title)` — one formatter for all three surfaces,
    with the id as the label fallback — depends on: U2 — premises: C4,C5
11. [x] U11 `App::announce_interrupted_session` — the boot notice becomes the status line AND
    a transcript annotation carrying the whole id + the command; the hub cursor still arms
    — depends on: U10 — premises: C4
12. [x] U12 `/continue` — the runtime slash command that prints the same command for the LIVE
    session, registered in the catalogue (`cli_only`) — depends on: U10 — premises: C5
13. [ ] U13 Verify + ship — check, clippy, the new tests, the full suite, then the repo's
    push/deploy protocol — depends on: U11,U12 — premises: C4,C5,C6

### Files touched for U10–U12
- `crates/operant-cli/src/cmd_continue.rs` — `format_continue_hint`, `format_exit_leftover` now a wrapper
- `crates/operant-cli/src/tui/app/mod.rs` — `announce_interrupted_session` (+ the re-applied U4–U6 hunks)
- `crates/operant-cli/src/tui/adapter_types/tui_app.rs` — the boot notice delegates to the App
- `crates/operant-cli/src/tui/app/commands.rs` — the `/continue` arm (+ the re-applied `/exit` flag)
- `crates/operant-cli/src/commands.rs` — the catalogue entry
- `crates/operant-cli/src/tui/app/tests.rs` — 4 tests (runtime, pre-first-turn, boot notice, deliberate exit)

### Design change after iter-2150 landed (recorded, because it is the kind of change that
### silently doubles a mechanism)

The first cut of the presence fix POLLED the agent's session id every frame and claimed on
change. iter-2150 (a peer) landed a claim at every TURN START — the id exists there — which
covers the same cases from an event instead of a poll. The poll was dropped: two claimers are
two places to keep in step, and a per-frame read is not free. What survives is the part the
turn-start claim cannot cover: the moment the agent RETARGETS (the hub switch and
`operant continue`'s attach), where the id changes without a turn. There the new session is
claimed (through iter-2150's own `claim_current_session`) and the outgoing one released — the
release needs an ownership check, because the presence record is per session and an
unconditional delete would erase ANOTHER process's claim.

### WHAT WAS LOST AND RE-APPLIED (recorded because it will happen again)
A peer agent's commits (iter-2152…2157) reverted the tree's tracked files, which silently dropped
every hunk of U2–U6 from main.rs / tui_app.rs / app/{mod,init,commands,tests}.rs /
operant-core (the two NEW files survived as untracked). All of it was re-applied here against the
peers' updated code — including the one hunk that had to be re-derived: the presence-claim site
now reads `match claim_session_here(...)` because iter-2157 taught the registry to REFUSE a claim
owned by a live process (RT-522). Lesson: in this shared tree, "the working tree is the ledger" is
false — the ledger is the files this session owns, and the tracked-file edits must be re-verified
after every peer push.
