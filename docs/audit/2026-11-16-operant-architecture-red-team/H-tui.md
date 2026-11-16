# Lane H — the TUI (191k LOC)

**Lane:** H (red-team + ponytail audit)
**Seam:** the TUI (`crates/operant-cli/src/tui/`, 191,882 LOC across 60 top-level entries)
**Pinned revision:** `25b86ae5` (`feat(iter-2472): retract the /bar status-line half-port, theme the input area itself`) — confirmed with `git rev-parse --short HEAD`
**Mode:** READ-ONLY (rg/cat/read only; no cargo, no TUI launch)

## Subtree LOC census (measured at 25b86ae5)

| subtree | files | LOC |
|---|---|---|
| `app` | 33 | 41,157 |
| `operant_ui` | 39 | 38,140 |
| `operant_app` | 79 | 25,207 |
| `prompt_input` | 20 | 10,191 |
| `session_hub` | 4 | 9,342 |
| `operant_markdown` | 18 | 8,713 |
| `adapter_types` | 18 | 4,264 |
| `operant_render_core` | 8 | 3,847 |
| `operant_model` | 11 | 3,390 |
| `keybindings` (+`keybindings.rs`) | 5 | 2,976+ |
| `settings_screen` | 5 | 2,699 |
| `overlays` | 6 | 2,656 |
| `operant_mermaid` | 9 | 2,549 |
| `vendor/style` | 7 | 2,455 |
| `render` | 5 | 2,324 |
| `diff_viewer` | 4 | 1,490 |
| `operant_render` | 5 | 1,329 |

## Files read (exact)
- (in progress)

## Subtrees NOT read
- (in progress)

## Concept counts
- (in progress)

## THE DOCTRINE TABLE
| claim | verified / refuted | proof file:line |
|---|---|---|
| ONE renderer: a row renders in exactly one section | (pending) | |
| Live partials never enter settled rows | (pending) | |
| Reasoning display is a three-state contract behind a process-global cell | (pending) | |
| Registered slash commands can never fall through to the model | (pending) | |
| `terminal_scroll_mode`: scrollback emitter append-only, Ctrl+L never resets it | (pending) | |

## Findings

### H1 — `/model roles` opens a surface that is never painted [VERIFIED] — HIGH

`models_panel/` (391 LOC) has state, a key handler and an opener, but **no renderer**:
`rg "models_panel" src/tui/render/ src/tui/operant_ui/` exits 1 (no hit), and the only
non-test references in the whole crate are `app/init.rs:246`, `app/mod.rs:696`,
`app/commands.rs:773`, `app/key_handling.rs:1247/3401+`, `mod.rs:60`.

The failing case: `/model roles` → `app/commands.rs:772-775` `self.models_panel.open(Roles)`
→ `models_panel.visible = true` → the frame paints the unchanged chat, while
`key_handling.rs:1247-1250` now routes EVERY key into `handle_models_panel_key` and
returns `false`. Arrow keys move an invisible cursor; `Enter` writes a status notice;
`x` on an assigned slot calls `persist_config_key(key_path, "")` and **writes the config
from an invisible panel**. Escape is the only way back.

Worse than a dead panel: `models_panel.visible` is NOT in `overlay_flags()`
(`app/commands.rs:93-133`, array size 29), so `any_modal_open()`/`any_overlay_open()`
report false while the panel owns the keyboard — the input gate is blind to it.
`models_panel.visible` appears exactly once outside its own module (key_handling.rs:1247),
so no paint gate consults it either.

The render half was announced and never landed: `models_panel/mod.rs:75` and `:88` both
carry `#[cfg_attr(not(test), allow(dead_code, reason = "the render half is the next unit"))]`
on `marker()`/`note()`, and the doc comment at `:138` still says "rendered full-frame while
`visible`". Commit `d54c2f23` (iter-2434) landed the panel; iter-2444 swept it in
unmodified.

Contrast: the sibling `subagent_navigator` / `agent_picker` DO have a paint path —
they are painted from inside `paint_command_palette` (`render/operant_overlays.rs:538-547`).

Fix direction: either add the OVERLAYS row + a `paint_models_panel`, or delete the
panel and repoint `/model roles` at the inline picker's Roles scope. Do NOT add the
visible flag to `overlay_flags()` alone — that hides the defect deeper.

### H2 — `[tui] theme` in the TOML config is write-only [VERIFIED] — MED

Two theme stores exist and only one is read at boot. `app/init.rs:82` seeds the
palette from `settings.theme` (`settings.json`, `adapter_types/config.rs:7-14`), while
`config.tui.theme` (`operant.toml`) is only ever WRITTEN (`app/providers.rs:594`) and
printed (`cmd_setup.rs:122`). `rg "config.tui.theme" src/` outside those two writes has
zero readers. Failing case: set `theme = "nord"` in `operant.toml` with no
`~/.operant/settings.json` → the session boots on `default`, and nothing says why. The
module doc at `theme_colors.rs:9-10` states the opposite ("set once at startup from
`config.tui.theme`") — the comment is the defect, not the code.

### H3 — the reasoning mode has THREE copies that can disagree [VERIFIED] — MED

- `config_shim::REASONING_DISPLAY_OVERRIDE` (`operant_app/config_shim.rs:403`) — the cell
  every renderer reads (`ui_prepare.rs:2667`, `adapter.rs:140`, `messaging.rs:26`).
- `App.thinking_display_on: bool` (`app/mod.rs:446`) — read by `tui_state_impl.rs:650`
  (the "thinking off" status marker) and by the debug snapshot (`commands.rs:161`).
- `App.remembered_reasoning_mode` (`app/mod.rs:453`).

`thinking_display_on` is seeded from `config().display.show_thinking` — the **trait method**
`TuiState::show_thinking` (`operant_app/tui_state.rs:393`), which is NOT
`DisplayConfig::show_thinking`, the legacy bool inside `reasoning_display()`
(`config_shim.rs:314`). So at boot the bool is `true` regardless of `agent.show_reasoning = false`,
while the cell says `Off`. The status row then reports a mode the renderers do not use. The
two writers (`scroll_anchor.rs:77`, `commands.rs:2061`) keep them in step after the first
toggle, so the split-brain window is startup → first `/thinking-display`. Fix: seed
`thinking_display_on` from `config().display.reasoning_display() != Off`, or delete the bool
and derive it.

### H4 — a test writes the process-global reasoning cell without the guard [VERIFIED] — MED

`app/tests.rs:10071` (`a_clicked_file_edit_row_untruncates_its_diff`) takes
`reasoning_override_test_guard()` and then never writes the cell — it uses the guard to
read `DiffDisplayMode`, which the guard does not protect. The guard is a `MutexGuard` held
for the test's whole body, so any OTHER test that legitimately holds it (e.g.
`thinking_off_hides_live_streaming_thinking`, `:4399`) blocks until this test ends. That is
not a correctness race — it is lock-order coupling: a diff-rendering test now serialises
against every reasoning test and can be failed by a leaked cell value. Every direct writer
found (`app/tests.rs:1465/1487/1513/4418/4425/6858/6871/6903/6912/6946/6955/6962/10000/10047/10069`,
`operant_model/adapter.rs:688/720/734/750/763/801/815`, `gateway_commands.rs:3530`,
`main.rs:3713`, `scroll_anchor.rs:76`, `commands.rs:2064`) IS preceded by the guard, with
that one exception being guard-only.

### H5 — two mouse hit-test maps are NEVER written: right-click and thinking-collapse are dead [VERIFIED] — HIGH

`App.thinking_row_map: RefCell<HashMap<u16,u64>>` (`app/mod.rs:995`) and
`App.message_row_map: RefCell<HashMap<u16,usize>>` (`app/mod.rs:997`) are initialised empty
(`app/init.rs:415-416`) and **nothing ever inserts into them** — `grep -rn "thinking_row_map\|message_row_map" src/ | grep -i "insert\|borrow_mut"` prints nothing outside
`app/tests.rs:4650`. Their only readers are:

- `mouse.rs:119` `message_index_at_row` → `:533` (`open_context_menu_at_cursor`, Ctrl+Shift+M)
  and `:1237` (right-click → `ContextMenuKind::Message`). Both take the `else` branch every
  time: right-click over the transcript ALWAYS calls `dismiss_context_menu()` (mouse.rs:1244),
  and Ctrl+Shift+M falls back to a selection-only menu.
- `mouse.rs:1331` — the click-to-collapse-thinking handler, which is unreachable because
  `thinking_row_map` is always empty. The only writer of `thinking_expanded` is that same
  dead branch (`mouse.rs:1335`), so `thinking_expanded` can never be non-empty either.

The live paths use different mechanisms: the pinned-todo band publishes a rect through
`pinned_todo_more_area()` (`ui_viewport.rs:1688`, wired at `mouse.rs:1343`) and tool rows
publish through `TOOL_ROW_REGIONS` (`ui_viewport.rs:1697`) — both have a frame-time writer.
The comment at `ui_viewport.rs:1686` still cites `thinking_row_map` as the working precedent.

Note the map pair is `RefCell<HashMap>` with no borrow held across a call, so there is no
borrow-panic risk — the finding is dead state, not a crash.

### H6 — a keybinding advertised in `/help` but never wired, or a surface with no opener

(pending)

## Ponytail cut list

## Rust discipline table

## Log
- 03:19Z pinned rev confirmed, skeleton written