---
ID: "AD-034"
Title: "Warnings triage deletes dead TUI paths, wires the two real gaps, and records the rest as expected debt"
Status: "Active"
Date: "2026-10-19"
Description: "The production build of operant-cli is warning-free: 35 dead items are deleted, error annotations keep their error role, and every kept dead-code item carries a reason or an open BUGS row"
---

# AD-034: Warnings triage deletes dead TUI paths, wires the two real gaps, and records the rest as expected debt

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The goal asked for every warning in operant-cli to be triaged. At the start of this pass, after the MCP approval dialog and the turn-diff mode had been removed, `cargo check -p operant-cli --bin operant` reported 35 warnings (the stop-hook counted 41 before those removals). The clippy allowlist `.ci/clippy-allowlist.txt` held 47 entries. The gate keys entries by file and lint, so an entry stays until its file has no warning of that lint.

## Decision

Rule: an item with no production caller is deleted when a live path already does its job, or when nothing can ever set it. An item the design still intends keeps its `expect`, and the open gap gets a BUGS row. Test-only functions use `cfg_attr(not(test), expect(dead_code, reason = ...))`. A dead type is deleted, not gated with `#[cfg(test)]`, because live impl blocks still reference it (lint-gate traps).

1. **Deleted, with their test-only callers.**
   - The turn-diff mode: `DiffType`, `diff_type`, `turn_files`, `open_turn`, `toggle_diff_type`, `set_turn_diff`, and the `d` key that toggled it. It had no producer since iter-209. `/changes` now opens the working-tree diff, the same view as `/review` and `/diff`. `/rollback` sets a status hint that names `/rewind`.
   - `App::intercept_slash_command_with_args` and `App::intercept_slash_command`. Both wrapped `intercept_slash_command_with_args_impl`, as did the live `handle_tui_command`, so the tests call `handle_tui_command(cmd, args)`.
   - `TurnState::Connecting`. Nothing constructed it.
   - Thirteen `KeyAction` variants with no reference anywhere: `MoveCursorHome`, `MoveCursorWordLeft`, `MoveCursorLineStart`, `MoveCursorLineEnd`, `DeleteCharLeft`, `DeleteCharRight`, `DeleteWordRight`, `DeleteToLineStart`, `DeleteToLineEnd`, `VimChangeWord`, `VimPaste`, `SubmitAlt`, `TogglePlanMode`.
   - `StatsDialogState::add_model_usage`, a second accumulator beside the production `build_model_breakdown`, with its three tests.
   - `image_paste::encode_image_base64` with its two tests.
   - `KeyBindingRegistry::find_with_fallback` and `remove`, with their tests.
   - `McpViewStatus::Error`. The core MCP manager records no connection failure, so nothing could set it. A configured server that is not connected already maps to `Disconnected`. See RT-32.
   - The `fn key` helper in `dialogs/tests.rs`, which only the removed tests used.

2. **Wired.**
   - `SystemAnnotation.style` now selects the row: `SystemMessageStyle::Error` renders through `DisplayMessage::error`, and every other style renders as a system row. Before, a notice from `push_notice(NotificationKind::Error)` drew the same as a status line. Test `error_annotation_keeps_the_error_role`. Mutation: removing the `Error` arm fails the test, and the file was restored.
   - `SessionBrowserState::selected_session` replaces two inline `sessions.get(selected_idx)` lookups, so the accessor is live and not duplicated.

3. **Kept, each with an `expect` that cites this record.**
   - Wire types in `adapter_types/types.rs`: the `ContentBlock` variants, `signature`, `retry_secs`, and the `ToolResultContent` variants are parsed from adapter JSON and are not built in this process. `ToolResultContent` stays a plain `expect`, because `Image` and `Blocks` are never built even in tests. The other wire items are `cfg_attr(not(test))`, because tests read them and a plain `expect` would go unfulfilled there.
   - `KeyAction::Redo`. An earlier iteration kept it on purpose so a future redo has a variant to bind. The catalogue guard test `test_registry_advertises_no_binding_for_an_unimplemented_action` pins that no binding uses it.
   - The space token scale `space::XXS`, `S`, and `XL`. The layout test pins the progression `XS * 2 == S` and `L * 2 == XL`, so removing them would break the scale the layout test checks.
   - Test-only helpers: `App::add_message`, `Message::user`, `Message::content_blocks`, `text_selection_bg`, `Sink::as_str`, the `Entry` fields `name` and `sink`, `KeyBindingRegistry::get_bindings`, `VimMode::label`, `rank_commands`, and the event bus `len` and `is_empty`. Clippy's `len_without_is_empty` requires that pair to stay together.
   - `App.bridge_state_tx`. The gateway runs as a separate process, so nothing sends through this sender. See RT-31.
   - The vendored style modules `vendor/style/palette.rs`, `theme.rs`, and `theme_mode.rs` carry `#![cfg_attr(not(test), expect(dead_code, ...))]`, because their unused entry points are kept as a unit. The light-theme functions in `theme_mode.rs` have no caller. See RT-33.

4. **Open gaps, tracked in BUGS.md as RT-29 to RT-33.** Each was found while the warnings were triaged, and each needs an owner or a design decision: the vim mode is never drawn (RT-29), slash-command usage is recorded but never ranks (RT-30), the bridge-state channel has no producer (RT-31), MCP connection failures are not recorded (RT-32), and light-theme colour adaptation is unwired (RT-33).

## Evidence

- `cargo check -p operant-cli --bin operant`: 0 warnings, from 35.
- `bash scripts/clippy-warning-gate.sh`: `seen=24 allowlist=24 new=0 stale=0`. The allowlist shrank from 47 entries. The entries are pruned with `--update` once the gate reports no new key.
- `cargo test -p operant-cli --features plugins-wasm --bin operant`: 1602 passed, 0 failed, 7 ignored. The count dropped from 1618 because the removed dead paths took their tests with them.
- Mutation: removing the `SystemMessageStyle::Error` arm fails `error_annotation_keeps_the_error_role`, and the file was restored.

## Consequences

- `/changes` opens the working-tree diff. The diff viewer's `d` key no longer switches modes.
- `/rollback` no longer opens a diff. It names `/rewind` in the status line.
- Error notices render as error rows in the transcript.
- The remaining 24 allowlist keys are all test-configuration dead code in the `operant_app/`, `operant_ui/`, `operant_markdown/`, `operant_render/`, `operant_model/`, and `adapter_types/` trees. They produce no warning in the production build. The owner decisions recorded for the iter-910 dead-code purge still cover them.

## Open

- RT-29 to RT-33 in BUGS.md.
- The 24 allowlist keys above, which the iter-910 owner decisions still cover.
