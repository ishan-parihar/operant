# 09 — Let the terminal own scrolling (native scrollback by default)

## Requirement (user, 2026-10-14)

"The scroll is not working properly. It must be like omp/jcode, where the terminal handles
the scrolling, rather than simulating a scrolling environment."

## Facts that shape the design

- **oh-my-pi** renders in the main screen. Finished transcript output is printed once and
  scrolls into the terminal's own scrollback. Only a live region at the bottom is redrawn.
  Full-screen pickers (the rewind selector, for example) temporarily use the alternate
  screen (`selector-controller.ts:1300-1310`: "Fullscreen alternate-screen overlay … the
  normal screen stays untouched"). On display reset it clears native scrollback and
  replays the transcript (`input-controller.ts:590-596`).
- **jcode actually uses the alternate screen** (`ratatui::init` at
  `jcode-tui/src/tui/app/commands.rs:3259`) with mouse capture, and scrolls in-app. It is
  not a native-scroll reference. Use oh-my-pi for the behaviour; use jcode only for its
  message-level rendering.
- **operant already has the mechanism, off by default:** `[tui] terminal_scroll_mode`
  (`operant-core/src/config.rs:715`).
  - It runs `Viewport::Inline(14)` with no alternate screen (`tui/adapter_types/tui_app.rs:421-457`).
  - `emit_settled_transcript_rows` writes settled rows into scrollback via
    `Terminal::insert_before` (`tui/app/scroll_emit.rs`).
  - The viewport floors its scroll at the emitted watermark (`operant_ui/ui_viewport.rs:422-428`).
  - Known defect **K-3** (docs/audit/2026-06-19-BUGS.md): the watermark counts *rows* and assumes the settled body
    only grows. Tool rows grow on ToolComplete, and `Current` reasoning disappears at the
    next prompt, so emission slices misaligned rows (duplicates or gaps in scrollback).
  - ratatui 0.30.2 supports `insert_before` and `resize`, and has a `scrolling-regions`
    feature.

## Design

### 1. Emit by message, not by row count

- Replace the row watermark with a **message watermark**: `emitted_through: usize`, an
  index into `display_messages()`, plus the width it was emitted at.
- A display message is *emittable* when it is final:
  - user prompts: immediately;
  - assistant text: once committed (boundary flush or Done);
  - tool rows: once status is Done/Error/Cancelled;
  - system annotations: immediately;
  - reasoning: see 3.
- Emit messages strictly in order. Stop at the first non-final one, so nothing after a
  pending tool row is emitted early.
- Each emission renders that message's rows with the body renderer, at the current
  width, through `insert_before(rows.len(), …)`.
- Rows stay byte-identical to what the alt-screen body would show, because the same
  renderer is used.
- Must land after plan 01: a duplicate emitted into scrollback can never be removed.

### 2. A live region that fits its content

Everything not yet emitted (the current turn's streaming tails, pending tool rows, the
batch-progress band) plus the composer and status line render in the inline viewport.
Its height = min(content, terminal height − 1), recomputed per frame. If it changes,
recreate the inline viewport. Spike first: check whether ratatui 0.30's `resize` on an
inline terminal does this without flicker; if not, use `Terminal::with_options` with a new
`Viewport::Inline(h)` after `clear`. When the live content is taller than the screen (a
long streamed answer), emit its *completed paragraphs* early. Markdown blocks are final
once a blank line follows them outside a fence. This keeps the live region at most a
screen tall, the way oh-my-pi's renderer pushes finished lines up.

### 3. Things that can't be edited once they are in scrollback

| Feature | Native-scroll behaviour |
|---|---|
| `Current` reasoning (collapses at the next prompt) | Emit reasoning as a single collapsed line, `▸ thought for 12s`, when its turn settles. The full trace is visible only while live. |
| Ctrl+O expand tool output | Prints the expanded output **below** as a new block (append), and a second press prints nothing (notice: "already expanded below"). |
| Ctrl+T / Alt+O visibility toggles (plan 08) | Affect live and future rows only. Notice: "applies to new output; press Alt+L to redraw history". |
| `/clear`, `/cls`, rewind (plan 05), session switch, theme change | **Replay:** clear the screen and native scrollback (`\x1b[H\x1b[2J\x1b[3J`), then re-emit the whole transcript at the current width. This is oh-my-pi's resetDisplay. |
| Resize | Always replay, debounced 150 ms: the screen and the operant-owned scrollback are redrawn at the new width, so no line stays wrapped at the old width (oh-my-pi behaviour). No config option. |
| Alt+L | Manual "redraw history" = replay (oh-my-pi `app.display.reset`). |

### 4. Input model in native mode

- **Mouse capture off by default.** The terminal then owns the wheel, scrolling and
  selection (native selection copies exactly what is on screen). In-app selection and copy
  mode are disabled in this mode. `/copy` (last answer) and Alt+Y copy mode remain
  available as keyboard-driven, live-region-only features.
- **PageUp/PageDown:** a TUI in the main screen can't scroll the terminal's scrollback. Map
  them to nothing and let the terminal's Shift+PageUp work. tmux users use copy-mode. The
  help overlay states this.
- **Full-screen overlays** (help, `/keys`, model picker, rewind selector, settings,
  session browser, diff viewer) enter the alternate screen for their lifetime, then leave
  it. The main screen is untouched, the same as oh-my-pi. Small popups (autocomplete,
  pickers that fit in the live region) render inline.

### 5. Quality bar ("like oh-my-pi/jcode, without issues")

- **No flicker or tearing:** every frame and every `insert_before` is wrapped in the
  synchronized-update bracket that is already used for alt-screen frames (DEC 2026).
- **Smooth streaming:** the live region repaints at most at the adaptive frame rate. Text
  emitted to scrollback appears in whole lines, never character-by-character.
- **Correct widths:** the glyph-width table from plan 01 S2 applies to everything
  emitted, because a misjudged width in scrollback is permanent.
- **tmux and plain terminals:** test under tmux (scrollback via copy-mode),
  kitty/ghostty/alacritty, and a non-kitty-protocol terminal. Native mouse-wheel
  scrolling and selection must work in all of them.
- **Exit:** on quit, the live region is replaced by its final rows, so the terminal holds
  the complete transcript after exit, exactly as with oh-my-pi.
- **Crash safety:** a panic hook restores the cursor and line discipline, and leaves
  already-emitted history intact.

### 6. Default flip

When acceptance passes, `terminal_scroll_mode` defaults to `true` (decision 5 in INDEX; confirmed by the user, who wants native scrolling fully functional).
Alt-screen mode stays available (`terminal_scroll_mode = false`) for users who want in-app
scrolling and selection, and it keeps all current behaviour.

## Steps

1. Message-watermark emission (replaces row watermark). Fixes K-3. Tests:
   - emitted rows == concatenated body rows for a scripted multi-turn, multi-tool session
     (use `--frame-per-event` and capture `insert_before` payloads in a `TestBackend`
     wrapper);
   - no message is emitted twice;
   - none is emitted while non-final.
2. Dynamic live-region height + early paragraph emission. Spike, then implement. Test:
   streaming a 200-line answer never makes the live region exceed screen height, and the
   final scrollback contains each line once.
3. Replay (`/clear`, `/cls`, rewind, resize, Alt+L). Test: after replay, the emitted
   stream == a fresh emission of the transcript.
4. Reasoning collapse line, Ctrl+O append semantics, toggle notices.
5. Overlays to the alternate screen. Test each overlay's enter/leave in a `TestBackend`
   that records mode switches.
6. Mouse capture off in native mode; help and README text.
7. Live verification (below), then flip the default and update `operant.example.toml`,
   README and CHANGELOG. Corpus: add `native-scroll-*` scenarios (the corpus already
   supports `config` fragments; schema field `config`).

## Acceptance (live, in the user's terminal and inside tmux)

- A 10-turn session with tools: the mouse wheel scrolls the terminal natively and smoothly
  from the first prompt to the live region. No row appears twice, and no row is missing,
  compared against `/transcript`.
- Selecting text with the mouse and pasting it gives exactly the on-screen text.
- Resize: the history is replayed at the new width, and no artifacts remain.
- `/rewind` and `/clear`: the history shown matches the agent context.
- Esc, Ctrl+T, Alt+O and Ctrl+O behave as specified, and overlays don't disturb
  scrollback.
