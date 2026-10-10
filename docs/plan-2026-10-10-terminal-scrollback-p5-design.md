# P5 — terminal-level scrollback: design + investigation record (2026-10-10)

Owner requirement (re-stated after the P4-1 fix): scrolling must be
**terminal level**, not a simulated ratatui page — so a long conversation
can be selected and copied via the terminal's own scrollback, including
everything off-screen. This supersedes the v2 outline's "jcode is also
alt-screen, so parity ≠ native scrollback" framing: the requirement is not
parity, it is native scrollback. jcode cannot serve it (its launcher calls
`ratatui::init()`, which enters the alternate screen — verified at
`src/cli/terminal.rs:353` + ratatui-0.30.2 `src/init.rs:400`). The port has
no jcode reference; it uses ratatui's own first-class support for exactly
this shape of app.

## The API (verified in the vendored ratatui-0.30.2 source)

- `init.rs:416` documents the init variant that does **not** enter the
  alternate screen: `try_init_with_options(TerminalOptions { viewport })`.
- `Viewport::Inline(n)` (init.rs:129-137 doc example): the app renders in
  the **last n rows** of the terminal; everything above is the terminal's
  normal buffer.
- `Terminal::insert_before(n, |buf| …)` (same doc example): prints `n`
  rows above the inline viewport into the native scrollback — ratatui's
  own doc example is literally a chat/streaming app.

## Design

**Mode, not cutover.** `tui.terminal_scroll_mode = true` (config; or
`--inline` flag) selects the mode. Default stays the current full-screen
renderer until the mode is live-verified; flipping the default is its own
decision.

**Architecture in the mode:**
1. Init: `try_init_with_options(TerminalOptions { viewport:
   Viewport::Inline(status_rows + composer_rows + stream_window) })` —
   the inline region holds exactly what the bottom strip holds today:
   status/usage row(s), the streaming bubble window, the composer, the
   key-hint strip.
2. Transcript emission: every settled transcript row — committed message
   lines, tool rows, system annotations, todo card — is emitted exactly
   once via `terminal.insert_before`, tracked by a last-emitted-row
   watermark. Flush boundaries (Done / ToolComplete / ToolError / system
   push) are the emission points — the same boundaries iter-725 defined.
   The streaming bubble stays in the inline region and its final content
   is what gets emitted; nothing is emitted twice by construction (the
   watermark + the 744 retry-clear both protect this).
3. Scrolling: the terminal owns it. PgUp/Alt+Up etc. reach the terminal
   (tmux copy-mode, host scrollback). The internal scroll ladder
   (`scroll_offset` machinery) retires in this mode; `scroll_anchor.rs`
   survives only as the window math for the inline region.
4. Selection: native terminal selection over emitted rows — the owner's
   "select all the items in a long scroll and copy" requirement is
   satisfied by construction; the TUI's internal selection stays for the
   inline region only (and P4-5 pane-scoping still applies to it).

**What must change around it:**
- The P4-2 notification purge should land FIRST: banners/error modals are
  alt-screen overlay concepts; in inline mode their surfaces are the
  status row + emitted cards, which is what the purge routes to anyway.
- Right-margin info widgets and the ambient animation cannot float over
  scrollback — in this mode they render inside the inline region or stay
  disabled (ambient mode already has a kill switch).
- Resize: `note_resize`/anchor reconcile becomes near-trivial (the
  terminal reflows its own scrollback; only the inline window geometry
  matters).
- Ctrl+L / `/cls` (iter-750): in inline mode, "clear the rendered view"
  reduces to emitting a spacer + collapsing the inline window — the
  jcode spacer semantics for real this time.

**Corpus impact (the real cost):** the scenario runner's TestBackend
renders full-screen frames; in inline mode the "screen" is the inline
region and the emitted rows are `insert_before` calls. The runner needs a
mode-aware golden: assert the inline region AND the ordered emission log
(the watermark gives the runner a deterministic "what was emitted" list).
The 64 existing goldens stay valid for the default mode; the mode gets its
own small scenario set (emit-order pins, no full-frame goldens).

**Wave breakdown (each its own iteration):**
- P5-1: mode plumbing (init options + flag + config) with the current
  renderer unchanged, inline region rendered empty — proves the terminal
  handoff (no alt-screen, resize, focus, mouse for the composer).
- P5-2: transcript emission at flush boundaries + watermark + emit-order
  unit tests.
- P5-3: streaming bubble in the inline window; tail behavior = jcode's
  (bubble grows, flushes into emission).
- P5-4: corpus mode support + scenarios; live tmux verification
  (copy-mode PgUp reaches history; native selection across emitted rows).
- P5-5: retire the internal ladder in the mode; default-flip decision.

## Duplication complaint — investigation record (2026-10-10)

The user reports streaming output still duplicated on the iter-749 binary.
Evidence gathered today:
- Persisted transcripts clean: `database.db` recent assistant messages show
  each reply once (lighthouse/caravan stories, tool results).
- Live frame-pair captures on iter-749: no duplicated rows across 8
  captures of a streaming turn; single-frame duplicate-key scan: zero.
- Handler paths traced and re-verified: boundary flush takes the buffer
  (no residue); Done discards `message.content` when the buffer is
  non-empty; iter-744 clears the buffer at RetryScheduled.
- The remaining live-only suspects: (1) a session started on a pre-744
  binary (running processes keep the old inode — if the duplicate was
  seen in a long-lived session, `operant --version` from a fresh shell is
  the discriminator), (2) the gateway double-emitting SSE content deltas
  without a RetryScheduled event.
- Forced-repro status: a double-emitting SSE proxy was built
  (/tmp/dup_proxy.py) but the repro was blocked by a NEW finding: **the
  `-c <config>` flag is silently ignored for provider selection by
  `chat`/`run`** (a session pointed at a dead endpoint still answered).
  That is itself a bug (config plumbing), and the repro needs it fixed
  first — next session: fix `-c`, then rerun the proxy repro against the
  TUI; if the TUI shows the doubled text, the dedup belongs in the agent
  stream parser or the TUI buffer layer with care (legitimate repetition
  exists).

## Command-sweep backlog (from iter-750's committed instrument)

37/106 registered commands consumed. The ~69-handler backlog is tiered by
cost: status-message tier (version/usage/info/model-status — data paths
exist in `tui debug`), dialog/panel tier (login/models/agents/observe —
vendored pickers exist for several), agent-flow tier (improve/refactor/
judge/review — jcode runs real loops; each is a project). The sweep script
is the tracker: `python3 tests/tui_scenarios/command_sweep.py`.

## Thinking status

Landed iter-749: default ON (jcode `show_thinking = true`), `/thinking-display
off|full|current` verified live in the notice strip. Visible thinking
additionally requires a provider that emits a reasoning channel — the
configured kilo gateway emits none under any dialect (4 probes: bare,
`thinking.type=enabled`, `reasoning_effort`, three models).
