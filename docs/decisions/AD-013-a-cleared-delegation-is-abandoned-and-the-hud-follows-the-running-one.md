---
ID: "AD-013"
Title: "A cleared delegation is abandoned, and the HUD follows the running one"
Status: "Active"
Date: "2026-10-19"
Description: "A delegation whose turn is cleared by /new or /rewind is marked abandoned, and the status bar shows a running delegation before a finished one"
---

# AD-013: A cleared delegation is abandoned, and the HUD follows the running one

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

A delegation's status entry was pushed when the call started and changed only when its `ToolComplete` event arrived inside the transcript-block lookup. `/new` and `/rewind` clear `tool_use_blocks` in five places, so a delegation running at that moment never completed, and the HUD kept it "running" forever. The HUD also read the first entry in the list with `.first()`, so a stale finished row hid a later running one. The red-team row TUI-9 asked for both fixes.

## Decision

1. Completion updates the HUD by `tool_call_id` even when the transcript block is gone. The HUD update no longer sits inside the block lookup.
2. Every clear of the transcript's tool blocks goes through `App::clear_tool_blocks`, which marks any delegation still `running` or `queued` as `abandoned`. The five call sites in `adapter_types/tui_app.rs` use it.
3. The HUD shows the first running or queued delegation, and falls back to the first entry only when none is running.

## Consequences

- An abandoned delegation stays in the HUD as "abandoned" until the next turn replaces the list; it is not removed. Removing it would hide that work was lost.
- If the subagent finishes after its turn was cleared, its completion still matches the HUD row by id and sets it to `done` or `error`, so an abandoned row can be corrected by a late result. The tool result is unchanged.
- Test: `a_cleared_delegation_is_abandoned_not_left_running` (`crates/operant-cli/src/tui/app/tests.rs`). Mutation check: removing the abandonment loop fails it.

## Orthogonality

Subagent telemetry and cost are unchanged.

## Revisions

- 2026-10-19: created
