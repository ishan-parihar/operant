# TUI — the terminal interface

Owns: the terminal UI's architecture — the renderer pipeline, the
model/view layering, the composer, overlays, and the debug/repro tooling.
Not for: the slash-command registry semantics (commands are app logic),
session storage ([sessions.md](sessions.md)).

Source: `crates/operant-cli/src/tui/` (300+ modules — `mod.rs` is the
map), with the headless simulation and live-repro mechanics in
[tui-debugging.md](tui-debugging.md) (companion page).

## The layering (four planes)

| Plane | Modules | Role |
|---|---|---|
| **App** | `tui/app/` (key_handling, mouse, commands, agent_events, channel_drains, turn_state, scroll_emit, …) | event routing + state machine |
| **Model** | `tui/operant_model/` (adapter, transcription, vendor_types) | the App's data, flattened by `display_messages()` |
| **Render core** | `tui/operant_render_core/`, `operant_markdown/`, `operant_render/`, `render/` | backend-neutral Document → ratatui cells (markdown, tables, selection, themes) |
| **UI assembly** | `tui/operant_ui/` (ui_prepare) | ONE frame assembly: every row renders in exactly ONE section |

Reference source: the jcode-parity port (`../parent-projects/jcode`), the
ground design for the visual layer.

## The invariants (load-bearing — read before touching render code)

- **One renderer.** `display_messages()` (operant_model/adapter.rs) is
  THE flatten layer; `ui_prepare.rs` is THE frame assembly. Every row
  renders in exactly ONE section — never both Body and Streaming.
- **Live partials never enter settled rows.** Streaming
  text/thinking render only in the mutable Streaming section; anything
  committed to `App.messages` is settled scrollback. Violating this
  double-renders at two wrap widths (the "shredded fragments" family,
  iters 784/789).
- **Reasoning display is a three-state contract**: `Off` (never
  accumulate), `Current` (the default: committed in place, hidden once
  the next user prompt lands), `Full` (every trace stays). Resolved
  through a process-global override cell in
  `tui/operant_app/config_shim.rs`, fed from `[display] show_reasoning`,
  flipped live by `/thinking-display`. Thinking is stored
  sentinel-wrapped (iter-780); the renderer keys off the sentinel, never
  role names.
- **Terminal-scroll mode** (`[tui] terminal_scroll_mode = true`):
  inline viewport, no alt-screen, native select-and-copy. The scrollback
  emitter (`tui/app/scroll_emit.rs`) is append-only with a watermark.
- **Tool rows anchor by arrival** (`after_index`); a tool call
  mid-response is "reserved" and renders in the Streaming section below
  the tails — no flushes at ToolStart.
- **Registered slash commands can never fall through to the model** —
  the not-wired catch-all covers both dispatch loops, and the unit test
  `no_registered_tui_command_is_unwired` pins it statically. Plain
  Enter dispatches a REGISTERED command even mid-stream (iter-1402:
  the command arms guard their own mid-stream safety — `/clear`,
  `/stop`, `/new`, `/undo`, `/retry`, `/compact` defer or refuse
  honestly, display toggles flip state — so the composer no longer
  swallows them while a turn streams; unregistered `/…` text still
  waits for idle). The
  functional-core subset now has gateway twins over a channel
  (see [gateway.md](gateway.md) §commands — RT-179).

## The composer (`tui/prompt_input/` + `file_discovery/` + `prediction/`)

Decomposed from a 5,305-line monolith into `kill_ring`, `typeahead`,
`vim`, `ghost` submodules. Three OMP-derived surfaces:

- **Word classes** — word-wise move/kill/delete ops share one boundary
  model (word / delimiter-run / CJK), joiners keep `don't` one stop;
  `vim.rs`'s motions keep VIM semantics deliberately.
- **@ mention grammar** — the typeahead triggers on `@`, `@file:`,
  `@folder:`; accepted FILES insert `@file:<path>` + space, accepted
  DIRECTORIES insert `@folder:<path>/` and chain. See
  [context-injection.md](context-injection.md) §references for the
  expansion grammar.
- **Ghost text** — the word at the caret completes from a
  personal-history trigram engine (`[tui] word_completion`, default
  `"ngram"`), TAB accepts + space, → accepts verbatim, divergence drops.
  Never shown while the popup is open or inside code spans.

## Overlays

`App::overlay_flags()` names every overlay; a full audit (iter-1002)
traced each to a production opener — none test-only. Notable:
permission dialogs (real prompts, iter-20), session browser/branching,
diff viewer, journey view (learning graph), skills/MCP/plugins panels,
settings screen (19 entries, all applied — iter-1176), stats, usage
overlay, theme screen, image paste/pinned images, osc8 hyperlinks.
The Ctrl+R history-search overlay previews the recalled prompt live in
the composer as the selection moves and restores the pre-search draft
on Esc (iter-1403, jcode parity).

## Redraw discipline (`tui/redraw.rs`)

Adaptive redraw interval by performance tier (Minimal for SSH/WSL,
Normal, High) × activity state (streaming, idle, deep idle) × focus —
5-10x CPU reduction on idle terminals.

## Debug + repro

`operant tui debug simulate --keys … --mouse …` drives the real event
handlers against a TestBackend; the corpus lives in
`crates/operant-cli/tests/tui_scenarios/` (goldens). Live repro: tmux +
`OPERANT_TUI_DEBUG=1 OPERANT_TUI_EVENT_LOG=…`. Full mechanics in
[tui-debugging.md](tui-debugging.md). **Rebuild the debug binary after
every edit** — stale cached binaries produced false regression readings
twice.

## Intention

The TUI is a port of jcode's visual layer with the same contract: the
frame is assembled in one place, streaming and settled content never
mix, and every overlay is reachable (a dialog no code opens is dead
code). The corpus + headless simulator exist because TUI regressions
are otherwise only found by hand — 64 gated scenarios now run in the
local gate.

## Transcript navigation keys (iter-1461)

`tui/app/scroll_anchor.rs` is the scroll state machine; the chords that
drive it are dispatched in `tui/app/key_handling.rs`. Beyond the row
ladder (PageUp/Down, Ctrl+Up/Down ±3, Alt+Up/Down ±20, Shift+Home/End,
Ctrl+G bookmark), the transcript is navigable by **prompt**:

- **Alt+[** jumps to the previous user prompt; **Alt+]** to the next
  (jcode's `scroll_to_prev/next_prompt`, and jcode's own DEFAULT chords
  — not fallbacks).
- Target rows come from `PreparedChatFrame::wrapped_user_prompt_starts`
  on the frame the renderer **last drew** (`operant_ui::last_chat_frame`)
  — the same seam jcode reads. No second copy of the starts exists
  anywhere: an earlier iteration of this idea kept a live mirror and the
  mirror could disagree with the paint.
- While auto-following, the first Alt+[ lands on the MOST RECENT prompt
  (the reader is at the tail, so "previous" is the last boundary above
  it) and pauses follow; paused, each press walks strictly earlier, and
  past the first prompt it clamps to row 0 rather than pivoting.
  Alt+] is a no-op while following, walks strictly later when paused,
  and past the last prompt resumes following.
- Both jump functions consume `scroll_memory.pending_resize`. An armed
  resize anchor re-pins the view on the next paint, so without the
  consume the reader snaps straight back to where they jumped from.

### Why the advertised chords are Alt+[, not Ctrl+[

`Ctrl+[` is byte `0x1b` — the Esc key — and crossterm's kitty parser maps
codepoint 27 to `KeyCode::Esc` (it is special-cased, before the generic
`Char(c)` arm). So the SAME physical key reports differently per mode:
`Esc+CONTROL` under the kitty protocol (pushed at boot since iter-1426),
`Char('[')+CONTROL` under tmux extended-keys, and indistinguishable from
plain Esc in the legacy path. `Ctrl+]` is worse: the legacy raw-byte
path maps 0x1D to `Char('5')+CONTROL`, a different chord entirely. All
three spellings are dispatched (`Char('[')+CONTROL`, `Char(']')+CONTROL`,
`Esc+CONTROL`), but the catalogue and help overlay advertise only the
Alt chords because those are the ones that work in every mode.

### The three Esc consumers are the load-bearing part

Every always-live `KeyCode::Esc` arm in `handle_key_event` must gate on
`KeyModifiers::NONE` or it swallows the Esc+CONTROL spelling of the
prompt jump. There are three, and each failure was a different
operator-visible regression:

1. `Esc if is_streaming` (cancel) — a mid-stream prompt jump would
   cancel the running turn.
2. `Esc if !is_streaming` (double-Esc → rewind selector) — would arm the
   500 ms double-tap clock, so the next plain Esc opened the fullscreen
   rewind selector instead of collapsing.
3. the transcript-focus `Esc` (mouse-focused pane) — would silently
   switch focus back to the composer instead of jumping.

Modal surfaces' Esc arms (dialogs, pickers, copy mode, the hubs) are
correctly UNguarded: a modal's Esc closes the modal whatever the
modifiers, which matches jcode.

The regression tests for this family live in
`tui/app/tests.rs::prompt_jump_tests`; the `streaming_esc_still_cancels_but_ctrl_esc_does_not`
and `transcript_focus_esc_returns_focus_but_ctrl_esc_jumps` pair is
what keeps a future refactor from re-widening any of the three arms.
## The session hub's preview pane (iter-1476)

`tui/session_hub/` is the hub's own module: `mod.rs` holds the state
(`SessionHubState`), `render.rs` paints it, and the App side wires the
background loads (`tui/app/mod.rs` + `key_handling.rs`). Two mechanisms
carry their own rules.

**The pane focus is what the step keys route on.** `PaneFocus`
(List/Preview, jcode's `session_picker/navigation.rs:191-196` — the hub
has 40/60 geometry but until iter-1476 it had no focus field, so Tab and
PageUp were keys with nothing to act on). Tab toggles it, `h`/`l` set it
explicitly, and Up/Down/j/k, PageUp/PageDown/K/J and Home/End each route
to the FOCUSED pane: selection in the list, scroll in the preview.
`Right` nudges the preview into focus as the discoverable way in for an
operator who never presses Tab.

**`Left` deliberately does NOT switch focus.** operant's Left is omp's
close-tap (armed by the ←← opener, one further Left closes
`agent-hub.ts:2460-2475`), where jcode uses Left/Right for pane focus.
This is a recorded divergence, not an oversight — do not "fix" it into
jcode's shape.

**The preview's scroll bound is measured, then applied.** The paint path
is read-only by design, so `render_preview` writes the lines-minus-
pane-height bound into `FrameCtx::preview_max_scroll`, which
`draw_operant_overlays` copies into `App::last_preview_max_scroll` (a
`Cell`, the same interior-mutability seam the layout-publish block uses)
and the frame loop applies by `set_preview_max_scroll` BEFORE any key
step is taken. Without that order the arrow keys scroll past the last
row into blank space, and a stale offset from a longer transcript
persists.

**Three states, never a lie.** With a cached transcript the pane renders
it (one row per message: role label, flattened text, tool-call labels
parsed off the stored JSON, and a placeholder row for a message with no
visible text — `preview_message_from_row`); with nothing cached and
nothing in flight it reads "No transcript cached — press r to load" (and
`r` reloads in either pane); only a load that is genuinely running shows
the ⏳. The replaced text ("Loading session…") was the operator's stuck
report: it was emitted on a timeout the switch had already overtaken.

**A late load is dropped, not painted.** The row id rides inside the
channel message and `apply_preview` refuses a load whose id is no longer
the selection, so moving the cursor while a preview is in flight cannot
show the wrong transcript. The load owns its own channel
(`App::preview_rx`) precisely so the list and pane loads are independent
flights.

### `user_turns` re-seeds on session switch (core, not TUI)

`user_turns` is a process-global list that only ever grew
(`record_user_turn` pushes, nothing cleared). Loading an OLD session —
the TUI's session switch, the cron path — therefore left the rewind
selector navigating either an EMPTY list (the turns happened in an
earlier process) or a STRANGER's (whichever session turned last). Every
key read as dead. `set_session_id` now calls
`reseed_user_turns_for_rehydration`, which re-derives the marks from the
rehydrated conversation (user-role, non-empty, index-ordered), so the raw
`rewind_marks()` list and the gateway's conversation-filtered
`conversation_rewind_marks()` can no longer disagree. The rewind fork
benefits from the same rule: it persists its prefix before retargeting,
so the post-fork mark list stays exact.
