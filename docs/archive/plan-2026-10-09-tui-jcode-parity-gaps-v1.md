# TUI jcode-parity gap outline v1 — 2026-10-09 second live audit

Binary audited: `operant 0.2.1` iter-731 deploy, md5 `0f829488f287b9aeebd3ac8e219a906f`
on `/usr/local/bin/operant` + `~/.local/bin/operant` (confirmed current — every
complaint below reproduces on it). Method: first-hand tmux feedback loop
(`operant chat` live sessions, synthetic keys, pane diffing), headless
`operant tui debug simulate --mouse` replays, raw SSE probes against the
configured provider, and jcode source contrast
(`../parent-projects/jcode`, launcher `src/cli/terminal.rs`).

Supersedes: the P2-8 item of `plan-2026-10-09-tui-live-audit-bugfix-plans.md`
(its premise is refuted below). All P0/P1/P3 items from that plan are landed
(iters 725–731, peer's 728/729 for max_tool_result_chars).

## Verdict table

| # | Complaint (user) | First-hand finding | Root cause | Wave |
|---|---|---|---|---|
| 1 | Legacy notification overlays still show; want jcode's mechanism, purge legacy | Full-screen `⚠ Error` modal (44 border rows) captured on a provider stream-death; "Response complete · /copy to copy · Ctrl+J for line break" banner fires after EVERY response (captured ~12s post-reply); same error ALSO duplicated into a right-margin info-widget box; PgUp dead inside the modal | `notifications.rs` overlay-banner system + `render_error_modal` (`operant_overlays.rs:665`) — ~15 `push_notification` call sites (`agent_events.rs:324,344,384`, `key_handling.rs:255…`, `messaging.rs:109…`) | P4-2 |
| 2 | Scroll behaves like one pinned page; PgUp/PgDn not working | 2 live sessions: PageUp ×8, Up ×3, Alt+Up ×2, Ctrl+Up ×3 — zero transcript movement (only the tok/s widget ticked), with content proven above the fold (session 1: 20+ tool rows + 13-item report; word-number overflow in audit2). F12 debug overlay toggles fine (38-line diff) → keys reach `handle_key_event`; the state machine is suite-pinned (`tests.rs:2103` PageUp→offset 10) | Render-side scroll resolution ignores user scroll live: anchor-precedence chain `ui_viewport.rs:405-416` (`anchored_scroll`/`resize_anchor_scroll` override `user_scroll` every frame) or `max_scroll`/`total_wrapped_lines` sizing; zero corpus scenarios exercise scroll keys | P4-1 |
| 3 | Tool calls sometimes split the stream before text renders | Session 1 streamed 20+ tool rows mid-reply with no mid-word seam (iter-725 holding); complaint is timing-dependent ("sometimes") | Residual timing path unproven; `tool-split-stream` scenario pins the deterministic case | P4-6 |
| 4 | Selection selects UI elements, not just text | Headless drag `<left,2,2>…<release,95,27>`: "Chat selection · 442 chars · 17 lines" — swallowed the info-widget box, the static left rail (model/provider/cwd), and blank spacers for a ~5-line conversation | Paint region spans the whole prepared frame; iter-728 fixed the COPY text (reflow), not the selection REGION | P4-5 |
| 5 | No todo list or thinking visible in the TUI | Todo: tool + events exist (`todo_tool.rs` → `AgentEvent::TodoUpdated`), renderers exist (`info_widget_todos.rs` + tests), `/todos` is registered — but `pinned_todos_payload()→None` (`tui_state_impl.rs:168`), `InfoWidgetData.todos: Vec::new()` (`:686`), `TodoUpdated` reduced to a one-liner (`agent_events.rs:520`), `/todos` has no handler (falls through as literal text). Thinking: chain complete end-to-end (openai.rs:99 parses `reasoning_content` → `AgentEvent::Thinking/Reasoning` → `DisplayMessage::reasoning`; 3 corpus thinking-block scenarios pass); live absence is provider-side — raw SSE probes show kilo `small-stack`, `deepseek/deepseek-v4.1-flash`, and `z-ai/glm-5.3-flash` (+`thinking.type=enabled`) all emit NO `reasoning_content` | Todo: deliberately-stubbed data path (the "[port-decision] wire when a todo store lands" notes). Thinking: configured provider strips/never emits reasoning; TUI needs no render change — needs request-flag wiring + a reasoning-forwarding provider to verify live | P4-3, P4-4 |

## Correction that changes the plan: jcode IS an alt-screen app

The earlier plan's P2-8 ("port jcode's no-alt-screen model — native terminal
scrollback") rests on a false verification: grep for `EnterAlternateScreen`
over `jcode-tui/src/tui/` finds none because jcode's launcher calls
`ratatui::init()` (`src/cli/terminal.rs:353`), which enters the alternate screen
inside the library. jcode scrolls exactly like operant intends to: an internal
viewport in the alt screen, driven by a full key ladder (Up/Down ±1 with
history-nav guard, Ctrl+Up/Down ±3, PgUp/PgDn ±10, Alt+Up/Down ±20, prompt
jumps, bookmarks, tail-catchup animation, resize/history anchors) plus an
optional handterm host scrollbar bridge. There is no native-scrollback port to
do. The gap is that operant's ladder is dead at the render seam — not the
architecture.

## Waves

### P4-1 — Fix the dead scroll ladder (highest priority; basic UX broken)

1. Add corpus scenario `scroll-keys`: ≥200 wrapped-line agent script +
   `--keys '<pageup>'` (runner already supports arbitrary keys), assert
   top-of-viewport content changes; variants: `PageUp`, `Ctrl+Up`, `Alt+Up`,
   back-to-bottom re-follow. Red today if the render path is broken headless
   too; if green, the failure is live-only and instrumentation moves to the
   `TuiState` seam (see 3).
2. Root-cause between the two named suspects, in order:
   a. `ui_viewport.rs:405-416` — `anchored_scroll` and `resize_anchor_scroll`
      take precedence over `user_scroll` every frame; verify no stale
      `pending_history_anchor`/`pending_resize_anchor` persists in a plain
      live session and forces tail.
   b. `compute_max_scroll_with_prompt_preview` (`ui_viewport.rs:1615`) +
      `prepared.total_wrapped_lines()` — confirm live `max_scroll > 0` when
      content overflows (expose via the F12 debug overlay: add
      `scroll/max_scroll/total_lines` fields — it already renders
      frames/uptime, and would have made this audit one probe instead of ten).
3. Fix, re-run the live tmux ladder (all six key families move the transcript,
   auto-follow re-arms at bottom, resize while parked keeps position).
4. Ship + full corpus re-prove if any golden shifts.

### P4-2 — Notification purge: port jcode's status-notice mechanism

jcode has no overlay banner system at all. Its surfaces: (a)
`status_notice: Option<(String, Instant)>` — one fading status-line notice
(`conversation_state.rs:419`), plus a distinct-color `learn_hint`; (b) inline
transcript cards (`push_display_message(system/error)`); (c) margin
info-widgets. Plan:

1. Port `set_status_notice` (text + Instant, auto-expiry on next status line
   paint) onto operant's status row.
2. Route all ~15 `push_notification` call sites: transient Info/Success →
   status notice; Error/Warning → inline `DisplayMessage` card (system/error
   style) + status notice; persistent (context-full) → card only.
3. Replace `render_error_modal` with the inline-card path (session-1 evidence:
   a provider stream death currently steals the whole screen; jcode shows a
   card + "reply 'continue' to retry" — same guidance, no modal). Esc/PgUp
   dead-key behavior inside the modal disappears with it.
4. De-duplicate: today one agent error renders as transcript rows AND a modal
   AND a right-margin box; single source of truth per event.
5. Delete `notifications.rs` push/tick/banner render + the countdown-bar
   machinery (which exists only to serve it, incl. the
   `OPERANT_FROZEN_NOTIFICATION_CLOCK` test seam) and sweep the 40-entry
   overlay registry for entries orphaned by the cutover (iter-717 method).
6. Corpus: the 9 banner-bearing goldens reshape; remove the frozen-clock pin.

### P4-3 — Wire the todo store (the "half-assed" gap, closed fully)

Everything exists except the store and the wiring:

1. `App.todos: Vec<TodoItem>` (+ goals, per `operant_app/todo.rs` types) —
   session-local, persisted like jcode's.
2. Port `TodoUpdated` through the bus (bus.rs `[port-excision]` note names
   exactly this) → replace the `agent_events.rs:520` one-liner with the store
   update.
3. Feed `InfoWidgetData.todos` (`tui_state_impl.rs:686` — the same missing
   source the [port-decision] notes name) + `pinned_todos_payload`
   (`:168-179`); `display.pin_todos` config knob already exists in the shim.
4. `/todos` handler: live-updating inline card in the chat (jcode
   `/todos` behavior: card, again-to-dismiss, `panel` legacy screen, `pin`
   toggle); the command registration exists, the handler does not.
5. Corpus scenario: script with two `TodoUpdated` events → assert card rows +
   info-widget todo band.

### P4-4 — Make thinking actually arrive (request-flag wiring)

The TUI is done (parse → event → `DisplayMessage::reasoning` → thinking-block
scenarios pass). The provider is not: three raw SSE probes (small-stack,
deepseek-v4.1-flash, glm-5.3-flash + `thinking.type=enabled`) show kilo emits
no `reasoning_content`. Plan: per-provider thinking request flags
(`thinking.type` / `reasoning_effort` dialect table in the provider registry),
a probe-verified provider to confirm live render end-to-end, and a doctor
check that names "provider emits no reasoning" plainly. No render work.

### P4-5 — Pane-scope the selection region

Port jcode's `CopySelectionPane::{Chat, SidePane, Input}` model: a drag
starts and is clamped within one pane; the chat pane's highlightable region is
the text column only — never the left rail, info-widget margins, status rows,
or blank spacers (headless drag today: 442 chars/17 lines including all of
those). Copy text already reflows (iter-728), so this is purely a paint/hit-test
boundary change. Corpus: extend `selection-highlight` with a chrome-spanning
drag variant asserting borders/rail cells stay unpainted.

### P4-6 — Residual stream/tool-split timing audit

Reproduce with a deliberately slow render cadence (large transcript, high
frame cost) while a scripted tool call interleaves mid-stream; inspect whether
the tool-row anchor can paint before the pending-text slot renders (the
"splitting when tool calls execute before the TUI renders the messages"
report). The deterministic seam is pinned by `tool-split-stream`; this wave
either finds the residual path or closes the item with evidence.

## Not gaps (verified, leave alone)

- PgUp/PgDn key *bindings* exist and the handler is correct — the loss is
  downstream (P4-1); don't re-bind.
- `scroll_anchor.rs` (history/resize anchors, `note_user_scroll`) is ported and
  suite-pinned — do not rewrite; debug its *consumers*.
- Thinking render, todo renderers, selection copy-reflow: shipped and tested.
- The prompt-entry fade pin and settled-color goldens (iter-730) are stable
  across 4 consecutive verifies.

## Order

P4-1 → P4-2 → P4-3 → P4-4 → P4-5 → P4-6. P4-1 is alone at the top: scrolling
is the most basic contract of a chat TUI and it is verifiably dead in the
field. Each wave = one iteration + deploy + live re-verification in tmux, per
the house loop.
