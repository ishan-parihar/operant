# Operant task ledger

Autonomous mode (`operant autonomous`, `operant run --autonomous`) reads this
file (`[autonomous] todo_path`): it works the `## Pending` list, runs
`[autonomous] test_command`, commits to `[autonomous] git_branch`, and records
progress in `autonomous-status.toml`. Only the `## Implemented` and
`## Pending` sections are parsed. Anything that needs an owner decision belongs
in `## Review`, which autonomous mode never touches.

## Implemented

- TUI rows render once while tools run (iter-797)
- Synchronous, viewport-independent drag copy (iter-797, iter-801)
- Reasoning display: jcode Current semantics by default (iter-797)
- Streams outlive `timeout_secs`; stream drops always earn retries (iter-799)
- Large pastes reach the model, not their placeholder (iter-800)
- Copy is clean document text: logical lines, no gutters, no tool rows (iter-801)
- Copy carries no chrome: system cards rejoin; code, quote, table as text (iter-802)
- Tool-loop guardrails warn the model and stop the turn cleanly (iter-805)
- Selection highlights only the text a copy carries (iter-806)
- oh-my-pi shortcuts, conflict-checked; `[tui] external_editor` (iter-807)
- Essential commands wired (`/compact`, `/cancel`, `/transcript`, `/changelog`, `/refresh-model-list`, `/auth`, `/goals`); jcode-only commands removed (iter-808)
- 35 more dead commands purged; wiring pinned by `no_registered_tui_command_is_unwired` (iter-817)
- Help/keys overlays generated from the keybinding registry; `/permissions` picker; Shift+Tab no longer cycles permission mode (iter-820)
- Visibility toggles: Ctrl+T thinking, Ctrl+Shift+O/Alt+O tools; copy mode on Alt+Y (iter-821)
- Reasoning effort end-to-end: config → request shape (per-provider) → rejection fallback → Shift+Tab/picker (iter-829)
- Cancel preserves context: typed `Error::Interrupted`, tail committed with synthetic tool results, interruptible tools, 3s watchdog (iter-832)
- Fullscreen `/rewind` selector over the real transcript; double-Esc opens it; forks are parent-linked sessions (iter-838)
- Web search and browser are sourcehound-only; memory is memory-wire-only (MEMORY.md/USER.md imported once) — alternates accepted-with-warning then ignored (iter-842/843/846)
- Subprocess hygiene: child stderr drains (no 64KB-pipe freezes), warmups reaped (no zombies), tool-output retention budgeted at 4MB (iter-848/850)
- `operant debug memory` + `docs/MEMORY_BUDGET.md` footprint surfaces (iter-851)
- Memory banks: `bank_for(identity, scope)` — seat banks (`emp:*`) isolated from the personal bank (iter-856)
- Terminal-scroll watermark keyed on message boundaries, not rows (BUGS K-3, iter-887)

## Pending

- Rebuild `/settings` over `operant.toml` (toml_edit, comments kept), tabs per config section, live-apply where supported; retire the legacy settings.json screen
- `[keybindings]` user overrides in `operant.toml` (`/keys` now shows the effective registry bindings + conflicts — iter-820; overrides remain)
- Unify the gateway's `agent.show_reasoning` with the TUI reasoning display mode
- Holon red-team (BUGS.md OS-5): `chair notify` exits 0 when every send failed (`crates/operant-cli/src/cmd_org.rs:367-368`). Verify: exit nonzero when `sent == 0 && failed > 0`, with a test over a failing sender.
- Holon red-team (BUGS.md OS-9): `supervise --loop` never clears `errored` (`cmd_org.rs:502, 529, 547`). Verify: the idle back-off returns to 30 s after one successful tick, with a test.
- Holon red-team (BUGS.md OS-8): native tool children outlive their 120 s timeout (`crates/operant-os/src/tools.rs:257-284`). Verify: `kill_on_drop` on the child, with a test that a timed-out child is reaped.
- Holon red-team (BUGS.md OS-10): timeout kill leaves TERM-trapping grandchildren alive (`supervisor.rs:273-285`). Verify: after the grace period the whole group gets SIGKILL, with a test using `trap '' TERM`.
- Holon red-team (BUGS.md OS-6): Ctrl-C or SIGTERM on `supervise` leaves the body running (`supervisor.rs:84, 110`). Verify: the supervisor forwards SIGTERM and SIGINT to the body group and kills it on exit, with a test.
- Holon red-team (BUGS.md OS-7): a reclaimed or cancelled run exits 0 and skips `finish` (`supervisor.rs:146-169, 192-193`). Verify: a non-`running` status is a non-zero exit with no self-submit, with a test for each status.
- Holon red-team (BUGS.md OS-4): `holon_skill_invoke` is allowed in observe posture (`crates/operant-os/src/policy.rs:100-107`). Verify: it is in `OBSERVE_HOLON_DENY`, with a policy test.
- Seat red-team (BUGS.md GW-2): cron turns never set the memory scope (`crates/operant-core/src/cronjobs/scheduler.rs:478-481`). Verify: `bind_seat_run` sets the scope from the seat, with a test that a cron recall uses the seat's bank.
- Seat red-team (BUGS.md GW-3): an unbound turn after `/new` keeps the previous employee's seat (`crates/operant-cli/src/gateway_runner.rs:1254-1276`). Verify: the unbound branch clears the seat, with a test that the next turn's consult uses no seat.
- Seat red-team (BUGS.md GW-4): a queued memory write resolves its bank when the worker runs (`crates/operant-core/src/memory_provider.rs:94-101`, `memory_wire.rs:190-204`). Verify: the job carries its scope, with a test that a scope change after queueing does not move the write.
- Seat red-team (BUGS.md GW-5): `/new` does not clear the transcript or the summary (`crates/operant-cli/src/gateway_commands.rs`, `gateway_runner.rs:464-469`, `gateway_session.rs:1401-1404`). Verify: the next turn after `/new` starts from an empty transcript, with a test.
- Seat red-team (BUGS.md GW-7): the continuation summary is keyed by chat, so `/resume` can inject another transcript's summary (`gateway_runner.rs:1313, 1601, 1632`). Verify: the summary is keyed by `session_id`, with a test for a pinned resume.
- Seat red-team (BUGS.md GW-9, same defect as G-1 in `docs/plans/2026-10-19-org-and-tui-foundation-ratified-decisions-red-team-findings-.md`): `/session` reports `premiere` for an unbound chat (`crates/operant-cli/src/gateway_commands.rs:835`). Verify: the report says "unbound — personal seat" for a NULL binding, with a test. Do not change the default binding; that is owner decision org-tui-foundation-003.
- TUI red-team (BUGS.md TUI-6): a shell-out spawn error skips the terminal restore (`crates/operant-cli/src/tui/adapter_types/tui_app.rs:1267`, `?` before `:1271-1272`). Verify: the restore runs on every exit path, including a spawn error, with a test that forces the spawn to fail.
- TUI red-team (BUGS.md TUI-7): the shell-out ignores the child's exit status (`tui_app.rs:341-343, 367-370`). Verify: a non-zero exit or a signal shows its status in the status line, with a test.
- TUI red-team (BUGS.md TUI-2): `/resume` shows a transcript but does not retarget the agent (`crates/operant-cli/src/tui/app/channel_drains.rs:236-270`). Verify: a load also sets the agent's session id and the header title, with a test that the next turn uses the loaded id.
- TUI red-team (BUGS.md TUI-3): creating an agent overwrites an existing definition with the same slug (`agents_view.rs:317-327`). Verify: a new agent whose target file exists is refused with an error that names the file, with a test; editing an existing agent is unchanged.
- TUI red-team (BUGS.md TUI-4): a multi-line agent description is truncated on save (`agents_view.rs:492, 420-425`). Verify: the description round-trips through save and reload unchanged, with a test.
- TUI red-team (BUGS.md TUI-5): the `/rewind` y/n confirmation stays armed after other keys (`key_handling.rs:1166, 1187`). Verify: any key other than y or n cancels it, with a test.
- TUI red-team (BUGS.md TUI-8): `/rename` ignores its argument and is not saved (`commands.rs:1003-1008`). Verify: `/rename <title>` writes the title to the database, with a test.
- TUI red-team (BUGS.md TUI-9): delegations stay "running" after `/new` or `/rewind` (`agent_events.rs:159-200`). Verify: the status entries clear on both commands, with a test.
- TUI red-team (BUGS.md TUI-10): the shell-out leaves mouse capture on for the child (`tui_app.rs:1240-1280`). Verify: capture is off for the child and back on after it, with a test of the toggle order.
- TUI red-team (BUGS.md TUI-12, TUI-13): command descriptions do not match their handlers (`crates/operant-cli/src/tui/operant_app/app.rs`, `app/commands.rs`). Verify: each description names what its handler does; change the description, not the handler, with a test that the registry text matches.

## Review

Owner decisions — not autonomous work:

- Context management (magic-context review): agent-driven reduction (`ctx_reduce`) and a background historian instead of the synchronous summarize rung — larger core changes
- BUGS.md R42-8: two live Telegram bot tokens unredacted in gateway.log
- Holon red-team (BUGS.md OS-1, OS-2): a same-uid body can read the supervisor's environment (`/proc/$PPID/environ`) and the operator's chair keys (`~/.config/holon/keys`). Needs an isolation decision: separate uid, or a restricted body environment with no chair keys. Overlaps O-11.
- Holon red-team (BUGS.md OS-3): enforcement is a silent no-op when `policy.json` is missing. Decide whether org mode fails closed.
- Holon red-team (BUGS.md OS-11): the Holon driver interpolates `employee_slug` and `scope` unquoted into `sh -c` (`holon/execution/drivers.py:142-143`). This is a Holon-repo change, not an operant change.
- Seat red-team (BUGS.md GW-1): the budget is charged to the employee bound at write time and never rolls. Decide the attribution rule (charge the turn's start-time seat, and add a window reset) before any code change.
- Seat red-team (BUGS.md GW-6): a rebind keeps the chat's transcript, so the new employee sees the old employee's conversation. Decide whether a rebind starts a fresh transcript.
- Seat red-team (BUGS.md GW-8): the budget gate fails open when the usage read errors. Decide fail-closed for org mode; a locked database would then block turns.
- Seat red-team (org-tui-foundation-003): the default employee for new org-mode chats. Options: a `[holon] default_employee` roster id, or leave new chats unbound. `premiere` is not on the roster, so AD-004 rules it out.
- TUI red-team (BUGS.md TUI-1): a pending approval prompt is painted under later overlays, and its keys go to whichever overlay is open. Overlaps `docs/ROADMAP-TUI-FLEET.md` §6.1, which deferred the dispatch-order fix. Decide: reorder the gates, or make `dialog_priority()` the real router.
- TUI red-team (BUGS.md TUI-11): some TUI actions run blocking I/O on the event loop (SQLite branch queries, `git diff`). Magnitude is unmeasured. Decide whether to move them to `spawn_blocking` now or after profiling.
