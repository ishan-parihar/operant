# TUI jcode-parity gap outline v2 — 2026-10-09 deep source audit

Supersedes `plan-2026-10-09-tui-jcode-parity-gaps-v1.md` (v1's wave set
carried forward; this revision adds root causes the first pass could only
name as suspects, pins jcode's exact mechanisms from source, and records the
four still-live user complaints against the iter-731 deploy).

Audit method this pass: ratatui 0.30.2 vendored source read
(`~/.cargo/registry/.../ratatui-0.30.2/src/init.rs`), jcode source read
(launcher, bus, todo tool, todos_view, conversation_state, commands), operant
live tmux repro (todo + text-then-tool session), and 4 raw SSE probes against
the configured kilo gateway.

## The four live complaints → pinned causes

### 1. "Todos: 0/0 done, 0 in progress"

Live-captured (also "0/2", "2/2" on later calls — the counts are real). The
one-liner IS the entire todo surface today:

- The tool works (`tools/todo_tool.rs`), the agent loop derives counts and
  emits `AgentEvent::TodoUpdated{total, completed, in_progress}`
  (`agent/run.rs:2014`, parse at `todo_tool.rs:289`), and the TUI renders the
  one-liner (`tui/app/agent_events.rs:520`).
- **The event carries counts only** — the full list JSON sits in
  `result.content` and is discarded at the event boundary. No TUI surface can
  render a list from counts.
- The 0/0 case: the model called `todo` with an empty/mismatched payload
  (parse yields `todos: []`) — legal input, rendered as a meaningless line.
- jcode's contract, verified in source: the todo tool persists the list
  (`save_todos`, `jcode-app-core/src/tool/todo.rs:920-935`), then publishes
  `BusEvent::TodoUpdated(TodoEvent{session_id, todos: Vec<TodoItem>})`
  (`jcode-base/src/bus.rs:46-49`) — the **full list**. The TUI
  (`app/local.rs:300`) calls `refresh_todos_view_now()`
  (`app/todos_view.rs:211`): refreshes the cached todos page AND feeds the
  info-widget `todos` band. Operant's equivalent renderers
  (`info_widget_todos.rs` — vendored, tested) and `/todos` registration exist;
  only the store + payload + handler are missing.

**P4-3 plan (concrete):** extend `AgentEvent::TodoUpdated` to carry
`todos: Vec<TodoItem>` (parse from `result.content` where the counts already
come from; derive counts instead of carrying both); add `App.todos` (+
`todo_goals` per the vendored types); replace the one-liner with a store
update; feed `InfoWidgetData.todos` (`tui_state_impl.rs:686`) and
`pinned_todos_payload` (`:168`); implement the `/todos` handler as a
live-updating inline card (jcode behavior: card, again-to-dismiss, `pin`
toggle via the existing `display.pin_todos` shim knob). Unit test: TodoUpdated
with 3 items renders the card; corpus scenario: script `todo` tool_start +
tool_complete carrying the JSON, assert card rows + widget band.

### 2. "The output message streaming is duplicated"

Reproduced the trigger class; root cause pinned in code. The duplication is
**retry-dependent** — which is why it appears "sometimes" and constantly on
this provider (the org's known stream-death flake):

- `AgentEvent::RetryScheduled` (`tui/app/agent_events.rs:472-487`) pushes a
  `SystemAPIError` notice block and re-arms the turn, but **never clears
  `streaming_text`/`streaming_thinking` and never resets `is_streaming`**.
- The died attempt's partial text therefore stays in the buffer. The retry's
  first Content delta hits the re-seed guard `if !self.is_streaming`
  (`agent_events.rs:34-44`) — false, because `is_streaming` was never cleared —
  so the buffer is NOT re-seeded and the full retry text **appends** to the
  partial: the user watches the reply print twice (partial, then whole).
- Clean runs (no stream death) never duplicate — live-verified: a
  text→tool→text turn rendered each sentence exactly once.
- jcode clears streaming render state at every hard boundary
  (`clear_streaming_render_state()`, `conversation_state.rs:837` et al.);
  operant clears at Done/Error but not at RetryScheduled.

**P4-2.5 plan (one small iteration):** in `RetryScheduled`, clear
`streaming_text` + `streaming_thinking` (and reset `is_streaming = false` so
the retry's first delta re-seeds the seed/anchor machinery) before pushing
the notice block. Unit test: stream partial → RetryScheduled → Content delta →
assert buffer equals the new delta text only. Corpus scenario
`retry-dedup`: script content("alpha") → RetryScheduled → content("alpha
beta") → done; assert the transcript does not contain "alphaalpha" or a
doubled row.

### 3. "Scroll non-functional; must integrate terminal scroll like jcode"

Byte-level mechanism, now fully pinned:

- **jcode is an alt-screen app**: normal path `ratatui::init()`
  (`src/cli/terminal.rs:353`) → `execute!(stdout(), EnterAlternateScreen)`
  (ratatui-0.30.2 `src/init.rs:400`). The v1 correction stands. jcode does
  NOT stream its transcript into native terminal scrollback; there is no
  native-scrollback model to port. (ratatui 0.30 does ship a
  no-alt-screen init variant — `init.rs:416` — jcode does not use it.)
- What the user experiences as "integrated terminal scroll" in jcode is an
  **internal viewport that never goes dead**, with the complete ladder:
  Up/Down ±1 (history-nav-guarded), Ctrl+Up/Down ±3, PgUp/PgDn ±10,
  Alt+Up/Down ±20, prompt jumps, scroll bookmarks, tail-catchup animation
  (`ui_viewport.rs` TAIL_CATCHUP_*), resize/history anchors, smoothness
  instrumentation, mouse-queue animation — plus an OPTIONAL host-scrollbar
  bridge (`handterm_native_scroll.rs`, Unix-socket `PaneState{position,
  content_length, viewport_length}`) that only activates under the Handterm
  terminal host. jcode's `/cls`+Ctrl+L "spacer into scrollback" language refers
  to the resume/inherit path (`JCODE_RESUMING` exec-handoff runs on the
  primary screen — `terminal.rs:363-378`), not the normal session.
- operant's ladder keys are bound and the state machine is suite-pinned; the
  render seam eats it live (v1 finding, unchanged: 8× PageUp / Up / Alt+Up /
  Ctrl+Up = zero transcript movement with proven overflow; F12 proves keys
  arrive; zero corpus scroll-key coverage).

**P4-1 plan (unchanged, now with a decision point):** root-cause the render
seam (anchor-precedence `ui_viewport.rs:405-416` vs `max_scroll` sizing),
fix, add the `scroll-keys` corpus scenario, port the ladder completeness
gaps (prompt jumps, bookmarks, tail catch-up — machinery names in v1).
**If the owner still wants literal native-terminal scrollback after this
evidence**, that is a distinct architecture wave (P5: ratatui 0.30's
no-alt-screen init + inline/fixed viewport + line emission) — say so and
it gets scoped; jcode parity does not require it.

### 4. "Thinking toggle, thinking on by default, like jcode"

jcode's exact surface, from source: `display.show_thinking = true` is the
**default** (`jcode-base/src/config/default_file.rs:175`), `/thinking-display
<off|full|current>` toggles rendering (`app/commands.rs:3358-3387`), and
`/effort` controls how hard the model thinks (reasoning-effort request
param). operant's chain is complete end-to-end (parse `reasoning_content`
`clients/openai.rs:99` → `AgentEvent::Thinking/Reasoning` →
`DisplayMessage::reasoning`; 3 corpus thinking-block scenarios green) — the
gaps are the display knob+toggle and the request flags:

- Display: add `display.show_thinking` (default **true**) to the shim +
  `/thinking-display` command (off|full|current; "current" = live
  streaming trace only, "full" = committed blocks — mirror jcode).
- Request: per-provider thinking flags (`thinking.type`/`reasoning_effort`
  dialect table). **Provider fact (4 raw SSE probes)**: the configured kilo
  gateway emits NO `reasoning_content` for `small-stack`,
  `deepseek/deepseek-v4.1-flash`, `z-ai/glm-5.3-flash` — with
  `thinking:{type:enabled}`, with `reasoning_effort:"medium"`, and bare.
  jcode on this same provider would also show no thinking. The toggle and
  default land regardless; visible thinking needs a reasoning-forwarding
  endpoint (verify live against one before closing the wave), and the doctor
  check should say "provider emits no reasoning channel" plainly.

## Carried unchanged from v1

- **P4-2** notification purge (modal + per-response banner + stacked error
  surfaces → jcode status-notice + inline cards; call-site inventory in v1).
- **P4-5** selection pane-scoping (paint spans chrome; 442 chars/17 lines).
- **P4-6** residual split-timing audit.

## Order

P4-1 (scroll seam) → P4-2.5 (retry dedup — smallest, user-facing daily) →
P4-3 (todo store) → P4-2 (notification purge) → P4-4 (thinking) → P4-5 →
P4-6. Each = one iteration + corpus pins + deploy + live tmux re-verify.
