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
`operant sessions list` on the server, and paste it back.

**Correction (added 2026-11-02, same session, after the reference re-read):** an
earlier revision of this section claimed jcode's channels navigate through the
same picker the TUI offers and that codex's channel surface uses the
thread-history protocol. **Both were wrong, and both are retracted here.**
Measured: jcode's `channel.rs` telegram/discord/jade-relay reply loops
(channel.rs:260, :469, :729) route every message into
`AmbientRunnerHandle::inject_message` (`ambient/runner.rs:108-136`), which either
pushes a `SoftInterruptMessage` into an active cycle or saves a directive and
triggers a wake. There is **no slash-command parsing in any channel path** —
the only text dispatch is `extract_permission_id` / `parse_permission_reply` for
approve/deny replies (channel.rs:232-234). And codex has **no channel surface at
all**: of its 132 crates there is no telegram/slack/discord/gateway crate
(verified by directory listing and by `grep -riE "telegram_token|slack_bot|
webhook_inbound"` returning nothing); `websocket-client` is transport, not
adapter.

So Gap 4 is **not a jcode/codex parity gap** — it is an operant-local usability
defect with no reference implementation to copy. That strengthens the case for
iter-2 rather than weakening it (there is nothing to vendor; it is a join and a
reply format), and it changes what "parity" means for this iteration: the
operator is not behind a reference here, operant is ahead, and the iteration
should be judged on its own UX, not on matching jcode.

Gap 4 is the second-largest gap and the second iteration. It is also the one
where the shipped `gateway_api.rs` route group (iter-E of the 2026-10-28 plan)
already did half the work.

## Gap 5 — no session status vocabulary

`RowStatus` (`session/listing.rs:25-31`) has exactly **three** variants —
`Live | Idle | Archived` — not four: an earlier revision of this plan named a
`Current` variant that does not exist in the tree (`rg "RowStatus::Current"`
returns nothing; the current session is expressed as `Live`, which the enum's own
doc calls "the session this process is actively streaming into right now").
jcode has `SessionStatus::Crashed{..}`, `Running`, `Closed`. Adding an
`Interrupted` variant is cheap; it is what makes Gaps 1-4 *visible* rather than
inferred.

## Gap 6 — `list_sessions` sorts on a timestamp that goes stale (found on validation)

Not in the original five. The validation pass measured it, and it is a real
latent defect rather than a cosmetic one, so it belongs in the plan.

`Database::list_sessions` (`database.rs:991-1003`) ends with
`ORDER BY s.ended_at DESC`. In SQLite `ORDER BY … DESC` puts NULLs **last**, so
any row whose `ended_at` is NULL sorts to the very bottom of
`operant sessions list`, `sessions browse`, `sessions prune`, `insights` and the
`session_search` tool. The schema allows NULL (`ended_at TEXT`, no NOT NULL) and
jcode's own `list_sessions_rich` guards against this with
`COALESCE(s.ended_at, s.started_at)` (database.rs:1045) — which is why the RICH
loader is correct and the legacy one is not. They are two loaders for the same
rows, and only one of them is NULL-safe.

Measured on the live DB (and this section is itself the product of a validation
correction — the first draft said "18, all `gw_*` rows", which was wrong on both
numbers): **0 of 3,226 rows have a NULL `ended_at` today** (because
`save_session`'s upsert always writes a non-NULL value), **but 19 rows are
stale** — `ended_at == started_at`, a zero-duration row. By prefix:
11 are `sess_*`/`source=agent` (all 11 carry messages) and 8 are
`gw_*`/`source=gateway` (7 of the 8 carry messages). So it is not a
gateway-only path; the `sess_*` rows come from a rewind fork's parent row or
another create-without-a-turn site, which is exactly the class of path the bug
is exposed by. Those 19 rows sort as though they closed the instant they opened,
indistinguishable from one whose `ended_at` was never refreshed.

The bug is latent rather than active only because `save_session` refreshes
`ended_at` on every turn (`turn_context.rs:181`). Any path that creates a session
row without a turn — the gateway create, a rewind fork's parent, a cron seat —
leaves it sorting wrong until something runs in it.

**The fix is one expression**, and it belongs in iteration 2 as a one-line
collateral: `ORDER BY COALESCE(s.ended_at, s.started_at) DESC`, matching what
`list_sessions_rich` already does beside it. Not its own iteration — that would
be a five-line change paying a full iteration's cost.

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

### Iteration 1 — the interrupted-session signal (core + one teardown line)

**Files:** `crates/operant-core/src/session_interrupted.rs` (new),
`crates/operant-core/src/database.rs`, `session/listing.rs`,
`crates/operant-cli/src/tui/app/mod.rs`.

> **The heading is corrected, and the reason matters.** The first revision of
> this plan called iteration 1 "core, zero TUI overlap" and named
> `adapter_types/tui_app.rs` as its TUI file. Both were wrong, and the second
> pass caught them by reading rather than trusting the first: the write point is
> in `app/mod.rs` (785 lines, a **peer-hot** file per the 2026-10-28 plan's own
> risk note), so a fresh `git status --porcelain -- crates/operant-cli/src/tui/app/mod.rs`
> is mandatory before this iteration starts. The overlap is now 1 function
> (`run_exit_teardown`) in 1 file rather than "zero", and the fleet-rule-3 check
> below is what keeps that honest.

**Validation note (verified on the second pass — this is what makes iteration 1
cheap, and the first revision of this plan had it wrong):** the write point
**already exists**. There is exactly ONE teardown on the shared exit path —
`App::run_exit_teardown` (`app/mod.rs:1454-1463`), three lines that already
persist state before the process goes away (`word_predictor.persist()`,
`debug_hub.dump_on_exit()`, `background_tasks.release_all(...)`), called once
from the single `if self.should_exit` consumer at `app/mod.rs:1644-1656`.
So the marker is a **fourth line in an existing function**, not new teardown
plumbing. Verified the id is reachable there too: `App::current_session`
(mod.rs:293) and `App::core_session_switch` (mod.rs:548) both exist on `self`,
and the frame-loop retarget at mod.rs:1632 already reads `core_session_switch`.

The seven `should_exit = true` sites were also re-checked rather than assumed:
`key_handling.rs:2650` (the interactive Ctrl+C/Ctrl+D double-press) and
`commands.rs:621` (`/exit` `/quit`) are the two real operator exits. The other
five (`event_coalesce.rs:92`, `mod.rs:1554/1564/1574/1589`) are all on the
scripted headless-simulation path (`is_simulating` / `sim_drain_*`) and end the
process without an operator having quit anything. **Consequence for iteration 1's
test:** a sim exit must NOT write the marker, because there is no operator and no
live conversation to resume. That guard is the mutation-proven behaviour, and it
is one the first revision of this plan would have gotten wrong by writing the
marker unconditionally in teardown.

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
`resolve_session(None)` still resolves the most recent when no marker exists;
and the guard the second pass added — **a headless sim exit (`is_simulating`)
writes no marker**, because there is no operator and no live conversation.

**Gates:** `cargo test -p operant-core --lib session_interrupted`;
`cargo test -p operant-cli --bin operant`; the CLI's `sessions resume` output
run against a real marker file by hand.

### Iteration 2 — gateway session browsing (`/sessions` becomes a browser)

**Files:** `crates/operant-cli/src/gateway_commands.rs`,
`crates/operant-core/src/gateway_session.rs`.

The `/sessions` handler stops reporting only bindings. It becomes:

**Platform reach (measured, and it scopes this iteration):** the operator's
question named telegram and slack. **Slack cannot reach this command at all** —
per AGENTS.md's own registry note and verified in the adapter source
(`gateway/slack.rs:142-149`, `start()` logs and returns; no `wss://` connection;
`handle_update` has no caller), Slack is outbound-only. The only platforms where
a user can actually TYPE `/sessions` are the two long-polling/ending receivers:
**Telegram and Discord**. Webhooks is inbound but is push-only (no reply path for
a listing command). So iteration 2's real surface is telegram + discord, and its
default test should be telegram. Slack gets the code for free (same handler) but
it must not be claimed as reachable, and the plan's own gates must not assert it.

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
notice; and Gap 6's collateral — a session row created with `ended_at == started_at`
(a zero-duration row) still sorts by `started_at` under
`COALESCE(ended_at, started_at)` rather than sinking to the bottom, asserted
against `list_sessions` (the legacy loader) so the regression cannot hide in the
rich loader's already-correct shadow.

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
| 1 | ~200 core + **~15 CLI** + 1 teardown line | the marker + presence port + status variant. The TUI cost is 15 lines, not 40, because the hook (`run_exit_teardown`) already exists and the only real work is the sim-exit guard. |
| 2 | ~150 + a 1-line SQL fix | one join + reply formatting, reusing two existing gates; the Gap 6 `COALESCE` is collateral. |
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

**Correction carried from the validation pass:** the first revision of this
section said iteration 1 "has no peer overlap" and named
`adapter_types/tui_app.rs`. Wrong on both counts — the write point is
`crates/operant-cli/src/tui/app/mod.rs` (`run_exit_teardown`, 1454-1463), a file
the 2026-10-28 plan's own risk list calls peer-hot. The overlap is now one
function in one file, which is small but nonzero, so the check before this
iteration is mandatory, not optional:

```bash
git status --porcelain -- crates/operant-core/src/session_interrupted.rs \
                            crates/operant-core/src/database.rs \
                            crates/operant-core/src/session/listing.rs \
                            crates/operant-cli/src/tui/app/mod.rs
```

Iteration 3 must follow iteration 1 (the badge needs `RowStatus::Interrupted` to
exist). Iteration 2 can run any time after iteration 1's marker ships, since its
join reads `RowStatus` but does not write it — and it carries Gap 6's one-line
`COALESCE` fix, which is independent of everything else and could ship alone.

## Risks

- **Fleet overlap (corrected — the first revision understated this).** iter-1 is
  *not* peer-free: its TUI write point is `crates/operant-cli/src/tui/app/mod.rs`
  (`run_exit_teardown`, 1454-1463), a file the 2026-10-28 plan's own risk list
  names as peer-hot. It does NOT touch `key_handling.rs` — that was the first
  revision's guess and it was wrong; the Ctrl+C arm only sets `should_exit`, and
  the shared consumer at mod.rs:1644 does the rest. iter-3 touches
  `session_hub/`. The check command is in "Resume here" above; run it before
  starting, and stage explicit paths only.
- **The marker must never strand a session.** If the TUI crashes before
  teardown, no marker is written and the last session stays the most recent row
  — which the operator can already reach. The marker is strictly additive
  information; absence degrades to today's behaviour.
- **`RowStatus::Interrupted` must clear on the next turn**, or every relaunch
  re-claims every old session as interrupted. The clear point is the existing
  `save_session` in the turn prologue — one line, and the test is "start a turn
  on an interrupted session, reload, status is no longer Interrupted".
