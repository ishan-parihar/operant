# 07 — Native scroll: fix the watermark, then finish plan 09

**Size:** L. **Finding:** 9 in `01`. Plan of record:
`docs/plans/2026-10-14-batch/09-native-scroll.md`. Its design stays. This file
adds the bugs I found in what landed and reorders what is left.

**Owner decisions in force** (00-INDEX "Resolved with the user"): native
scroll must work fully, like oh-my-pi. On resize, history is **always**
replayed at the new width, with no option to turn that off.

## Bugs in the landed code (iter-839, "step 1")

Step 1 was planned as a **message** watermark. What landed is still a
**row-count** watermark, clamped to the last final message
(`crates/operant-cli/src/tui/app/scroll_emit.rs:41-49`).

1. **No reset when the transcript shrinks.** `scroll_emitted_rows` is set only
   in `scroll_emit.rs` (and `init.rs:163`). `/clear`, `/new`, rewind
   (`tui_app.rs:966`), session resume and `/compact` all shrink
   `App.messages`. Then:
   - `emittable <= watermark` returns early (`scroll_emit.rs:85-87`), so new
     rows are never written to history until the transcript grows past the
     old row count;
   - the live viewport floors its scroll at the stale watermark
     (`operant_ui/ui_viewport.rs:423`, `scroll.max(app.scroll_emitted_rows())`),
     so the same rows are also **hidden from the live view**.
   Case: 500 rows emitted, `/clear`, type a prompt, and the answer is invisible
   in both places until 500 new rows exist.
2. **Resize drops rows.** On a width change the watermark jumps to the new
   `emittable` and returns (`scroll_emit.rs:52-58`). Anything that settled since
   the last emit is never emitted. The module comment (`:17-20`, "never
   re-emits at the new width … upgrade path … if a user ever reports it") goes
   against the owner decision above.
3. **Re-layout of already-emitted rows.** The thinking toggle (Ctrl+T), the tool
   toggle (Ctrl+Shift+O / Alt+O), `/thinking-display`, the theme and `Current`
   reasoning GC all change the row count of messages that were *already emitted*.
   A row watermark then points into the wrong message: duplicates or gaps.
   This is the K-3 class. The clamp narrowed it but didn't remove it.

## Fix order (each one is an iteration)

### Step 1′ — the real message watermark (replaces the row watermark)

- `App.scroll_emitted_through: Cell<usize>`: an index into
  `operant_model::adapter::display_messages(app)` (one past the last emitted
  message). Delete `scroll_emitted_rows` and change `ui_viewport.rs:423` to
  floor at the wrapped row count of messages `[0, emitted_through)` in the
  **current** frame. That number comes from `message_boundaries` (already
  used in `message_final_rows`, `scroll_emit.rs:120-140`).
- Emit loop: for `i in emitted_through..final_display_message_count(app)`,
  render message `i`'s rows from the prepared frame's boundaries, then
  `insert_before`. Advance `emitted_through` only when `insert_before` returns `Ok`.
- Delete the row-based `committed_prefix_rows` path if nothing else reads it.

Test (TestBackend wrapper that records `insert_before` payloads, the plan-09
step-1 test, which never landed):
- multi-turn, multi-tool script with `--frame-per-event` → concatenated
  payloads == the body rows of a fresh render. No message is emitted twice, and none
  is emitted while non-final.
- Ctrl+T toggled between turns → no duplicate or missing rows. RED today.

### Step 2 — `replay()` (plan 09 step 3, moved up because bugs 1–2 need it)

`fn replay_native_history(&self, terminal)`: write `\x1b[H\x1b[2J\x1b[3J`
inside the DEC 2026 synchronized-update bracket the alt-screen path already
uses, then set `emitted_through = 0`. The next emit pass re-emits everything
at the current width.

Call it on: `/clear`, `/new`, `/cls` (it then prints the divider), rewind,
session switch (resume, branch), `/compact`, theme change, the Ctrl+T and
Alt+O toggles (instead of plan 09's "applies to new output" notice; the
user wants it fully functional, and replay makes toggles retroactive for
free), and **resize, debounced 150 ms**. Alt+L = manual replay.

Make `03` §2 call it (that file says so).

Tests:
- `/clear` after 3 turns, then 1 turn → payloads after the clear == that 1 turn
  only, and the live view shows it. RED today (bug 1).
- Resize 110→80 while a message settles in the same frame → the message appears
  once, at width 80. RED today (bug 2).

### Steps 3–7 (unchanged from plan 09, in this order)

3. Dynamic live-region height + early paragraph emission (plan 09 step 2; spike
   ratatui 0.30 inline `resize` first).
4. Reasoning collapse line `▸ thought for Xs` (resurrect `reasoning_heading()`,
   marked `#[allow(dead_code)]` in `transcript_turn.rs`, per the 2026-10-13
   handoff) and Ctrl+O append semantics.
5. Full-screen overlays enter and leave the alternate screen (help, `/keys`,
   pickers, rewind, settings, session picker, diff viewer). The inline pickers
   from `05` B render **inline** (they fit the live region). That is a reason to
   do `05` B before this step.
6. Mouse capture off in native mode. Help and README text (Shift+PgUp, tmux copy-mode).
7. Live verification under tmux + kitty/ghostty/alacritty + one non-kitty
   terminal, then flip `terminal_scroll_mode` to default `true` in
   `crates/operant-core/src/config.rs:905` and `operant.example.toml`, README and
   CHANGELOG.

## Quality bar (from plan 09 §5, restated as checks)

- [ ] No flicker: every frame and every `insert_before` is inside the synchronized-update bracket.
- [ ] After `/quit`, the terminal holds the full transcript (live region replaced by its final rows).
- [ ] A panic restores the cursor and line discipline. Emitted history stays intact.
- [ ] Wide glyphs (plan 01 S2) are measured with the same width table everywhere a row is emitted.
- [ ] tmux: scrollback through copy-mode contains each line exactly once.

## Note on the headless limit

The 2026-10-13 handoff says `TestBackend` makes `insert_before` inert. Build
the recording wrapper (step 1′ test) first. It is the only way any of this is
testable headlessly, and the corpus can't see scrollback without it.
