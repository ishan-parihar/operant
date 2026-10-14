# 08 — Thinking and tool-call show/hide shortcuts

## Symptom (user, 2026-10-14)

"The shortcuts for thinking hide/show and tool-calls hide/show are not working on the TUI."

## Root causes

1. **No key is bound to either action.** The global dispatch
   (`tui/app/key_handling.rs:1234-1480`) and the registry
   (`tui/keybindings/defaults.rs`) have no thinking or tool-visibility action. The user's
   muscle memory comes from oh-my-pi (`parent-projects/oh-my-pi/packages/tui/src/app-keybindings.ts:111-141`):
   - `app.thinking.toggle` = **Ctrl+T**. In operant, Ctrl+T enters copy mode
     (`key_handling.rs:1433`).
   - `app.tools.toggleVisibility` = **Ctrl+Shift+O**. In operant, that arrives as Ctrl+O
     (expand the latest tool output) unless the terminal reports Shift, and nothing
     handles the shifted form.
   - `app.tools.expand` = Ctrl+O. This one matches operant.
2. **"Thinking off" doesn't hide live thinking.** `/thinking-display off` sets the
   override and hides *committed* reasoning (`operant_model/adapter.rs:128-135`,
   `ui_prepare.rs:1726-1740`). The Streaming section renders `streaming_thinking`
   unconditionally (`ui_prepare.rs:2459-2481`), so thinking keeps streaming on screen while
   "off".
3. **`[tui] show_tool_calls` is a dead config field.** It is defined in
   `operant-core/src/config.rs:716` (default `true`), and nothing in the TUI reads it.
4. **Each flip writes a system message into the transcript.** `/thinking-display` pushes
   "Thinking display: …" (`app/commands.rs:1519`). A key toggle must only show a transient
   status notice.

## Design (decisions 2 and 3 in INDEX)

| Key | Action | Notes |
|---|---|---|
| **Ctrl+T** | Toggle thinking: if the effective mode ≠ Off, remember it and set Off; else restore the remembered mode (initially the config's: `show_reasoning = true` → Current). | oh-my-pi parity. Copy mode moves to **Alt+Y** (jcode's copy-mode key); Alt+Y yank-pop is dropped from the defaults. |
| **Ctrl+Shift+O** | Toggle tool activity visibility. | Needs kitty keyboard reporting, which operant already requests (`operant_app/keyboard.rs:12`, DISAMBIGUATE). Match `Char('o' \| 'O')` with CONTROL+SHIFT. |
| **Alt+O** | Same as Ctrl+Shift+O. | Fallback for terminals or tmux setups without extended keys, where Ctrl+Shift+O arrives as Ctrl+O. |
| Ctrl+O | (unchanged) Expand/collapse the latest tool output. When tools are hidden, it shows them first, then expands. | |

**Hidden tool activity renders as a single dim row per contiguous run of tool rows**:
`▸ 3 tool calls · read_file, shell, grep (Alt+O to show)`, with error count when > 0:
`▸ 4 tool calls · 1 failed`. The row:
- is a UI row: copy excludes it, using the same `is_ui_row` predicate as iter-806;
- updates live as calls complete;
- also replaces `live_tool_rows` in the Streaming section while hidden.

The state lives in `App::show_tool_calls`, initialized from `config.tui.show_tool_calls`,
which finally wires that field. Pinned todos and the batch-progress band are not tool rows,
so they stay.

## Steps

1. **Tests first** (`tui/app/tests.rs`):
   - `ctrl_t_toggles_thinking_and_restores_previous_mode`: Current → Ctrl+T → Off →
     Ctrl+T → Current. Also from Full. Reasoning display messages appear and disappear
     accordingly, and no system message is added.
   - `thinking_off_hides_live_streaming_thinking`: feed a `Thinking` event, then switch
     Off. The prepared frame's Streaming section contains no thinking text. RED today.
   - `ctrl_shift_o_and_alt_o_toggle_tool_visibility`: after a 3-tool turn, toggling
     hides the 3 rows behind one summary row; toggling again restores them. Repeat during
     streaming (reserved rows).
   - `show_tool_calls_false_in_config_starts_hidden`.
   - `ctrl_t_no_longer_enters_copy_mode` and `alt_y_enters_copy_mode`.
   - Registry: both new actions are present, `the_default_keymap_has_no_conflicts` still
     passes, and Alt+Y is no longer yank-pop.
2. **Thinking:**
   - add `App::toggle_thinking()` (remember-and-flip using
     `config_shim::replace_reasoning_display_override`, then `invalidate_transcript()` and
     `set_status_notice("Thinking hidden" | "Thinking shown (current)")`);
   - make `/thinking-display` share the flip code, and change its system message into a
     status notice;
   - gate the thinking splice in `prepare_streaming_cached` on
     `reasoning_display() != Off`.
3. **Tools:**
   - `App::show_tool_calls`, plus `toggle_tool_visibility()`;
   - in `operant_model/adapter.rs::display_messages`, when hidden, collapse each
     contiguous run of tool rows (including `tool_rows_after` groups) into one
     `DisplayMessage::system`-styled summary row, with a dedicated role or flag so the
     renderer dims it and copy treats it as UI;
   - `live_tool_rows` returns the single summary row when hidden;
   - make sure `stable_cache_hash` changes when visibility flips: the rows change, so it
     will, but assert it in a test.
4. Key wiring in `key_handling.rs` (before the composer's `Char(c)` fallthrough) and in
   the registry (`keybindings/defaults.rs`) with descriptions. Move copy mode to Alt+Y,
   and remove the `Alt+Y` yank-pop arm (`key_handling.rs:1438`).
5. Status line: a compact indicator when hidden (`thinking off`, `tools hidden`), so
   hidden state is never a mystery.
6. Docs: README shortcuts table, CHANGELOG, and `operant.example.toml` comment on
   `show_tool_calls` (now live).

## Acceptance

Live in the user's terminal: Ctrl+T during a streaming turn makes the thinking text
disappear immediately and reappear on the second press. Alt+O and Ctrl+Shift+O both
collapse and expand tool rows during and after a turn. `/keys` lists all three, and the
help overlay (plan 10) shows them.
