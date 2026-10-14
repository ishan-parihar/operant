# 01 — Duplicate tool rows that disappear after scrolling

## Symptom (user report, 2026-10-14)

During and after tool-heavy turns, the transcript shows redundant tool-call rows. Scrolling
up and back down makes them go away.

## What scrolling actually changes (why the symptom narrows the cause)

The run loop draws every iteration (`app/mod.rs:1046-1060`, `terminal.draw` is not gated).
The prepared frame is cached by content keys only: `FullPrepCacheKey`
(`operant_ui/ui_prepare.rs:685-716`) has no scroll field, and neither does the body cache
key (`:1146-1166`). So a scroll **does not rebuild the prepared content**. It only
re-slices it, and ratatui then rewrites every cell whose buffer value changed.

So "heals on scroll" points first at **terminal cell residue**: the terminal shows
something different from ratatui's back buffer, and nothing rewrites those cells until a
scroll changes them. A content bug (wrong rows in the prepared frame) would survive a
scroll-and-return, unless it lives in a cache whose key happens to change on scroll.
**Do not guess. Run the two oracles below first**, and fix whatever they convict.

### Suspects, ranked

- **S1. Terminal residue from writes outside ratatui's diff.** After `terminal.draw`,
  the loop writes directly to the terminal twice: `crate::osc8::emit_hits(&osc8_hits)`
  (`app/mod.rs` right after the draw, re-emitting URL cells wrapped in OSC 8) and
  `crate::tui::pinned_images::emit_strip`. If either writes a cell sequence whose width
  differs from what ratatui recorded, or at a stale position, the terminal and the buffer
  diverge. ratatui then never repaints those cells, because its buffer says they're
  already right.
- **S1b. Direct `println!`/`eprintln!` while the TUI owns the screen.** These land at
  whatever the cursor position is and are invisible to ratatui:
  - `app/commands.rs:1053` (`/keybindings` failure);
  - `operant_app/permissions.rs:851-869`;
  - `operant_app/storage.rs:209,216` (settings backup recovery);
  - `operant_app/logging.rs:215,219,610` (logger write failures);
  - `operant_app/info_widget/info_widget_git.rs:581,586`;
  - `operant_ui/session_picker.rs:2456`.

  Fix them regardless of verdict: route them through `tracing` or a status notice, and add
  `#![deny(clippy::print_stdout, clippy::print_stderr)]` on the `tui` module, with
  `#[allow]` only where the terminal is provably not in raw mode.
- **S2. Width disagreement on glyphs in tool rows.** Rows use `○ ● ⛭` (East-Asian
  *Ambiguous*), `⚡ ⏳ ⏰ 🐝 🧠 📣` (*Wide*), and `⚠` with a possible VS16 (U+FE0F appears in
  `app/tui_state_impl.rs`, `operant_app/output_style.rs`, `operant_app/tui_fns.rs`,
  `operant_ui/ui_input.rs`). When `unicode-width` and the terminal disagree, every later
  cell on that row is shifted by one in the terminal but not in the buffer. Ghost
  fragments of the previous frame's row then survive until the row is fully rewritten,
  and a scroll does exactly that.
- **S3. Incremental body-cache reuse.** `build_body_from_base` (`ui_prepare.rs:1240-1330`)
  reuses cached rows by prefix hash (`matching_prefix_len`, `:2126`) and by *suffix* hash
  (`matching_suffix_len` + `prepare_body_prepended`). Suffix reuse was written for
  "older history prepended", but it fires for any transcript whose tail hashes match while
  something earlier changed. That happens mid-turn when a pending tool row completes
  while later rows are unchanged, or when tails flush and reserved rows return to the Body
  (`operant_model/adapter.rs:150-170`). If a row's rendering depends on anything outside
  its own `stable_cache_hash` (neighbour grouping, the live/settled position,
  prompt-number color), the stitched frame can repeat or misplace rows. This can't heal
  on scroll unless the cache entry changes, so it ranks below S1/S2. It can't be ruled
  out by reading, because **none of these paths is exercised by any test** (`cfg!(test)`
  bypass).
- **S4. Live tool rows rendered with the wrong context.** `prepare_streaming_cached`
  (`ui_prepare.rs:2496-2522`) renders `live_tool_rows` through `render_message_into` with
  `messages: &live_tools` and `idx` local to that slice. Any renderer that groups
  consecutive tool rows, or looks at `messages[idx-1]`, sees a different neighbourhood than
  the Body does, so the same call can render twice in two shapes across the ToolComplete
  boundary.

## Step 1: build the two oracles (one iteration, no behaviour change)

1. **`--frame-per-event` for the simulator** (see INDEX, "Cross-cutting test
   infrastructure"). When `is_simulating && frame_per_event`, `drain_agent_events` handles
   one event per loop pass.
2. **Cache-coherence oracle** (content bugs, S3/S4). Behind
   `OPERANT_DEBUG_PREP_ORACLE=1` (read once into a `OnceLock<bool>`), at the end of
   `prepare_messages` on a non-test build, also run `prepare_messages_inner(app, w, h)`
   uncached. Compare `wrapped_plain_lines` (or the joined plain text of every section),
   and on mismatch `tracing::error!` with the frame index, the cache path that produced
   the frame (`exact | incremental | prefix_reuse | suffix_reuse | full`, already returned
   by `build_body_from_base`), and a unified diff. In the simulator, make a mismatch fail
   the run (`--assert prep_oracle_mismatches == 0`, exposed through the existing assert
   plumbing).
3. **Agent scripts that reproduce real turn shapes** (`tests/tui_scenarios/agent_scripts/`):
   - `parallel-tools-two-iterations.json`: thinking → content → 3× `tool_start` → completes
     out of order → thinking → content → 1 tool → content → done. (A ready-made version
     is in the scratchpad of the planning session; recreate it, it's 15 lines.)
   - `tool-progress-and-error.json`: tool_start → tool_progress ×3 → tool_error, then a
     second tool that completes.
   - `many-identical-tools.json`: 8× `read_file` with the same args and output (hash
     collisions between rows).
   Run each with `--frame-per-event --capture-frames 0,1,...,N` and the oracle on. A
   scenario per script goes into the corpus (`*.scenario.json`), with an `assert_screen`
   that each tool name appears exactly as many times as it was called. That needs a new
   assert form, `count:<needle>==N`, in `cmd_tui_debug.rs`.
4. **Terminal-coherence capture** (residue bugs, S1/S2). Add a debug action (on the F12
   debug overlay, key `d`) that writes `~/.operant/debug/frame-<ts>.txt` with: the
   ratatui front buffer as text, each row's cell symbols with their recorded widths, and
   the list of OSC 8 hits and pinned-image strips emitted after that draw. Then reproduce
   live in tmux: run `operant` in a tmux pane against a real model with a 3-to-5-tool
   prompt. When duplicates appear, press F12 → `d` and run `tmux capture-pane -p -e -t <pane>`
   at the same time. If the pane differs from the buffer dump, it's S1/S2. If they agree,
   it's a content bug, so go back to the oracle.

## Step 2: fix by verdict

- **S1 (post-draw writes):** stop writing outside the frame. OSC 8 can be applied inside
  the draw: ratatui 0.30 cells hold multi-char symbols, so wrap the first cell's symbol as
  `ESC]8;;url ESC\ + text` and close on the last cell. Or keep the overlay, but after
  writing call `terminal.backend_mut()` cursor restore and mark the touched rows dirty, so
  the next diff rewrites them: `Terminal::current_buffer_mut()` reset for those rows
  forces a rewrite. For pinned images, do the same for their rows. Test: a unit test
  driving `TestBackend` that asserts no bytes are written after `draw` other than the
  sync-end sequence, with OSC 8 enabled and a URL row on screen.
- **S2 (widths):** normalize every glyph used in transcript chrome to width-unambiguous
  code points. Drop VS16 everywhere, and replace Ambiguous/Wide pictographs in rows that
  re-render often (tool rows, spinners, status) with narrow equivalents (`✓ ✗ · ▸ ▪`), or
  pad them explicitly. Add a test that walks the tool-row renderer's output for every
  status and asserts `UnicodeWidthStr::width(symbol) == terminal_width_estimate(symbol)`,
  using a table of the glyphs we allow (anything else fails the test). This is the same
  kind of guard jcode keeps in its icon module.
- **S3/S4 (cache):** the oracle names the reuse path.
  - If it's `suffix_reuse` outside a genuine prepend: gate suffix reuse on an explicit
    "history was prepended" signal (the compacted-history loader sets a one-shot flag),
    not on hash coincidence.
  - If it's prefix reuse with context-dependent rendering: either include the
    dependency in `DisplayMessage::stable_cache_hash` (for example, "previous row is a
    tool row" for grouping), or truncate the reuse base one message earlier than the first
    mismatch.
  - If it's S4: render live tool rows as a suffix of the real display list (pass the full
    `display_messages() ++ live_tools` slice and the true index), so a call renders with
    the same neighbours before and after it settles.

## Acceptance

- The oracle reports 0 mismatches across all corpus scripts with `--frame-per-event`.
- New corpus scenarios pass, with each tool row counted exactly once in every captured
  frame.
- Live: a 5-tool turn in tmux, with `tmux capture-pane` taken every 200 ms during the turn
  (a small script). No frame shows a tool row twice, and no capture differs from the
  buffer dump.
- The cause is written into `CHANGELOG.md` and the fixed suspect is recorded in `BUGS.md`.
  Rejected suspects go in too, with the oracle evidence that cleared them.

## Non-goals

Re-architecting the caches. If the oracle shows the caches are coherent, leave them alone.
