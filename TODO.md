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
- [x] Rebuild `/settings` over `operant.toml` (iter-1176, RT-171): the screen's OWN save path was discarding what you typed. `settings_screen/state.rs::apply_and_save` offered 19 editable entries and matched FIVE, so toggling `notifications`, `verbose`, `vim`, `reduce_motion`, `show_cwd`, `fileInjectionEnabled` and 8 more rendered the new value, stored it in `pending_changes`, and dropped it in `_ => {}` — while still rewriting the whole `settings.json`. Five of the snapshot's fields were CONSTANTS (`verbose: false`, `cursor_blink_enabled: false`, `disable_claude_mds: false`, `file_injection_enabled: false`, `OutputFormat::default()`), so saving ANY setting reset all five to defaults. All 19 keys now apply, every snapshot field is carried from `settings`, and two regression tests pin it — one derives its key list from `all_entries` so a newly offered entry with no applier fails the build. **Residual, still open:** the screen still persists to the LEGACY `settings.json` rather than `~/.operant/operant.toml`, and 4 of the 19 entries write `AppConfig` which `apply_and_save` never saves — so `max_tokens` and `compact_threshold` still do not survive a restart. The toml_edit rebuild (tabs per config section, comments kept, live-apply, retire the settings.json screen) is the remaining half

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
- The TUI's approval prompt answers its key before any overlay (BUGS TUI-1; AD-011, iter-969)
- `/rename` saves the confirmed name, and a bare `/rename` starts once the list loads (BUGS TUI-8; AD-012, iter-969)
- A delegation whose turn is cleared is marked abandoned, and the HUD shows a running one first (BUGS TUI-9; AD-013, iter-969)
- A shell-out restores the terminal before its result is reported, and frees the mouse for the child (BUGS TUI-6, TUI-7, TUI-10; AD-014, iter-969)
- The Ctrl+B fork tree is read off the event loop, and a database error is shown (BUGS TUI-11 branch half; AD-015, iter-969)
- The TUI's command descriptions match their handlers, and `/mouse` reports the real capture state (BUGS TUI-12, TUI-13, iter-969)
- The TTS engine's ONNX Runtime panic is a tool error, and the release profile unwinds (BUGS CR-1, CR-3; AD-016, iter-970)
- ONNX Runtime is pinned to 1.24.4 in every script and AGENTS.md, and `install.sh` refuses a library without API 24 (BUGS CR-2; AD-017, iter-970)
- `operant doctor` reports a tool available only through its configuration, flags a second `operant` on PATH, and the config loads once (BUGS CR-4, CR-5, CR-6; AD-018, iter-970)

## Pending

- `[keybindings]` user overrides in `operant.toml` (`/keys` now shows the effective registry bindings + conflicts — iter-820; overrides remain). The blocker is the DISPATCH: `tui/app/key_handling.rs` still runs the if-chain, and the registry is the catalogue, not the dispatcher — the keybindings module calls migrating it "a separate, riskier change". Until then the table is read-and-ignored with a warning, and `operant tui keybindings` prints the bindings in effect instead of opening a dead file (iter-1133, RT-146).
- Unify the gateway's `agent.show_reasoning` with the TUI reasoning display mode

## Review

Decided on 2026-10-19; each choice and its reason are in the record named. The owner can overturn any of them with a new record.

- Autonomous publish: AD-027. The loop stages and commits only the paths its tick changed.
- Seat default for new org chats: AD-028. New chats stay unbound. The comment that calls an unbound session ungoverned is still open for review.
- Context management: AD-029. The synchronous summarize rung stays; agent-driven reduction and a background historian are not built.
- BUGS.md R42-8 (Telegram tokens in gateway.log): AD-030. The filter is covered by a test; the historic log is not rewritten; rotation is the owner's call.
- TUI-11, git-diff half: AD-026 (closed). KB-6 retention: closed by iter-971 (AD-019).
