# Session resumability and gateway session navigation — a ponytail-scoped upgrade

Dated-frozen: published `2026-11-02`. Registered in `genome.toml`
`core.living_plans`.

## What was measured, and against what

The operator asked how `../external-repos/{jcode,codex}` implement session
resumability after the TUI closes on Ctrl+C, and how a gateway channel
(telegram, slack) navigates sessions by slash command, then asked for the
implementation and feature gaps and a plan to close them in a `/ponytail`
manner.

Read in full:

| surface | what was read |
|---|---|
| jcode | `crates/jcode-tui/src/tui/session_picker/{filter,render,navigation,memory}.rs`, `crates/jcode-storage/src/active_pids.rs` (550 lines), `crates/jcode-base/src/session/crash.rs` (840 lines), `crates/jcode-app-core/src/restart_snapshot.rs`, `jcode-tui-session-picker/src/lib.rs` (`SessionInfo`, `SessionFilterMode`, `ResumeTarget`, `SessionSource`) |
| jcode | `cli/main.rs` `resume` / `resume --last` / `fork` parsing (`finalize_resume_interactive`, main.rs:2677-2704); the `/resume` dispatch rename (`tui/app/commands.rs:1768`) |
| codex | `cli/main.rs` resume flags (205-441), `tui/src/cli.rs` internal resume controls, `tui/src/session_resume.rs` (cwd + target validation), `app-server-transport/src/daemon_recovery.rs`, `app-server/src/daemon_thread_recovery.rs`, `request_processors/persisted_resume_settings.rs`, `tui/src/daemon_recovery.rs` (the daemon-recovery picker view), `app-server-protocol/src/protocol/thread_history.rs` |
| codex | `app-server/src/request_processors/{daemon_snapshot,daemon_continuation}.rs` — the snapshot write and the restore path |

Operant side (all verified this session, not from memory):

- `crates/operant-cli/src/tui/session_hub/{mod,render}.rs` (744 + 633 lines) — the
  landed iter-B/C page, which already vendor-ports jcode's picker geometry
  (40/60 split, vertical constraint stack, tokenized search with per-char
  highlight, filter cycle).
- `crates/operant-cli/src/cmd_sessions.rs` (570 lines, 12 subcommands).
- `crates/operant-cli/src/gateway_commands.rs` (`/resume` :1260, `/sessions`
  :1237, `/new` :957) + `gateway_runner.rs` (`SessionPins` :2016,
  `derived_gateway_session_id` :820, `run_session_scoped_command` :2270).
- `crates/operant-core/src/active_sessions.rs` (201 lines), `session/listing.rs`,
  `database.rs` `list_sessions` :991 / `list_sessions_rich` :1037.
- The live DB at `~/.operant/database.db` (3,226 sessions; `gateway_sessions`
  holds 57 rows; 22 `gw_*` rows in `sessions`).

Supersedes nothing. It is the gap-closing sibling of
`docs/plans/2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md`
(landed in full, iter-A..E) and is built on the findings in
`docs/audit/2026-11-02-session-resumability-gap-findings-jcode-and-codex-against-op.md`.

---

## The honest headline

**Operant already has 80% of both features.** The jcode/codex mechanisms were
mostly ported or built in parallel by 2026-10-28's plan. What is missing is not
the architecture but five specific behaviours, and three of them are small.
This plan closes those five and nothing else.

The gap is *behavioural*, not structural: the picker exists, the resume pin
exists, the transcript reload exists. What does not exist is (a) anything that
tells you a session was interrupted rather than closed, (b) anything that
restores your last session when you relaunch, and (c) gateway navigation that
can *browse* rather than only name an opaque id.

---

## Gap 1 — "which session did I quit out of?" — nothing answers it

**jcode** writes a `restart-snapshot.json` (`restart_snapshot.rs:39-45`) holding
the open sessions with `auto_restore_on_next_start`, arms it from recent crashes
(`arm_auto_restore_from_recent_crashes` :76, 24h cutoff), and consumes it at
startup. `SessionStatus::Crashed{..}` is derived from
`detect_crash()` (crash.rs:283) against the active-pid registry.

**codex** writes a `RecoverySnapshot` of root thread ids from the app-server
(`daemon_thread_recovery.rs:11-28`, off the tokio blocking pool so a forced exit
never waits on I/O), deletes it on read so it serves exactly one generation,
and restores in the background (`start_recovery` :34) without delaying
readiness. `tui/src/daemon_recovery.rs` renders the incompatibility picker when
restore is not safe.

**operant**: nothing. Ctrl+C exits through `handle_exit_key_confirmation`
(`key_handling.rs:2635`) → `should_exit = true` → teardown. The session id is
persisted per-turn (`turn_context.rs:178-186` `save_session` refreshes
`ended_at`), so the *transcript* survives — but there is no record that the
session was live at exit, and nothing reads one at startup. On relaunch the
user must open `/resume` and hunt. In the gateway the equivalent is
`/new`+`/resume` by opaque id.

This is the largest gap and the whole of iteration 1.

## Gap 2 — the Ctrl+C resume-after-close round trip is not discoverable

`/resume` opens the hub (commands.rs:525) and Enter retargets
(`pending_switch_session`). That works. But:

- The hub's `open()` always resets `selected = 0` (session_hub/mod.rs:232) and
  `rebuild_visible()` — so it lists *all* sessions by `updated_at DESC` (the
  `list_sessions_rich` order, database.rs:1053). The session just quit from IS
  usually row 0, but nothing marks it, and a session the operator quit 10 turns
  ago is indistinguishable from one that closed cleanly.
- Nothing anywhere answers "what was I last doing".
- CLI parity: `operant sessions resume <id>` prints a recap (cmd_sessions.rs:501)
  but attaches nothing. Codex has `codex resume --last "prompt"` (main.rs:3691:
  with `--last`, the positional is re-read as a prompt) — one token reopens the
  last conversation and runs a prompt in it. jcode has `jcode resume --last`.

## Gap 3 — no crash/liveness distinction

`active_sessions.rs` is a *concurrency cap*, not a presence registry: lock files
under `sessions/active/` pruned after 6h, no pid written. jcode's
`active_pids` writes the owning pid, checks `kill(pid, 0)` liveness, and has a
`StreamingGuard` RAII that clears on every exit path including panic. Without
this, operant cannot distinguish "closed at Ctrl+C" from "crashed mid-turn" —
which is precisely what the user is asking about.

Gap 3 is the dependency for Gap 1's "interrupted" labelling, and it is ~90
lines of near-direct port. Both are one iteration.

## Gap 4 — gateway session navigation requires an opaque id

Gateway `/resume <session-id>` (gateway_commands.rs:1260) demands a
`sess_<uuid>` or `gw_<hash>` string. `/sessions` (:1237) reports
`list_active_sessions(platform)` from `gateway_sessions` — the *binding* table,
which maps platform+user+channel to a session id. It does not list transcripts.

So a telegram user must copy an id out of a message they cannot see, run
`operant sessions list` on the server, and paste it back. jcode's gateway has
`/remote` (control.rs:36-76) as the *auxiliary* face, but its session
navigation is the same picker the TUI offers (the `SessionPicker` with
`SessionFilterMode` including `Active` — "sessions with a live process right
now, annotated with whether each is still streaming"). Codex's channel surface
uses the thread-history protocol so a remote client lists threads as *rows*.

Gap 4 is the second-largest gap and the second iteration. It is also the one
where the shipped `gateway_api.rs` route group (iter-E of the 2026-10-28 plan)
already did half the work.

## Gap 5 — no session status vocabulary

`RowStatus` has `Current | Live | Idle | Archived` (session/listing.rs:36).
Neither codes "was interrupted by a Ctrl+C exit" or "crashed". jcode has
`SessionStatus::Crashed{..}`, `Running`, `Closed`. Adding a variant is cheap;
it is what makes Gaps 1-4 *visible* rather than inferred.

---

## What this plan deliberately does NOT do (ponytail exclusions)

Each of these is a real jcode/codex feature that was measured and rejected with
a reason:

1. **No external-CLI importers.** jcode's picker lists Claude Code / Codex / Pi
   / OpenCode / Cursor transcripts via `SessionSource` and `ResumeTarget`.
   Operant has no such sources and no importer; the picker's vendor comment
   (session_hub/mod.rs:12-14) already records the cut. Building them is a
   separate feature.
2. **No second daemon.** Codex's recovery snapshot is written by an app-server
   daemon. Operant's `GatewayDaemonLock` (gateway_runner.rs:1812-1880) already
   provides single-instance; the snapshot rides the gateway process when it is
   up, and the TUI process when it is not. No new listener. Confirms and closes
   the "server phase" question the 2026-10-28 plan already answered.
3. **No codex-style cwd prompt on resume** (`session_resume.rs`,
   `ResumeCwdMode`). Codex asks which directory a resumed thread should run in.
   Operant sessions carry `cwd` and the TUI already runs in the operator's cwd.
   Reopening a session in a *different* directory is a real behaviour change
   the operator did not ask for.
4. **No jcode `SessionFilterMode::CatchUp`/`Saved`/`Active` full cycle.** The
   hub's cycle is `All | CurrentDir | ByEmployee | Active | CurrentBranch |
   Archived`. Adding two modes means adding two semantics; `Active` already
   covers the live-process case once Gap 3 lands.
5. **No streaming marker.** jcode's separate `streaming_pids` dir with
   `StreamingGuard`. Operant's `is_streaming` + delegation registry already
   answers "is a turn running" in-process, and the hub's G3 join uses it. A
   cross-process streaming flag has no consumer inside one process, and the
   gateway's own `is_streaming` is already surfaced by `/status`.
6. **No jcode 66K-line harness surface** — re-affirmed from the 2026-10-28 plan
   (its own "Explicitly out").

---

## The plan

Five iterations. Ordered so each is independently shippable and testable, and
so that no two touch the same file (the fleet's rule 3).

### Iteration 1 — the interrupted-session signal (core, zero TUI overlap)

**Files:** `crates/operant-core/src/session_interrupted.rs` (new),
`crates/operant-core/src/database.rs`, `session/listing.rs`,
`crates/operant-cli/src/tui/app/adapter_types/tui_app.rs`.

1. `SessionInterrupted` marker. On Ctrl+C/Ctrl+D exit confirmation
   (`handle_exit_key_confirmation` sets `should_exit`), the teardown writes a
   single json under `operant_data_dir()/sessions/interrupted.json` holding the
   app's own live session id, timestamp, cwd. On clean `/quit` it is deleted.
   Shape mirrors codex's `RecoverySnapshot` (one generation, consumed on read)
   and jcode's `RestartSnapshot` (stable file, `auto_restore_on_next_start`).
   *This is the codex shape, chosen because "the last thing I was doing" is a
   single fact, not a list — jcode's snapshot exists to restore multi-session
   servers, which operant does not have.*
2. PID-liveness companion. Extend `active_sessions.rs`'s registry with
   `register_pid(session_id, pid)` / `session_presence()` — a direct port of
   `active_pids.rs`'s core: write the pid, `kill(pid, 0)` liveness, prune dead.
   No `StreamingGuard`, no `internal_pids` (exclusion 5).
   Scope: only the gateway's per-chat sessions and the TUI's own session.
3. `RowStatus::Interrupted` + `RowKind` untouched. Written when the marker is
   consumed and matched to a session, cleared when that session's next turn
   starts (the existing `save_session` at turn_context.rs:178 is the natural
   clear point — one line).
4. Startup consumption. The TUI's launch path reads the marker, marks the row,
   and — the actual UX — surfaces a status notice on first frame: "Resume the
   session you quit out of? /resume (Esc to dismiss)". Not an auto-attach.
   *Ponytail call: codex arms `auto_restore_on_next_start`, so does jcode. But
   auto-attaching a session on the TUI after a Ctrl+C that the operator may
   have pressed to get away from a stuck turn is the wrong default. One notice,
   zero risk.*
5. CLI: `operant sessions resume` with no `<id>` (already defaults to most
   recent via `resolve_session`) plus `operant sessions resume --last` printing
   the marker's session and the interrupted notice.

**Tests (each fails without the change):** marker write on exit path and
absence on clean quit; `session_presence()` skips a dead pid and counts a live
one; `RowStatus::Interrupted` set on consume and cleared on the next turn;
`resolve_session(None)` still resolves the most recent when no marker exists.

**Gates:** `cargo test -p operant-core --lib session_interrupted`;
`cargo test -p operant-cli --bin operant`; the CLI's `sessions resume` output
run against a real marker file by hand.

### Iteration 2 — gateway session browsing (`/sessions` becomes a browser)

**Files:** `crates/operant-cli/src/gateway_commands.rs`,
`crates/operant-core/src/gateway_session.rs`.

The `/sessions` handler stops reporting only bindings. It becomes:

- No args: list this platform's sessions **with transcripts** — join
  `gateway_sessions` (binding row: platform, chat name, last activity) against
  the main `sessions` table (title, message count, `RowStatus`). Rows render
  with a 1-based index, a title, a relative recency, and a message count:
  enough for a telegram user to *choose*. It is the shipping `list_sessions_rich`
  + the store's `list_active_sessions`, one SQL join, no new loader.
- `/sessions <n>` or `/sessions <session-id>`: pins exactly as `/resume` does
  today, so the existing honesty gate (`open_stored_transcript`, :1272) and
  `SessionPins` sticky semantics are reused verbatim rather than reimplemented.
- `/new` and `/resume` (no args) stay.

The reply is capped at a telegram-safe length (cite `truncate_for_reply`'s
existing pattern at gateway_runner.rs:407) — the last 15 sessions with a
"• older sessions omitted" tail, which is what makes the listing honest in a
chat surface.

**Tests:** the join returns the telegram session the operator owns, not another
platform's; `/sessions 2` pins row 2's id; `/sessions bogus` refuses without
pinning (the honesty gate); a long listing truncates at the cap with the tail
notice.

### Iteration 3 — the interrupted badge in the hub, and the resume affordance

**Files:** `crates/operant-cli/src/tui/session_hub/{render,mod}.rs`.

`RowStatus::Interrupted` renders as a distinct badge (the hub already renders
row status; this is one arm) and the hub sorts interrupted rows above idle rows
within the same recency — jcode's picker has a `saved` pin and an `Active`
filter for exactly this reason: the row you want is usually not the newest row.

Also: the hub's `open(Some(interrupted_session))` selects that row when the TUI
launched with a marker not yet consumed, so the operator's first `/resume` or
←← lands with the cursor already on the right row. One parameter on the
existing `open()`.

**Tests:** the badge renders for `Interrupted` and not for `Idle`; `open(Some(id))`
selects that row; interrupted rows outrank idle rows at equal recency.

### Iteration 4 — CLI/platform parity for the new surface

**Files:** `crates/operant-cli/src/cmd_sessions.rs`, `operant.example.toml`,
`docs/CHANGELOG.md`, `README.md`.

- `operant sessions list` gains an `--interrupted` filter (the same marker
  query as iter-1's consume, read-only this time).
- `operant sessions resume --last` (iter-1) documented in `--help` and README.
- `/sessions` documented in the gateway command help (`ui_overlays`-equivalent
  for telegram) and `operant.example.toml`'s gateway section keeps its honesty
  line: it browses transcripts, `/resume` pins.
- Config: none. The marker path derives from `operant_data_dir()`, and the
  liveness registry rides `ActiveSessionTracker`'s existing dir. **No new
  config key** — this is a ponytail plan and a `[sessions] interrupted = true`
  block would be a switch nobody flips.

**Gates:** `command_sweep.py` still 98/98 (no TUI command names change — the
gateway `/sessions` handler is extended, not renamed); clippy gate; fmt.

### Iteration 5 — delete nothing, document everything

**Files:** `docs/INDEX.md`, `docs/library/sessions.md`,
`docs/library/gateway.md`, `docs/RECORDS.md`, `docs/handoff/STATE.md`.

The genome's one-owner rule: `sessions.md` owns the interrupted marker and the
presence registry's semantics; `gateway.md` owns the `/sessions` browse and
`/resume` pin. `docs/INDEX.md` gains one row for this plan. No E303 work is
carried here — `genome lint --advisory` reports **0 errors** at publish time,
and `sessions.md`'s two mentions of `session_browser` / `session_branching`
(:18, :37) are historical prose about retired overlays, not live citations, so
they stay. The only live warnings are the two pre-existing duplicate-plan (W006)
and three no-table-of-contents (W010) advisories, none of them this plan's.

State: `genome handoff --write` at the end, after the push.

---

## Cost line

| iteration | new lines | notes |
|---|---|---|
| 1 | ~200 core + ~40 CLI | the marker + presence port + status variant |
| 2 | ~150 | one join + reply formatting, reusing two existing gates |
| 3 | ~80 | one render arm + an `open()` parameter |
| 4 | ~30 | a flag, help text, docs |
| 5 | docs only | |

Five iterations, no new dependency, no new config surface, no new daemon, no
new crate, no new tool.

## Verification that is not negotiable

Per AGENTS.md, every code iteration: `cargo fmt --all`, scoped check, scoped
tests, `bash scripts/clippy-warning-gate.sh`, then a `scripts/build.sh
--release` + the deploy leg with the provenance gate (`ls
--time-style=full-iso target/release/operant` must postdate the commit) and the
md5-of-the-running-process check. Iteration 1's TUI touch is teardown-only, so
the deploy verify uses `operant tui debug simulate` with the marker present:
that is the only cheap way to prove the resume notice actually paints.

## Resume here

All five iterations open. Start with iteration 1 — it is the one the operator's
question is actually about ("how do they resume after Ctrl+C"), and iterations
2-5 are each independently skippable if the operator wants only the
resume-after-close behaviour.

Iteration 1 has no peer overlap (`session_interrupted.rs` is a new file, and the
TUI touch is teardown-only in `tui_app.rs`), so it is the safe leg to start on
in a dirty tree. Re-run `git status --porcelain -- crates/operant-core/src/session_interrupted.rs
crates/operant-cli/src/tui/app/adapter_types/tui_app.rs` before it, per fleet rule 3.

Iteration 3 must follow iteration 1 (the badge needs `RowStatus::Interrupted` to
exist). Iteration 2 can run any time after iteration 1's marker ships, since its
join reads `RowStatus` but does not write it.

## Risks

- **Fleet overlap.** iter-1 touches `key_handling`-adjacent code only through the
  teardown path in `tui_app.rs`; iter-3 touches `session_hub/`. Re-grep
  `git status` before each, and stage explicit paths only.
- **The marker must never strand a session.** If the TUI crashes before
  teardown, no marker is written and the last session stays the most recent row
  — which the operator can already reach. The marker is strictly additive
  information; absence degrades to today's behaviour.
- **`RowStatus::Interrupted` must clear on the next turn**, or every relaunch
  re-claims every old session as interrupted. The clear point is the existing
  `save_session` in the turn prologue — one line, and the test is "start a turn
  on an interrupted session, reload, status is no longer Interrupted".
