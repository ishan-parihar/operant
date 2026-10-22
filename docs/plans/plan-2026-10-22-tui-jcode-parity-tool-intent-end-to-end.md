# 2026-10-22 TUI jcode parity: tool-intent end-to-end, scroll seam, selection, /thinking-display, todo stages

Dated-frozen: published `2026-10-22`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Mechanism

### Root cause 1 — tool calls carry no description (the dominant gap)

The renderer is ready: `operant_ui::ui_messages::render_tool_message` reads
`tc.intent` and `tool.input["intent"]` (`ui_tools.rs:878-884`) and prints it as the
tool row's descriptive segment; `get_tool_activity_detail` (`ui_tools.rs:871-889`)
composes it for the live status line. Both read one field that NOTHING in this tree
ever fills: `ToolUseBlock` (`tui/app/enums.rs:204-217`) has no `intent` member, so
`adapter::display_message_from_tool_block` (`operant_model/adapter.rs:289-355`) sets
`intent: None`, and `tui_state_impl.rs:360` does the same for live blocks.

The upstream mechanism is a SCHEMA-level contract, not a per-tool opt-in:

- `jcode-tool-core/src/lib.rs:17-19` — `TOOL_INTENT_DESCRIPTION = "Required short
  label shown in the UI: why this call is being made."`, exposed as
  `intent_schema_property()` (:27-32).
- `jcode-tool-core/src/lib.rs:48-90` — `ensure_intent_in_schema()` injects the
  `intent` property into every tool's parameter schema and marks it required.
  Applied centrally from `Tool::to_definition()` (:183), so native tools AND MCP
  proxies get it with no per-tool wiring.
- `jcode-message-types/src/lib.rs:671-677` — `ToolCall::intent_from_input()` reads
  it back off the parsed arguments; `refresh_intent_from_input()` (:679) re-derives
  it whenever arguments are updated.
- `jcode-tui/src/tui/app/turn.rs:1080`, `state_ui_storage.rs:513`,
  `remote/server_events.rs:770` — every construction site fills it from input.
- `jcode-base/src/mcp/tool.rs:53-70` — the MCP proxy STRIPS `intent` before
  forwarding unless the server's own schema declares one, and never forwards
  `accept_large_output`.
- Display: `ui_tools.rs:864-889` `get_tool_summary`/`get_tool_activity_detail`
  (intent · technical summary), `ui_messages.rs` tool row, `ui_input.rs:887` live
  activity line, `ui_todo_stages.rs` swarm lane notes — all read the one field.

**Fix**: port the same three layers into operant:
1. `operant-core::schema::ensure_intent_in_schema()` + the two description consts,
   applied in `ToolRegistry::get_schemas*` (the single choke point before any
   request serialises — `agent/run.rs:115` `get_schemas_for_request`,
   `agent/run.rs:126` `get_schemas`, `prompting.rs:435`, `tools/tool_search.rs:328`,
   `tools/tool_state.rs:176/187`, `cli/mcp_serve.rs:326`).
2. Fill `intent` at every tool-call construction: `ToolUseBlock.intent` onto the
   block; `display_message_from_tool_block` sets `ToolCall.intent`;
   `streaming_tool_calls()` derives it from the parsed input; `ContentBlock::ToolUse`
   path (`adapter.rs:423-430`) derives it via `ToolCall::intent_from_input`.
3. Gateway: `gateway_runner.rs:2964 tool_preview_line` formats only
   `emoji name: arg`; extend it to the `intent · summary` shape so the channels
   show what each call is doing (the user's explicit ask: "the TUI and gateway
   channels can display what each tool call is doing").
4. Strip `intent` in the MCP proxy before forwarding (jcode parity, and an
   undeclared parameter can be rejected by a strict server).

### Root cause 2 — scrolling

The seam is documented as `P4-1` in `docs/plans/plan-2026-10-09-tui-jcode-parity-gaps-v2.md`
("8× PageUp / Up / Alt+Up / Ctrl+Up = zero transcript movement with proven
overflow ... root-cause the render seam (anchor-precedence `ui_viewport.rs:405-416`
vs `max_scroll` sizing), fix"). The precedence chain in
`ui_viewport.rs:405-416` is: resize anchor → history anchor → `auto_scroll_paused()`
→ tail follow. Reading it against the writers:

- `scroll_offset` is a *distance from the bottom*; the render path inverts it
  (`scroll = max_scroll - scroll_offset`, `scroll_anchor.rs:271-279`).
- `App::note_user_scroll` (`scroll_anchor.rs:161-176`) re-pins the reflow anchor
  from `last_render_scroll_offset`, and `reconcile_scroll_anchor`
  (`scroll_anchor.rs:220-316`) VOIDS the pin whenever `scrolled !=
  anchor.applied_offset` — a comparison that fires on the very first frame after
  any write, because nothing else re-writes `applied_offset`.
- `auto_scroll` is a bool, and every scroll-up writer sets it false then calls
  `note_user_scroll`, which returns immediately when `auto_scroll` is true
  (`:162-165`) — so a reader who scrolled up while auto-following owns no anchor.
- `ui_viewport.rs:412-413` then clamps `user_scroll.min(max_scroll)` but the
  `auto_scroll` writer at `key_handling.rs` runs BEFORE the frame that publishes
  the new `max_scroll`, so the first paint after a key still clamps against the
  previous frame's max.

Diagnose per defect with a harness before editing (see Gates): the ladder keys, the
wheel, the anchor reconcile and the ghost-offset clamp each need a red test.

### Root cause 3 — tool-call text selection

`render/selection.rs:59-67` `UI_ROLES` includes `"tool"`, so
`selection_line_selectable` / `selection_copy_text` treat a tool row as chrome and
DROP it unless the selection lies entirely inside tool rows. jcode's contract
(`ui/copy_selection.rs::copy_selection_text_from_raw_lines_base`) does not have a
tool-role exclusion: it drops only rows with no line map (header, padding) and
applies chrome-free substitutions (code gutter, blockquote gutter, table rules).
operant's expanded form is deliberate for PASTED history but wrong for the
in-lane case the user reports: dragging across a tool call copies nothing.

Fix: restore selectability for tool rows that carry a line map, keeping the
existing "all-UI selection copies what it shows" escape, and assert both halves.

### Root cause 4 — /thinking-display

Two defects, both found by reading:
1. The command name is `thinking-display`, but the intercept arm
   (`app/commands.rs:1413-1461`) is reached only via the `cmd` token, and the
   registry publishes `/thinking-display` as public with hidden aliases
   `/thinking` and `/reasoning`. `/reasoning` matches an EARLIER arm
   (`commands.rs:1116-1129`) that toggles an unrelated `show_reasoning` boolean
   with no render reader — so the advertised alias silently does the wrong thing.
2. The `""` arm (`commands.rs:1438`) is a no-op: bare `/thinking-display` reports
   nothing. jcode's equivalent (`app/commands.rs:3588-3647`) prints the current
   mode plus the three-mode legend on an empty argument, and rejects an unknown
   mode with a usage message rather than a silent pass.

Fix: empty args print the current mode + legend; `/reasoning` routes to the
thinking-display handler (jcode treats all three spellings as one command);
`show_reasoning`/`thinking_display_on` collapse onto the single config-shim mode
so the two flags cannot disagree.

### Root cause 5 — todo visual parity

operant's card (`ui_messages.rs:1110-1200`) groups by `group` and renders a flat
checklist per group. jcode additionally has a STAGE/LANE grid
(`ui_todo_stages.rs:295-330` `push_stage_pips`, `:690-780` `push_stage_lines`,
`jcode-todo-policy/src/stages.rs:77-116` `stage_layout`): `phase 1 ●●●  phase 2 ●●`
headers, one row per lane (`●●○  parser 🦊  Extract lexer, Update callers, Add tests`),
later phases dimmed, lane notes for a live swarm worker. The data model lacks the
field entirely: operant's `TodoItem` (`operant_app/todo.rs:273-309`) has no `stage`
while upstream has it with `alias = "wave"` / `alias = "phase"`
(`jcode-task-types/src/lib.rs:460-474`). Without the field the model cannot even
express stages, so this is a data-model port then a renderer port.

Fix: add `stage: Option<u32>` (+ `details`, jcode parity) with the aliases and the
lenient `deserialize_optional_stage`; port `stage_layout`/`lane_group`/
`has_stages`/`lane_id`; render the stage grid when any todo declares a stage and
fall back to the existing flat card otherwise (jcode's `has_stages` rule).

## Gates / success criteria

(fixed before the run)

## Cost

Each iteration is one crate-scoped build + its tests + a release deploy. The
tool-intent iteration touches `operant-core` (schema) + `operant-cli` (adapter,
ui, gateway) + `mcp_tool.rs` (strip), so it is the largest of the five.
