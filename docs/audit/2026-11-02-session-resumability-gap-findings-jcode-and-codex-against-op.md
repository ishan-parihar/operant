# session-resumability gap findings — jcode and codex against operant

Dated-frozen: audited `2026-11-02`; findings mark verified vs asserted.

The operator asked how `../external-repos/{jcode,codex}` implement session
resumability after the TUI closes on Ctrl+C, and how a gateway channel
(telegram, slack) navigates sessions by slash command, then asked for the
implementation and feature gaps.

Every operant claim below was measured against the live tree or the live
database at `~/.operant/database.db` in the same session — none is taken from
an existing doc's summary of itself.

## Scope

- jcode: `jcode-tui/src/tui/session_picker/*` (6,328 lines), `jcode-storage/src/active_pids.rs`
  (550), `jcode-base/src/session/crash.rs` (840),
  `jcode-app-core/src/restart_snapshot.rs` (241), `jcode-tui-session-picker/src/lib.rs`,
  `jcode-base/src/gateway/control.rs`.
- codex: `cli/src/main.rs` resume parsing, `tui/src/cli.rs`,
  `tui/src/session_resume.rs`, `app-server-transport/src/daemon_recovery.rs`,
  `app-server/src/daemon_thread_recovery.rs`,
  `app-server/src/request_processors/persisted_resume_settings.rs`,
  `tui/src/daemon_recovery.rs`.
- operant: `tui/session_hub/` (2,652), `cmd_sessions.rs` (570),
  `gateway_commands.rs` (`/resume` :1260, `/sessions` :1237, `/new` :957),
  `gateway_runner.rs` (`SessionPins` :2016, `run_session_scoped_command` :2270),
  `active_sessions.rs` (201), `session/listing.rs`, `database.rs` (:991, :1037).

## Findings

- **[verified] Operant already ships ~80% of the jcode picker.** The 2026-10-28
  plan's iter-A..E all landed: `SessionEntryV2` + `list_sessions_rich`,
  `sessions.employee_id` via column reconcile, the full-frame hub with the 40/60
  split and tokenized search, the ←← gesture with omp's `[40ms, 500ms)` window,
  and the `gateway_api.rs` route group. `genome lint --advisory` reports 0
  errors. This audit is not a re-derivation of that work.

- **[verified] There is no record that a session was interrupted.** Ctrl+C exits
  through `handle_exit_key_confirmation` (`key_handling.rs:2635-2665`) which sets
  `should_exit = true`. Nothing writes a marker anywhere. Verified by grep: no
  `restart-snapshot`, `RecoverySnapshot`, `interrupted.json`, or any equivalent
  path exists in `operant-core` or `operant-cli`. jcode's
  `save_current_snapshot()` / `arm_auto_restore_from_recent_crashes()` and
  codex's `daemon_thread_recovery::snapshot` have no operant twin.

- **[verified] The transcript survives the exit, so this is a discoverability
  gap, not a durability gap.** `turn_context.rs:178-186` calls
  `save_session(&session_id, None, "agent", &now, &now)` on every turn, and
  `save_session`'s `ON CONFLICT DO UPDATE SET ... ended_at = excluded.ended_at`
  refreshes it. Measured on the live DB: 3,226 sessions, the newest
  `sess_38d62298…` has `ended_at = 2026-11-01T21:24:34` with 43 messages. So
  `list_sessions_rich`'s `ORDER BY updated_at DESC` (database.rs:1053) does put
  the live session near the top — but nothing *identifies* it, and an operator
  who worked six sessions cannot tell which row is the one they quit.

- **[verified] `active_sessions.rs` is a concurrency cap, not a presence
  registry.** It writes mtime lock files under `sessions/active/` and prunes
  after `STALE_AFTER_SECS` (6h). No pid is written, no liveness check exists, no
  `session_presence()` function. jcode's twin writes the owning pid and gates on
  `libc::kill(pid, 0) == 0 || last_os_error() == EPERM`. Consequently operant
  cannot distinguish a cleanly closed session from a process that died — the
  exact question the operator asked.

- **[verified] Gateway `/resume` demands a `sess_<uuid>`/`gw_<hash>` string the
  chat user cannot see.** `/resume` with no args answers with usage text pointing
  at `operant sessions list` (gateway_commands.rs:1262-1267). `/sessions`
  (:1237) calls `store.list_active_sessions(Some(platform))`, which reads
  `gateway_sessions` — the binding table (session_key, session_id, platform,
  chat_type, tokens), not transcripts. Measured on the live DB: `gateway_sessions`
  has 57 rows; the `sessions` table has 22 `gw_*` rows carrying the actual
  messages. The two tables are different surfaces joined only by id, and `/sessions`
  reports only the first.

- **[verified] `RowStatus` has no interrupted/crashed state.** `session/listing.rs:36`
  defines `Current | Live | Idle | Archived`. jcode has `SessionStatus::Crashed{..}`
  derived from `detect_crash()`; codex distinguishes running/completed turn state
  per thread. Without a variant, nothing can render the distinction even where
  the data exists.

- **[verified] The gateway `/sessions` and `/resume` gates are already correct
  and reusable.** `/resume` calls `open_stored_transcript` (gateway_commands.rs:842-852)
  which counts `db.get_session_messages(session_id)` and refuses to pin on zero —
  the honesty gate. `/resume` then `SessionPins::set` (sticky, so the second
  message does not silently fall back) plus `persist_conversation` (the GW-6 row
  write, so a restart keeps it). The gateway-navigation work needs a join and a
  reply format, not new machinery.

- **[verified] `sessions resume` CLI prints a recap and attaches nothing.**
  `cmd_sessions.rs:501` resolves the id and prints `build_recap`. Neither jcode
  (`jcode resume --last`) nor codex (`codex resume --last "prompt"`, where
  main.rs:2691 re-reads the positional as a prompt) has an operant equivalent
  that reopens.

- **[asserted] No fleet agent owns this surface right now.** The 2026-10-28 plan
  is fully landed and its workflow rows are closed; the last `session_hub` work
  landed at iter-1591 (RT-206, the Ctrl+B anchor). The `claim` registry shows two
  open claims, both `agent-omarchy`'s and neither session-related.

## What is NOT a gap (measured, so it is not re-litigated)

- **External-CLI importers** (jcode's `SessionSource`/`ResumeTarget` for Claude
  Code / Codex / Pi / OpenCode / Cursor): operant has no such sources and no
  importer. A feature, not a gap.
- **A second daemon** for the snapshot: `GatewayDaemonLock`
  (gateway_runner.rs:1812-1880) already gives kernel-released single-instance.
- **Codex's resume-time cwd prompt** (`ResumeCwdMode`): operant sessions carry
  `cwd` and the TUI runs in the operator's cwd. Reopening elsewhere is a
  behaviour change nobody asked for.
- **`streaming_pids` + `StreamingGuard`**: operant answers "is a turn running"
  in-process via `is_streaming` + the delegation registry, and the gateway via
  `/status`. No cross-process consumer exists.

These four plus two more (the jcode 66K-line harness surface, and the extra
`SessionFilterMode` variants) are the plan's exclusions, each with this reason
recorded so a future agent does not rebuild them.
