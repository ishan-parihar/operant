# 05 — Rewind selector: preview, search, and a real context rewind

## Requirement (user, 2026-10-14)

"Context-modification commands like rewind [need] a separate functional UI, which can
preview the context and perform search on the previous prompts, just like how oh-my-pi
implements it."

## Today's `/rewind` is cosmetic (a bug, not just a UI gap)

- `open_rewind_flow` (`tui/app/turn_state.rs:124-142`) lists every display message as an
  80-char preview. `handle_rewind_flow_key` (`app/key_handling.rs:2017-2053`) then does
  `self.messages.truncate(idx)` on the **TUI display list only**.
- The agent's stored conversation (`OperantAgent::conversation`) is untouched, so the
  model still sees everything that was "rewound", while the screen says otherwise.
- `tool_use_blocks` are not truncated, and `invalidate_transcript()` isn't called, so
  stale rows can linger. The rewound prompt's text isn't returned to the composer.

## Reference: oh-my-pi (`parent-projects/oh-my-pi`)

- `packages/tui/src/overlays/rewind-selector.ts` (header comment, `:1-30`) is a
  **fullscreen** overlay that replays the transcript and outlines the block a rewind would
  land on.
  - Up/Down step through rendered items; Left/Right jump between user turns.
  - `f` opens a filter: typing narrows the list to matching items, and Esc leaves the
    filter with the selection kept.
  - Enter rewinds, `A` loads earlier turns, Esc cancels.
  - When the turn has sibling branches, a side strip shows them.
- `packages/coding-agent/src/modes/controllers/selector-controller.ts:1269-1400`
  (`showUserMessageSelector`, `#rewindFromTranscript`):
  - rewinding uses `navigateTree`, which keeps the old path as a sibling branch (it doesn't
    delete);
  - a user-prompt target rewinds *past* itself, and its text goes back into the editor.
- `input-controller.ts:570-600`: **double-Esc on an empty editor** opens the rewind
  selector (setting `doubleEscapeAction = rewind | tree | none`, default `rewind`).

## Design

### Core: rewind = fork at a user turn (decision 4 in INDEX)

- Each real user prompt gets a stable turn id. In `turn_context` step 4
  (`agent/turn_context.rs:150-160`), when the user message is added, record
  `UserTurnMark { turn_id: u64, conv_index: usize, conv_epoch: u64, text: String }` in
  `OperantAgent::user_turns` (a `RwLock<Vec<UserTurnMark>>`).
  - `conv_epoch` increments on any non-append mutation of the stored conversation:
    `micro_compact`, `compact_history_now`, `clear_history`, the preflight ladder
    eviction/summary that rewrites the store.
  - Nudge, continue and steer messages are user-role but are not turns; they are never
    recorded.
- Emit `AgentEvent::TurnStarted { turn_id }` right after recording, so the TUI can attach
  the id to its display row for that prompt.
- `pub async fn rewind_to_turn(&self, turn_id) -> Result<RewindOutcome>`:
  1. Find the mark. If `conv_epoch` is unchanged, the cut point is `conv_index`.
     Otherwise relocate it by scanning the store from the end for the user message whose
     content equals `text` (unique match required). If not found, the turn was compacted
     away: return `Err(RewindError::Compacted)`, and the UI explains "That turn was folded
     by compaction; the earliest rewindable turn is #k."
  2. Refuse while a turn is running (the UI disables Enter while streaming).
  3. Fork: create a new session id (same generator as `/new`). Write the prefix
     `conversation[..cut]` as the new session's transcript (session store + DB, with
     `parent_session_id` = old id; the DB schema already has the column,
     `database.rs:41`), `set_session_id(new)` (`agent/events.rs:502`), and
     `notify_session_switch(new, old, reset=false)` (`events.rs:316`).
  4. LCM: ingest happens per session, so the new session starts from the prefix. Call
     `engine.ingest_turn(new_session, prefix)`.
  5. Return `RewindOutcome { new_session_id, parent_session_id, prompt_text, removed_turns }`.
- The old session is untouched and remains listed in `/sessions` (shown as a child of
  nothing, while the new one shows its parent). The session browser can later show
  branches as a tree.

### TUI: fullscreen rewind selector (replaces `overlays/rewind_flow.rs`)

- **Open** with `/rewind`, or double-Esc on an empty composer when not streaming (within
  500 ms; new config `[tui] double_escape_action = "rewind" | "none"`, default `rewind`).
  A single Esc keeps its current meanings.
- **Layout:** full-screen (in native-scroll mode, plan 09, this enters the alternate
  screen temporarily).
  - Top: a filter bar, hidden until `f` or `/` is pressed.
  - Body: the **real transcript rendered with the normal body renderer** (reuse
    `prepare_body` on the current display messages) with a dotted outline (or reverse-video
    gutter) around the selected user turn. Everything after the selection is dimmed: that
    part is the preview of what is removed.
  - Bottom: hints, plus a context preview line: `Keeps N messages · ~T tokens · drops M turns`.
    T is computed with `context_management::estimate_total_tokens` on the prefix (an
    agent call: `agent.preview_rewind(turn_id) -> (msgs, tokens)`, read-only).
- **Keys:**
  - Up/Down: previous/next user turn.
  - PageUp/PageDown: scroll the preview.
  - `f` or `/`: filter. Typing filters user turns by case-insensitive substring of the
    prompt text; matches are highlighted in the preview, and Enter in the filter jumps to
    the selection and closes the filter.
  - Enter: rewind to the selected turn.
  - Ctrl+O: expand tool output in the preview (same as the main view).
  - Esc: cancel (or leave the filter).
  - These keys go in the plan 10 registry under the context `RewindSelector`.
- **On Enter:** call `rewind_to_turn` through the agent-action channel
  (`TuiApp::run_pending_agent_actions`, as `/compact` does). On success:
  - rebuild the display from the agent's conversation: reuse the session-resume loader
    (`drain_session_load` path) so display and context come from one source;
  - clear `tool_use_blocks`/annotations that belonged to removed turns;
  - `invalidate_transcript()`;
  - put `prompt_text` into the composer (oh-my-pi behaviour), and show the notice "Rewound
    to before prompt #k · previous path kept as session <short-id>".
- **On `Err(Compacted)`:** show the inline message in the selector and keep it open.
- **`/rewind <n>`** (numeric argument): rewinds to before prompt #n without opening the
  UI, but asks for confirmation (y/n) in the status line.

### Sessions browser (small, same plan)

Ctrl+B (session branch browser) currently opens with an empty list
(`key_handling.rs:1266`). Make it list the current session's ancestors and children
(DB `parent_session_id`), with Enter to switch. This is how a user gets back to the
pre-rewind path.

## Steps

1. Core marks + `TurnStarted` event + `conv_epoch` bumps (one iteration, with tests).
2. Core `preview_rewind` + `rewind_to_turn` (fork). Tests below.
3. TUI selector component (`overlays/rewind_selector.rs`, new), wired to the core and
   replacing `rewind_flow`. Delete `rewind_flow.rs` and `message_selector.rs` if unused
   after. The corpus scenario `rewind-flow` is replaced by `rewind-selector` (and
   `rewind-selector-filter`).
4. Double-Esc + config + docs.
5. Ctrl+B branch list.

## Tests (RED first)

- Core: 3 turns, then `rewind_to_turn(turn2)`. The new session's conversation ==
  pre-turn-2 prefix, the old session's stored transcript is unchanged, `session_id()` is
  the new id, and the DB has `parent_session_id` set. The next `run("x")` sends a request
  whose messages don't contain turn 2 or 3 (assert on a mock client's captured request).
  This test is RED today, because nothing changes the agent.
- Core: compaction folds turn 1, then rewinding to turn 1 returns `Err(Compacted)`.
  Rewinding to turn 3 still works (relocated by text).
- Core: nudge/steer user messages don't create marks.
- TUI: open `/rewind`, press `f`, type a word unique to prompt #2, press Enter (closes the
  filter), Enter (rewind). The composer contains prompt #2's text, the display shows only
  prompt #1's turn, and the agent mock received `rewind_to_turn(id2)`.
- TUI: double-Esc on an empty composer opens it; while streaming it doesn't open; a
  single Esc doesn't open it.
- Corpus: `rewind-selector` (open + navigate) and `rewind-selector-filter` goldens.

## Acceptance

Live: three prompts, `/rewind`, select #2. The preview dims turns 2 and 3 and shows the
token estimate. Enter puts prompt #2 back in the composer. Asking "what did I ask you
before?" yields only prompt #1, which proves the context was rewound. `/sessions` shows
the old path, and Ctrl+B can switch back to it.
