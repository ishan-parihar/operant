# TUI visual layer — fix outline (2026-10-13)

Source: `docs/2026-10-13-tui-visual-layer-handoff.md` (on `origin/main` @ `f17daf72`;
local `main` is 1 behind) checked against the code at `2c5a14c9` + working tree.

## 0. Corrections to the handoff (read first)

- **The user runs alt-screen mode, not terminal-scroll mode.** `~/.operant/operant.toml`
  `[tui]` has no `terminal_scroll_mode` → default `false` (`operant-core/src/config.rs:723`).
  `emit_settled_transcript_rows` returns immediately (`app/scroll_emit.rs:35`).
  Handoff suspects A and B (scrollback emission, flush residue) **cannot cause what the user
  saw**. The handoff repro recipe forces scroll mode → it tests a surface the user isn't on.
  Repro in alt-screen; it can be done headless (`tui debug simulate --capture-frames`;
  TestBackend only breaks `insert_before`, which alt-screen never uses).
- **`[tui] refresh_rate_ms` is dead.** Nothing outside `config.rs` reads it. The loop
  paces through `tui::redraw::redraw_interval` (`app/mod.rs:1093`). The user's
  `refresh_rate_ms = 80` does nothing.
- Data path (Reasoning → Content → Done once each) is clean, as the handoff says.
  The defects are on the render side and in copy.

## P0-1 — Live tails render twice while a tool row is reserved  ← main duplication

**Mechanism (confirmed by reading the code):**
- `operant_model/adapter.rs:150-170` (iter-785): when any tool block has
  `after_index > messages.len()` (reserved at ToolStart), `live_tail_rows()` (thinking + text)
  is **spliced into `display_messages`**, which becomes the **Body** section.
- `operant_ui/ui_prepare.rs:854-863`: `has_streaming` is true whenever the tails are non-empty,
  so `prepare_streaming_cached` **also** renders the same thinking + text in the Streaming section.
- Window: ToolStart → first ToolComplete (the whole tool run; with 5 parallel tools it is long
  and very visible). The screen shows `thinking/text → tool rows → … → thinking/text again`.
- That breaks the invariant the same file states at `adapter.rs:137-146` ("never render
  them as settled Body rows"). In scroll mode it would also freeze the partial into native
  scrollback, which is where the "cut off at col ~76" fragment would come from.

**Fix (one renderer per row):** remove the Body splice. Move the reserved tool rows into the
mutable tail: while tails are live, `display_messages` omits reserved rows, and the Streaming
section renders `tails → reserved tool rows` (render the rows via the body renderer on that
slice). This keeps iter-785's ordering (thinking above its tool) and the AG|ENTS.md seam
(still no flush at ToolStart).
Rejected: gating `has_streaming` off when reserved. That fixes alt-screen but leaves live
rows in the "settled" Body, so scroll-mode emission breaks again.

**Verify:** unit test in the adapter/ui_prepare tests: thinking + text + 2 reserved
ToolStarts → each tail string appears exactly once in the prepared frame's
`wrapped_plain_lines`, in order thinking < text < tool rows. Then ToolComplete → still once.
Corpus: `tool-split-stream`, `thinking-block-streaming`, `tool-block` goldens re-proved
(and the diffs reviewed).

## P0-2 — Copy reads a stale, viewport-clamped cell scrape  ← "select & copy not like jcode"

**Mechanism:**
- On release, `app/mouse.rs:1083-1108` copies `self.selection_text`. That text is written only
  by the **post-render** pass `render/selection.rs:374`, so it is one frame behind.
- `app/mod.rs:1105-1137` coalesces up to 32 buffered events per wake
  (`event_coalesce.rs:32`). A fast drag+release lands Drag…Up in one wake with no draw in
  between, so `selection_text` is empty (silent no-copy) or truncated to an older focus.
- The scrape clamps lines to the visible window (`selection.rs:261-268`), so a selection
  that runs past the viewport (edge autoscroll) copies only its visible part.
- jcode extracts the selection **synchronously from the prepared-frame snapshot**
  (`jcode-tui/src/tui/ui.rs:2369` `copy_selection_text`). That function is **already ported**
  at `operant_ui/mod.rs:2699` and has **zero callers**.

**Fix:** on Up (and in copy-mode live drag), compute the text with
`operant_ui::copy_selection_text(self.copy_selection_range()?)` against `last_chat_frame()`.
`copy_selection_range` is already implemented (`app/tui_state_impl.rs:999`). Keep the
render pass for **painting only**, and drop its text extraction plus the `reflowed_selection_text`
interior splice (that splice also compares section-local `raw_line` across sections at
`selection.rs:69`, so it can merge lines at the Body→Streaming boundary).

**Verify:** test: anchor+Drag+Up with **no render** between → clipboard capture is non-empty
and equals the expected text; a selection spanning beyond the viewport copies in full.
Corpus `selection-highlight` must still pass (paint unchanged).

## P1-1 — Full-mode reasoning commits a second copy of the answer  (owner decision)

The user's TOML has `show_reasoning = true` → `Full` (`config_shim.rs:253`, `main.rs:2830`).
At Done, `flush_streamed_assistant_message` commits `Thinking + Text`. The model's thinking
often drafts the answer (table and all), so even with P0-1 fixed the transcript shows the
answer twice: the dim draft, then the real one. jcode's default is `Off`. `Current`
streams the reasoning live and then collapses it.

Options (recommend **a**):
- **a.** `show_reasoning = true` → `Current`, plus port jcode's `TurnReasoningTrace` anchor
  (`reasoning_region.rs:150-200`: the trace stays until the next user prompt, then is GC'd)
  and the `▸ thought for Xs` header (resurrect `transcript_turn.rs:56` `reasoning_heading`).
  `Full` stays reachable through `/thinking-display`.
- **b.** Keep `Full`, but render committed thinking collapsed to the `▸ thought for Xs` line
  (expand on click).

Docs/config: `operant.example.toml`, README, CHANGELOG.

## P1-2 — `refresh_rate_ms` dead knob

Either feed it into `redraw_interval` as the streaming cadence, or delete it from
`TuiConfig` + `operant.example.toml`. Recommend **delete**: the adaptive tier already
paces frames, and a knob that does nothing is worse than no knob.
Re-test the user's "scroll and refresh out of sync" symptom **after P0-1**, since that
symptom is most likely the doubled tail jumping.

## P1-3 — Residual selection drift during streaming (verify after P0-1/P0-2)

Selection points are frame content-line indices (`mouse.rs:134`). Anything that changes
height above the anchor shifts the selected text. P0-1 removes the biggest shifter (the
splice that appears and disappears). What's left: tool rows growing on ToolComplete
(`output_preview`). Live-check in tmux. Fix only if it still reproduces.

## P2 — Cleanup / scroll-mode only

- **Delete `build_transcript_turns`** (`transcript_turn.rs:83`): no live callers; only
  `app/tests.rs:2153-2218` and comments use it. The adapter is the one path. Keep
  `reasoning_heading` if P1-1a uses it.
- **Scroll-mode watermark** (`scroll_emit.rs`): the row-count watermark assumes Body is
  append-only, but tool rows above the watermark change height and Current mode drops
  thinking at flush. Only matters when `terminal_scroll_mode = true`, so it is not the user's
  complaint. Log it in `docs/audit/2026-06-19-BUGS.md`. Fix if scroll mode becomes a target.
- Renderer line cache (jcode `get_cached_message_lines`): skip until a profile shows markdown
  re-render cost.

## Order + gates

1. P0-1 → 2. P0-2 → 3. owner picks P1-1 a/b → 4. P1-2 → 5. P1-3 live check → 6. P2.

Each fix ships as one iter, one push, from a worktree (the shared tree has peer
uncommitted edits in `operant-core/{config,client,lib}.rs`, `recap.rs` and others, which are
not ours). Gates per the handoff:
`cargo fmt --all`; `AR=/usr/bin/ar cargo test -p operant-cli --bin operant --no-default-features`;
corpus `OPERANT_BIN=<fresh> python3 _run.py verify`; clippy gate; then a live tmux repro in
**alt-screen** with `show_reasoning = true` and the 5-tool prompt from the paste.
