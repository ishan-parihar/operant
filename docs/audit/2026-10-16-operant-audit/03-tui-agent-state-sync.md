# 03 — The TUI must drive the agent, not a copy of it

**Size:** M. **Findings:** 2, 3, 5, 6, 15 in `01`.

## The pattern behind all of these

`App` (the TUI state) keeps its own `messages`, `model_name`, `fast_mode`,
`settings.permission_mode`, `session_goal` and `is_streaming`. The agent
(`Arc<OperantAgent>`, owned by `TuiApp`) keeps the real ones. Slash commands
edit `App` and print success. Only six things cross over, through
`TuiApp::run_pending_agent_actions` (`crates/operant-cli/src/tui/adapter_types/tui_app.rs:875-1032`):
repair, branch list, rewind, rewind preview, effort and compact.

**The fix pattern, used everywhere below:** the slash arm sets a
`pending_<x>` field on `App`. `run_pending_agent_actions` drains it and calls
the agent. Then it rebuilds the display from the agent:
`agent.conversation()` → `App.messages`, the same code the rewind branch
already has at `tui_app.rs:962-985`. **Extract that rebuild into one
`fn rebuild_display_from_agent(app, conv)` and reuse it.** Don't add a new layer.

## §1 Concurrent runs: `/stop`, `/clear`, `/new` mid-stream (finding 2)

Evidence:
- `crates/operant-cli/src/tui/app/commands.rs:618-632` `"stop"` sets
  `is_streaming = false` and clears `tool_use_blocks`. It never touches
  `agent_interrupt`.
- `commands.rs:555-575` `"clear"` and `:637-655` `"new"` also set
  `is_streaming = false` ("so new input isn't silently dropped").
- Composer path `tui_app.rs:418-450`: if `!is_streaming` it spawns
  `agent.run(input)`. Nothing checks that the previous `agent_task_handle`
  finished.
- The correct cancel is `App::cancel_streaming_turn` (used by `/cancel`,
  `commands.rs:778-785`, and Esc). It triggers the interrupt flag and arms the
  3 s watchdog (plan 04).

Fix:
1. `"stop"` → `return self.intercept_slash_command_with_args_impl("cancel", args)`.
   Delete the old arm body.
2. `"clear"`, `"new"`: if `self.is_streaming`, call `cancel_streaming_turn()`
   and set the pending clear/new (§2). The drain runs it after the run
   completes (`drain_run_complete`). Don't force `is_streaming = false`.
3. Composer spawn guard (`tui_app.rs:418`): treat
   `agent_task_handle.as_ref().is_some_and(|h| !h.is_finished())` as streaming
   (steer-queue it, or say "a turn is still stopping"). This is the one guard
   that covers every caller. Put it inside `submit_user_message` after §5
   merges the two paths.

Test first (`crates/operant-cli/src/tui/app/tests.rs` style, or a corpus
scenario with `--agent-script` + `--frame-per-event`):
- Script a long streaming turn. Send keys `/stop<enter>hello<enter>`. Assert
  that the agent's `run` was entered once, not twice: count `TurnStart` events
  or the mock's call count. RED today.

## §2 `/clear`, `/new`, `/undo`, `/retry` must change the agent's context (finding 3)

Evidence: `commands.rs:555-575` (clear), `637-655` (new), `660-680` (undo),
`686-718` (retry). All of them edit `App.messages` only. Agent APIs that already
exist (`crates/operant-core/src/agent/events.rs`): `clear_history()`,
`set_session_id()`, `conversation()`. In `agent/rewind.rs`: `rewind_marks()`,
`rewind_to_turn()`. `main.rs:2639` already calls `agent.clear_history()` for
the CLI chat loop.

What each command must do:

| Command | Agent side | Display side |
|---|---|---|
| `/clear` | `agent.clear_history().await`. It discards only the current session (W3-1 semantics). | rebuild from `agent.conversation()` (empty) |
| `/new` | mint a new session id the way `agent/rewind.rs:118` does (`format!("sess_{}", uuid::Uuid::new_v4())`; extract a tiny helper and use it in both places) → `agent.set_session_id(new)` + `notify_session_switch(new, old, false)`. The old session stays in the store and in `/resume`. | rebuild (empty), reset title and goal |
| `/undo [N]` | `let marks = agent.rewind_marks()`. Take the Nth-from-last mark's `turn_id`, then `agent.rewind_to_turn(turn_id)`. This forks, so the old path is kept (decision 4). | rebuild + put the undone prompt in the composer (rewind already does `set_prompt_text`) |
| `/retry` | same as `/undo 1`, then submit the returned `prompt_text` | rebuild, then `pending_retry_query = Some(prompt_text)` |

So `/undo` and `/retry` become thin wrappers over the rewind machinery that is
already tested. Delete their hand-rolled `messages.truncate` code.

Native scroll: each of these must also call the replay from `07` §2. Until `07`
lands, at least reset `scroll_emitted_rows` to 0 and `scroll_emitted_width`
to 0.

Test first:
- Mock agent, two turns, then `/clear`, then assert
  `agent.conversation().await.is_empty()`. RED today (the TUI clears only its copy).
- `/retry` after one turn: assert the conversation holds the user prompt
  **once** after the retried turn. RED today (twice).

## §3 Permission mode must reach the permission decision (finding 5)

Evidence:
- Readers of `settings.permission_mode`: `tui/render/utils.rs:123,138`
  (status badge) and tests. That's all.
- Writers: `/plan` `commands.rs:601-615`, `/yolo` `commands.rs:1097-1123`
  (writes `settings.json` too, **with no confirmation**), the `/permissions`
  picker `key_handling.rs:199`, and `turn_state.rs:165`.
- The decision point is `App::drain_permission_requests`
  (`tui/app/channel_drains.rs:272-333`). It shows a dialog for every
  request, or auto-approves via the bash prefix list.

Fix (one place, every caller goes through it): at the top of the `while let Ok(req)` loop:

```rust
match self.settings.permission_mode {
    PermissionMode::BypassPermissions => { let _ = req.response_tx.send(ToolPermissionResponse::AllowSession); continue; }
    PermissionMode::Plan => { let _ = req.response_tx.send(ToolPermissionResponse::Deny); /* notice: "plan mode: read-only — /permissions to change" */ continue; }
    _ => {}
}
```

The core's hardline and blocked checks run before the channel
(`crates/operant-core/src/agent/stream.rs` ~1040-1080), so bypass can't
widen a hardline block. Check that this is still true when you implement it.

`/yolo`: open `bypass_permissions_dialog` (the same confirm `/permissions` uses)
instead of writing the mode directly. Persisting to `settings.json` happens
only on confirm.

`Plan` mode only denies tools that reach the permission channel (gated tools).
Read-only tools still run, which is the intended "plan before acting". Say so
in the `/plan` notice.

Also: `drain_permission_requests` auto-**denies** a second request while a
dialog is open (`channel_drains.rs:283-290`). Tools run up to 8 in parallel
(`stream.rs:1608` `buffer_unordered(8)`), so two gated tools in one batch → the
second one is silently denied. Queue them instead: keep a
`VecDeque<ToolPermissionRequest>` and open the next one when the current dialog
closes. That's about 10 lines in the same function and in `dialog_routing.rs:155`.

Test first:
- `PermissionMode::Plan`, then a mock `PermissionRequest` → assert the response
  is `Deny` and no dialog opens. RED today.
- `BypassPermissions` → `AllowSession`, no dialog. RED today.
- Two requests back-to-back → two dialogs in order, no auto-deny. RED today.

## §4 Bash "always allow" prefix (finding 6)

Evidence: `tui/app/dialog_routing.rs:237-241` stores
`command.split_whitespace().next()`. `channel_drains.rs:297-311` auto-approves
when the first word matches.

Fix (smallest safe change; the danger classifier already exists in core):
- Store the first **two** tokens (`git status`, not `git`). Store one token only
  for single-token commands.
- Before auto-approving: refuse if the command contains any of
  `; && || | \` $( > < \n`, or if
  `operant_core::approval::ApprovalGuard::check_dangerous(cmd).is_some()`
  (`crates/operant-core/src/approval.rs:508`). Take the constructor from the
  same file. Don't write a second classifier.
- Delete the `#[cfg(test)] bash_command_allowed_by_prefix` twin
  (`dialog_routing.rs:248-252`) and test the real path.

Test first: allow `git status`, then `git push --force` → dialog;
`cd /tmp && rm -rf ~` → dialog; `git status --short` → auto. The first two
are RED today.

## §5 One submit path (finding 15)

Evidence: the composer path `tui_app.rs:429-454` is a near-copy of
`submit_user_message` (`tui_app.rs:486-526`), minus `recap_activity.notify()`
and minus `clear_failed_input_recovery()` (which the composer path needs to
*keep* armed, so pass a flag).

Fix: `fn submit_user_message(&mut self, agent, text, from_composer: bool)`.
The composer path keeps its steer-queue branch first, then calls it. This
deletes about 25 lines and fixes the recap.

Test first: the recap engine with a short idle window. Complete a turn, then
type a prompt through the composer path → assert no recap fires during the
second turn. RED today.

## §6 `/fast` and `/goal` (small, same pattern)

- `/fast` (`commands.rs:591-600`) flips `App.fast_mode`, which nothing in the
  agent reads (grep: only the picker reads it). Either wire it (if the client
  supports a priority flag, the gateway stores `fast_mode` in session metadata
  at `gateway_commands.rs:1571-1595`, so check what reads that), or **delete
  `/fast` and the picker's fast column.** Ask the owner. Default: delete. It
  is an OpenAI/Anthropic-specific knob and the endpoint is the omp gateway.
- `/goal` (`commands.rs:731-753`) says it is "injected so the agent sees it",
  but it only adds a display annotation. Either send it through the steer
  queue / `pending_user_message` as a system note, or change the notice to
  "shown in the status bar only".

## Acceptance (whole file)

- Every RED test above is green. The corpus has 0 unexpected drift.
  Re-prove only the goldens whose text changed (state that in the commit).
- In a manual tmux run with a live model: `/clear` then "what did I just ask?"
  → the model doesn't know. `/plan` then "create file x" → refused with the plan
  notice.
- `README.md`: update the `/stop`, `/clear`, `/undo`, `/retry`, `/plan` and
  `/yolo` descriptions. CHANGELOG entry.
