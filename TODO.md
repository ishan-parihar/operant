# Operant task ledger

Autonomous mode (`operant autonomous`, `operant run --autonomous`) reads this
file (`[autonomous] todo_path`): it works the `## Pending` list, runs
`[autonomous] test_command`, commits to `[autonomous] git_branch`, and records
progress in `autonomous-status.toml`. Only the `## Implemented` and
`## Pending` sections are parsed. Anything that needs an owner decision belongs
in `## Review`, which autonomous mode never touches.

Working a `## Pending` row, one row per iteration:

- Write the test the row names first and watch it fail. Then make it pass, and run `cargo fmt --all`, `cargo check --workspace`, `cargo test --workspace` and `bash scripts/clippy-warning-gate.sh`.
- Mark the row's BUGS.md entry `[x]` with the iteration id. Move the row to `## Implemented` with the same id.
- If a gate fails twice on the row, the fix changes a Holon contract (kernel fields, notify semantics, the driver), or it needs an owner decision, move the row to `## Review` with the output or the reason. Do not guess.

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
- Async memory writes carry their bank from enqueue; a scope change after queueing does not move the write (BUGS GW-4, KB-7, iter-962)
- Skill writes are screened and rolled back when refused; skill names and descriptions are escaped in the system prefix; a Holon run's learning delete or edit is an annotation (BUGS KB-4, KB-5, KB-9, iter-962)
- Observe posture denies `holon_skill_invoke`, which writes the org ledger (BUGS OS-4, iter-963)
- Every turn starts unseated: an unbound chat no longer keeps the previous chat's employee seat (BUGS GW-3, iter-965; the test covers the seat accessor, not the consult in `stream.rs`)
- `/resume` retargets the agent: the picked session becomes the agent's session and the transcript is rebuilt from it, through the branch-switch path (BUGS TUI-2, iter-966)
- A new agent never replaces an existing definition, dangling symlinks included; the error names the file (BUGS TUI-3, iter-964)
- Agent descriptions round-trip through save and reload: a multi-line description is written as one JSON-quoted YAML scalar (BUGS TUI-4, iter-964)
- Any key other than y or n cancels the `/rewind <n>` confirmation (BUGS TUI-5, iter-964)
- `chair notify` exits nonzero when no chair was reached (BUGS OS-5, the exit-code half, iter-963)
- Holon bodies are fenced from the supervisor's secrets and their slot (BUGS OS-1, OS-2 implicit path, OS-6, OS-7, OS-8, OS-9, OS-10; AD-006, iter-967). Residual: an explicit read of the operator's chair key by a same-uid body waits on the O-11 runner isolation.
- Org mode refuses a Holon run whose `policy.json` is missing (BUGS OS-3; AD-007, iter-967)
- Holon chair notices are retried until delivered, and driver placeholders must be plain tokens (BUGS OS-5 retry half, OS-11; AD-008; Holon commit `992b08f`, local to that repo)
- Gateway budgets meter per turn into a seat-spend ledger charged to the start seat, and a hard cap fails closed when its usage cannot be read (BUGS GW-1, GW-8; AD-009, iter-968). The ledger starts empty on upgrade.
- A chat's conversation id is kept on its session row: `/new`, a seat change and `/resume` pick a transcript that survives a restart, and the continuation summary is keyed by it (BUGS GW-5, GW-6, GW-7; AD-010, iter-968)
- Cron and socialization runs set their seat as the memory scope, and `/session` no longer names the premiere for an unbound chat (BUGS GW-2, GW-9; AD-010, iter-968)

## Pending

- Rebuild `/settings` over `operant.toml` (toml_edit, comments kept), tabs per config section, live-apply where supported; retire the legacy settings.json screen
- `[keybindings]` user overrides in `operant.toml` (`/keys` now shows the effective registry bindings + conflicts — iter-820; overrides remain)
- Unify the gateway's `agent.show_reasoning` with the TUI reasoning display mode
- TUI red-team (BUGS.md TUI-6): a shell-out spawn error skips the terminal restore (`crates/operant-cli/src/tui/adapter_types/tui_app.rs:1267`, `?` before `:1271-1272`). Verify: the restore runs on every exit path, including a spawn error, with a test that forces the spawn to fail. Status: the fix is applied (iter-966, the spawn result is held until the restore); the test needs a terminal and is not written.
- TUI red-team (BUGS.md TUI-7): the shell-out ignores the child's exit status (`tui_app.rs:341-343, 367-370`). Verify: a non-zero exit or a signal shows its status in the status line, with a test.
- TUI red-team (BUGS.md TUI-8): `/rename` ignores its argument and is not saved (`commands.rs:1003-1008`). Verify: `/rename <title>` writes the title to the database, with a test.
- TUI red-team (BUGS.md TUI-9): delegations stay "running" after `/new` or `/rewind` (`agent_events.rs:159-200`). Verify: the status entries clear on both commands, with a test.
- TUI red-team (BUGS.md TUI-10): the shell-out leaves mouse capture on for the child (`tui_app.rs:1240-1280`). Verify: capture is off for the child and back on after it, with a test of the toggle order.
- TUI red-team (BUGS.md TUI-12, TUI-13): command descriptions do not match their handlers (`crates/operant-cli/src/tui/operant_app/app.rs`, `app/commands.rs`). Verify: each description names what its handler does; change the description, not the handler, with a test that the registry text matches.

## Review

Owner decisions, not autonomous work. Until ruled, make no change in the area an item names. Items with a recommendation give it at the end.

- Autonomous publish (`crates/operant-cli/src/autonomous.rs:660`, `publish_changes`): it runs `git add --all .`, so any uncommitted file in the checkout (a peer's BUGS.md rows, `Cargo.lock`, `xd:/`) is committed to `[autonomous] git_branch`. Recommendation: stage only the paths the agent changed. Until then, start `operant autonomous` only when `git status --short` is empty.
- Context management (magic-context review): agent-driven reduction (`ctx_reduce`) and a background historian instead of the synchronous summarize rung — larger core changes
- BUGS.md R42-8: two live Telegram bot tokens unredacted in gateway.log
- Seat red-team (org-tui-foundation-003): the default employee for new org-mode chats. Options: a `[holon] default_employee` roster id, or leave new chats unbound. `premiere` is not on the roster, so AD-004 rules it out. Recommendation: option (c), new chats stay unbound. GW-9 changes only the `/session` report, which fits (c).
- TUI red-team (BUGS.md TUI-1): a pending approval prompt is painted under later overlays, and its keys go to whichever overlay is open. Overlaps `docs/ROADMAP-TUI-FLEET.md` §6.1, which deferred the dispatch-order fix. Decide: reorder the gates, or make `dialog_priority()` the real router. Recommendation: reorder the gates so the approval dialog comes first; defer the router rewrite.
- TUI red-team (BUGS.md TUI-11): some TUI actions run blocking I/O on the event loop (SQLite branch queries, `git diff`). Magnitude is unmeasured. Decide whether to move them to `spawn_blocking` now or after profiling. Recommendation: profile once, then decide.
- KB red-team (BUGS.md KB-6, retention half): every turn is retained, including turns from untrusted channels, so a channel message can be stored and replayed. The escape and data label are fixed (iter-962). Decide which channels are untrusted (for example, anything but the owner's chat) and whether they skip `sync_turn`. Recommendation: skip retention for non-owner channels, and keep recall unchanged.
