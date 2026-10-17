# 05 — `/model` that switches the agent, as jcode's inline picker; retire the legacy overlays

**Size:** L. **Findings:** 4, 13 in `01`.
Part A (correctness) is small and comes first. Part B (the jcode look) builds on it.

**Boundary (owner instruction):** port jcode's **visual layer** only. Keep
operant's agent core. Do not import jcode's provider, auth or route crates.
The data a picker shows comes from operant (`config.client`,
`ProviderRegistry`, `agent.model()`).

## What the user sees today (simulator, example config, 110×35)

```
┌───────────────────────────────────────────────┐
│ Select model                                  │
│ Search                                        │
│   gpt-4.1  OpenAI                             │
│   gpt-4o  OpenAI                              │   ← 3 identical rows
│   gpt-4o  OpenAI                              │
│   gpt-4o  OpenAI                              │
│   gpt-4o-mini  OpenAI                         │   ← 4 identical rows
│   …                                           │
│ enter select                     Esc to close │
└───────────────────────────────────────────────┘
```

It's a centred legacy modal (`tui/model_picker/`). The rows come from the
models.dev snapshot for **one** provider, guessed from the model *name*. The
agent's real endpoint (`config.client.base_url`, here `http://localhost:20129/v1`,
the omp gateway with routes such as `small-stack`) is never queried.

## Part A — correctness (do first)

### A1. Root causes

| # | Where | What |
|---|---|---|
| 1 | `tui/app/providers.rs:255-267` `App::set_model` | changes `model_name`, `config.agent.model`, cost tracker; **never reaches `agent.set_model`** (`operant-core/src/agent/events.rs:491`). Requests keep using `agent.model()` (`events.rs:606`). |
| 2 | `tui/app/init.rs:73` | `active_provider = infer_provider_from_model(&config.agent.model)`, a name heuristic (`tui/provider.rs:3`). It ignores `config.client.base_url`. |
| 3 | `tui/app/providers.rs:10-100` | catalogue = `models_for_provider_from_registry(provider, models.dev)`. The live fetch uses `auth_store.api_key_for(provider)` + `settings.providers[provider].api_base` or `PROVIDERS[...].default_base_url`, which is **not** the agent's client. |
| 4 | `tui/model_picker/render.rs:136` | renders `display_name`, so dated snapshot ids collapse to identical rows. |
| 5 | `tui/app/key_handling.rs:678-718` | confirm builds `"{provider}/{id}"` from the guessed provider. With a custom endpoint this produces ids the endpoint doesn't know. |
| 6 | `providers.rs:222-252` Alt+P `cycle_configured_model` | same bug as 1. |
| 7 | `tui/app/key_handling.rs:691-694` | the picker's effort column → `pending_effort`. This one *does* reach the agent (plan 03). Keep it. |

### A2. Fix

**Catalogue source (one function, `fn configured_model_catalog(config) -> Vec<CatalogEntry>`):**
1. `GET {config.client.base_url}/models` with the client's key, the same
   `ClientConfig::from(&config.client)` the agent uses (`main.rs:946`). Parse
   OpenAI-style `{"data":[{"id":…}]}`. Cache in
   `~/.cache/operant/models-<hash(base_url)>.json` with a timestamp; a stale
   cache shows immediately and refreshes in the background (the existing
   `model_fetch_rx` channel pattern, `providers.rs:53-78`).
2. Plus `[agent] model` and `[agent] fallback_models` (always shown, marked
   "configured", even when `/models` is unreachable).
3. Plus every `[providers]` entry that `ProviderRegistry` holds a client for.
   **Verify first** that the agent can switch to a registry client by name
   (`crates/operant-core/src/agent/provider_registry.rs`). If it can't, leave
   this group out and file it in `BUGS.md`. Don't add a switching layer here.
4. models.dev is used only to *decorate* rows (context window, price) when the
   id matches. It is never a row source.

Rows show the **id** (`small-stack`, `gpt-4o-2024-08-06`), with the
models.dev display name as secondary text. Dedupe by id.

**Apply:** confirm sets `app.pending_model = Some(id)`.
`run_pending_agent_actions` (`tui_app.rs:875`) calls `agent.set_model(id)`, then
`app.model_name = agent.model()` (display from the source of truth),
`refresh_context_window_size()`, and persists via `09`'s surgical writer
(`agent.model` only). Alt+P goes through the same `pending_model`.

**Current marker:** from `agent.model()` (mirror it into `App` once per
pending-action drain), never from `App.model_name` alone.

**Delete:** `infer_provider_from_model` (both copies, `tui/provider.rs:3` and
`cmd_tui_debug.rs:780`) once nothing reads `active_provider` for routing. The
status line can show the host of `client.base_url` instead. Also delete
`models_for_provider_from_registry`'s `free` / `codex` special cases if `05` B
removes those providers (see B4).

### A3. Tests first

- `App` + mock agent: open `/model`, select row 2, Enter → assert
  `agent.model() == row2.id`. RED today.
- The catalogue function with a stub HTTP server returning
  `{"data":[{"id":"small-stack"},{"id":"gpt-4o-2024-08-06"},{"id":"gpt-4o-2024-05-13"}]}`
  → 3 distinct rows, no duplicates. RED today (the picker never queries it).
- Alt+P → `agent.model()` changes. RED today.

### A4. Acceptance

Live, in tmux: `/model` lists the omp gateway's routes. Pick one, ask "which
model are you?" or check the gateway log → the request used the picked id with
no restart.

## Part B — jcode's inline picker replaces the legacy modals

### B1. What is already ported (don't re-port)

- The renderer: `tui/operant_ui/ui_inline_interactive.rs` (1,499 lines),
  reached from `operant_ui/ui_inline.rs:38` and `ui_input.rs:220-227`.
- The types: `tui/operant_app/tui_fns.rs:278` `PickerKind`, `:478` `PickerAction`,
  `:508` `InlineInteractiveState`, `:593` `PickerEntry`, `:638` `PickerOption`.
- The seam: `TuiState::inline_interactive_state()` (`operant_app/tui_state.rs:517`).
  **It returns `None`** (`app/tui_state_impl.rs:970-978`), so everything above
  is dead.
- The session picker: `tui/operant_ui/session_picker.rs` + `session_picker/`
  (with loading) + tests, about 9.8k lines. **Nothing opens it.** `/resume` opens
  the legacy `session_browser` (`commands.rs:481-485`). See also
  `operant_ui/mod.rs:3387-3395` (the session, login and account picker
  overlays are "excised").

### B2. What is missing (the actual port)

jcode reference (read, don't import):
`../parent-projects/jcode/crates/jcode-tui/src/tui/app/inline_interactive.rs`.
It has `open_model_picker` :1150 → `open_model_picker_inner` :1191 →
`open_model_picker_with_routes` :1544, the key handler
`handle_inline_interactive_key` :3341, and the model apply branch
`PickerAction::Model` :3759. The `inline_interactive/openers.rs` file has the
agents, login and logout openers.

1. `App` gets one field: `inline_interactive: Option<InlineInteractiveState>`.
   `inline_interactive_state()` returns `self.inline_interactive.as_ref()`.
2. `open_model_picker_inline()`: build `PickerEntry`s from Part A's catalogue
   (`name` = id, `options` = one `PickerOption { provider: <endpoint host>,
   api_method: "openai-compatible", available, detail }`, `is_current` from
   `agent.model()`, `effort` from the plan-03 effort list). Fill `filtered` with
   all indices.
3. Key handling: port `handle_inline_interactive_key`'s navigation, filter,
   column, Enter and Esc. Leave out the remote, SSH, favourites and
   `RemoteLogin` branches (operant has no remote mode). Enter on
   `PickerAction::Model` → Part A's `pending_model` (+ `pending_effort` when the
   effort column changed).
4. `/model`, `/models` and Ctrl+A open the inline picker. Delete
   `tui/model_picker/` (5 files, 981 lines) and `App.model_picker*` fields in
   the same commit. `overlay_flags` (`commands.rs:38-84`) loses
   `model_picker` and gains `inline_interactive`.
5. `/effort`: the same inline picker with `PickerKind::Model`, effort column
   only, or keep it as one row type. Delete `tui/effort_picker.rs` and its
   overlay.
6. `/resume`, `/sessions`, `/session`: open the ported `session_picker`. Data
   comes from `agent.database()` sessions (the legacy browser's loader:
   `session_list_pending`, `channel_drains.rs`). Enter → `agent.set_session_id(id)`
   + display rebuild (the `03` pattern). Delete `tui/session_browser.rs` and its
   key arms (`key_handling.rs` ~771+) in the same commit. Fold `/rename` into
   the picker's rename action.
7. `/login`, `/logout`: the inline picker over operant's own auth store
   (`auth_store`), with `PickerKind::Login`. The ported `PickerAction` has
   `Login`/`Logout` **excised** (`tui_fns.rs:481,492`). Add back
   `Login(String)` / `Logout(String)` holding an operant provider id. That is
   the only type change.

### B3. Legacy overlay retirement map

Painted by `render/operant_overlays.rs:480-650` on top of the jcode chrome:

| Legacy overlay | jcode replacement | Action |
|---|---|---|
| `model_picker` | inline picker (Model) | B2.4 delete |
| `effort_picker` | inline picker (effort column) | B2.5 delete |
| `session_browser` | ported `session_picker` | B2.6 delete |
| `session_branching` | session picker with a fork-tree filter, or delete (`04` `/branch`) | decide with `04` |
| `connect_dialog`, `key_input_dialog`, `custom_provider_dialog`, `free_mode_dialog`, `device_auth_dialog` | inline Login picker + one key-input line | B2.7, then delete the four dialogs |
| `permissions_picker` | inline picker (one-column) | optional, same pattern |
| `command_palette` (`dialog_select`) | jcode has none; the `/` typeahead is the palette | delete after `04` makes typeahead complete |
| `help_overlay`, `settings_screen`, `theme_screen`, `stats_dialog`, `mcp_view`, `skills_view`, `plugins_hub`, `journey_view`, `hooks_config_menu`, `diff_viewer`, `export_dialog`, `context_viz`, `rewind_selector`, `ask_user_dialog`, `permission_request`, `bypass_permissions_dialog`, `mcp_approval`, `import_config_*` | no 1:1 jcode picker | **keep** (operant-only surfaces). In native-scroll mode they use the alternate screen (`07` step 5). |

### B4. Provider onboarding: ask the owner, default lean

The TUI carries an 84-row `PROVIDERS` table (`crates/operant-cli/src/provider.rs`), a
composite `free` provider (Zen → OpenRouter), Codex OAuth and Copilot device
flow (which never finishes, `tui_app.rs:309-314`). The agent uses **one**
OpenAI-compatible client. The owner's setup is the omp gateway. Recommended
default: the Login picker offers "OpenAI-compatible endpoint (base URL +
key)" plus the providers the user actually has keys for. Delete `free`,
device-flow and Codex OAuth unless the owner says they use them.
**Ask before deleting.** It's user-facing.

### B5. Tests

- Corpus: `model-picker-inline.scenario.json` with the stub catalogue, then a
  new golden. Re-prove the old `model-picker*` goldens as **deleted** and state it in
  the commit.
- `inline_interactive_state()` is `Some` after `/model`, and `None` after Esc.
- `/resume` → `session_picker` is visible and `session_browser` no longer exists
  (compile-level).
- Visual parity: render the same catalogue in jcode
  (`../parent-projects/jcode`, `cargo run -- …`, if it builds) and in operant
  at 110×35. Put both screens in the PR description.

### B6. Acceptance

- `rtk grep "model_picker::" crates/operant-cli/src` → 0 hits.
- `inline_interactive_state` no longer has a `[port-decision]` comment.
- The `[port-decision]` count (`rtk grep -c "\[port-decision\]"`) goes down. Note
  the number in the CHANGELOG (213 on 0ee25739).
