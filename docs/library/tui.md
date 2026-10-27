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
  `no_registered_tui_command_is_unwired` pins it statically.

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
