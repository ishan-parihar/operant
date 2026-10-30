# Session Picker Upgrade Plan — Subagent Distinction & Animation Collision

## Overview

This plan addresses two related issues in the operant TUI session hub:

1. **Subagent session distinction** — Subagent rows (`RowKind::Subagent`) need to visually stand out from regular sessions, similar to how omp shows subagents separately with badges/icons.

2. **Home-screen animation collision** — The idle TUI animation collides with the session picker when opened, causing visual artifacts and potential performance issues.

## Current State Analysis

### Subagent Handling in Session Hub

The session hub (`crates/operant-cli/src/tui/session_hub/`) already supports subagent rows via `RowKind::Subagent { goal, model, dispatch_time, status }`. However, the rendering does not visually distinguish them from regular sessions:

- In `render.rs`, the list row renders `row_title(row)` which only shows the title/goal
- There's no badge or icon to indicate "this is a subagent"
- The `focused_subagent` field exists but is only used when that specific subagent is focused
- The `status_glyph` function only handles `RowStatus` (Live/Idle/Archived), not subagent status

### Animation Collision

The idle animation (`crates/operant-cli/src/tui/operant_anim/mod.rs`) runs continuously and:
- Renders a radar-pulse animation on the terminal
- When the session picker is open, the animation runs underneath the picker
- This causes visual artifacts and can make the picker harder to read
- The animation should pause or be suppressed when the session picker is active

### Double-Left Gesture

The session hub already has `armed_close_tap` support (from iter-1476 commit 49cde7b4):
- Opening the hub with `←←` arms the close-tap
- A single further `←←` closes the hub
- This needs to work correctly with subagent rows

## Changes Required

### 1. Visual Distinction for Subagent Sessions

**File**: `crates/operant-cli/src/tui/session_hub/render.rs`

**a) Update `row_title` to show subagent badge**:
```rust
fn row_title(row: &SessionEntryV2) -> String {
    match &row.kind {
        RowKind::Subagent { goal, .. } => {
            let short: String = goal.chars().take(48).collect();
            format!("🤖 {short}")  // Add 🤖 badge prefix
        }
        // ... existing cases
    }
}
```

**b) Add subagent status glyph in list rendering**:
- In the list item rendering, after `status_glyph(row.status)`, add a subagent indicator
- Use a different color (cyan or magenta) for subagent rows
- Show `🤖` or similar icon for subagent rows

**c) Update preview rendering for subagents**:
- In `render_preview`, the match against `(&row.kind, state.focused_subagent.as_deref())` already handles subagents
- When a subagent row is selected and `focused_subagent` matches, show the subagent detail view
- Add distinct styling for the subagent goal line

### 2. Fix Animation Collision

**File**: `crates/operant-cli/src/tui/operant_ui/ui_animations.rs`

**a) Add session hub open/close tracking**:
- Track whether the session hub is visible
- When session hub opens, pause the idle animation
- When session hub closes, resume the idle animation

**b) Modify `idle_animation_reserved_height` and `draw_idle_animation`**:
- Check if session hub is visible before reserving space for animation
- Skip animation rendering when session hub is open

**c) Add app state flag**:
- In `crates/operant-cli/src/tui/operant_app/mod.rs` or `tui_state.rs`, add a flag
- `session_hub_visible: bool` that the animation module can check

### 3. Ensure Double-Left Gesture Works

**File**: `crates/operant-cli/src/tui/operant_app/app.rs` (or relevant file)

**a) Ensure `/resume` (session picker) opens with armed_close_tap**:
- When `/resume` is invoked, pass `Some(Instant::now())` as the armed_close_tap
- This enables the `←←` gesture to both open and close the hub

**b) Verify `left_tap_close` works correctly**:
- The `SessionHubState::left_tap_close()` method already handles this
- Ensure it's called from the right place in the key handling

## Implementation Details

### Subagent Visual Pattern (from jcode reference)

Looking at jcode's session picker (`crates/jcode-tui-session-picker/`), subagents are distinguished by:
- 🧵 Claude Code badge
- 🧠 Codex badge  
- π Pi badge
- ◌ OpenCode badge
- ▮ Cursor badge

For operant, we'll use `🤖` as the subagent indicator, consistent with the existing `🤖` used in `row_title` for `RowKind::Subagent { goal, .. }`.

### Animation Pause Mechanism

The animation module already has:
- `idle_animation_enabled(self) -> bool` in `perf.rs`
- `idle_animation_fast_path_blocked_reasons()` diagnostic
- `idle_animation_reserved_height(show_animation: bool, input_height: u16) -> u16`
- `record_idle_animation_area(area: Option<Rect>)`

We'll add:
- Check against `session_hub_visible` flag
- Pause/resume logic based on hub visibility

## Migration Path

1. **First**: Add visual distinction for subagent rows (badges, icons, colors)
2. **Second**: Add animation pause when session hub is open
3. **Third**: Ensure double-left gesture works end-to-end
4. **Fourth**: Add tests for all three areas

## Verification

After implementing:
- Subagent rows should show `🤖` badge in the list
- Subagent rows should have distinct coloring (accent cyan/magenta)
- Session picker opening should pause the idle animation
- Closing the session picker should resume the idle animation
- `←←` should open the hub and a second `←←` should close it
- All existing tests should pass