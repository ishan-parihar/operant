# Operant TUI Audit Report: Input-Box & Interaction Features
**Date**: 2025-07-19
**Scope**: Contrast operant TUI with claurst TUI (`/home/ishanp/Documents/GitHub/CLONED-REPOS/claurst/src-rust/crates/tui/`)
**Focus**: Input-box, history navigation, suggestions, dialog handling, key interactions

---

## Executive Summary

| Area | Operant Status | Claurst Status | Gap |
|------|----------------|----------------|-----|
| **History Navigation** | ✅ Fixed (iter-83) | Works | Minor UX polish |
| **Suggestion Acceptance** | ❌ Missing | Full `accept_suggestion_for_submit` | Critical |
| **Multi-line Visual Movement** | ✅ Implemented | Same approach | Parity |
| **Context Menu System** | ❌ Missing | Full implementation | Medium |
| **Dialog Handling** | ✅ Partial | More comprehensive | Medium |
| **Alt+Up/Down (Message Jumps)** | ❌ Missing | Implemented | Low |
| **Key Context System** | ❌ Missing | `KeyContext` enum | Medium |

---

## Detailed Findings

### 1. Suggestion Acceptance Flow — **Critical Missing**

**Claurst** (`app.rs:3054-3087`):
```rust
/// Attempt to accept the currently-selected suggestion and returns whether
/// the prompt should now be submitted.
fn accept_suggestion_for_submit(&mut self) -> bool {
    if self.prompt_input.suggestions.is_empty() { return false; }
    self.prompt_input.suggestion_index
        .and_then(|i| self.prompt_input.suggestions.get(i))
        .map(|s| { self.prompt_input.accept_suggestion(); true })
        .unwrap_or(false)
}
```
Used in Enter key handler to complete suggestions before submitting.

**Operant**: No equivalent. Enter always submits raw text, suggestions never auto-complete.

**Fix Required**: Add `accept_suggestion_for_submit()` to `PromptInputState` and wire into Enter key handler in `app.rs`.

---

### 2. Context Menu System — **Medium Gap**

**Claurst** (`app.rs:3116-3131`):
```rust
if self.context_menu_state.is_some() {
    match key.code {
        KeyCode::Esc => { self.dismiss_context_menu(); return false; }
        KeyCode::Up | KeyCode::Down => { self.navigate_context_menu(key.code); return false; }
        KeyCode::Enter => { self.execute_context_menu_item(); return false; }
        _ => {}
    }
}
```

**Operant**: No context menu implementation.

**Fix Required**: Add `ContextMenuState` to `App`, implement render/navigate/execute, hook into right-click or Ctrl+Shift+M.

---

### 3. Suggestion Index Behavior — **Subtle UX Difference**

**Claurst** (`app.rs:4443-4450`):
```rust
if !self.prompt_input.suggestions.is_empty() {
    if self.prompt_input.suggestion_index.is_none() {
        self.prompt_input.suggestion_index = Some(0);  // Auto-select first
    }
    self.prompt_input.accept_suggestion();
}
```
Auto-selects first suggestion on Enter if visible but none selected.

**Operant**: Requires explicit Tab to enter suggestion mode (iter-83 fix).

**Recommendation**: Add auto-select behavior on Enter when suggestions visible.

---

### 4. Multi-line Input Handling — **Parity Achieved**

Both implementations now use the same pattern (claurst `app.rs:4537-4564`, operant `app.rs:5089-5127`):

```rust
KeyCode::Up => {
    if suggestions_visible && (text.starts_with('/') || has_file_ref) {
        suggestion_prev();
    } else if !text.contains('\n') {
        history_up();
    } else {
        move_visual_up(width);
    }
}
```

**Status**: ✅ Parity (fixed in iter-83)

---

### 5. Alt+Up/Down Message Boundary Navigation — **Missing in Operant**

**Claurst** (`app.rs:4518-4530`):
```rust
KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
    self.scroll_up_by(20);  // Jump ~20 lines (message boundary)
}
KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
    let new_off = self.scroll_offset.saturating_sub(20);
    self.scroll_offset = new_off;
}
```

**Operant**: Not implemented.

**Fix**: Add Alt+Up/Down handlers in key match arm.

---

### 6. Key Context System — **Architectural Difference**

**Claurst** (`app.rs:4597-4630`):
```rust
fn current_key_context(&self) -> KeyContext {
    if self.diff_viewer.visible { KeyContext::DiffDialog }
    else if self.agents_menu.visible || ... { KeyContext::Select }
    ...
}
```
Used to determine which key bindings apply.

**Operant**: No context system; all keys handled in flat match.

**Recommendation**: Add `KeyContext` enum and `current_key_context()` for cleaner routing.

---

### 7. Dialog Handling Priority — **Operant Lacks Hierarchy**

**Claurst** handles dialogs in strict priority order:
1. Context menu
2. Bypass permissions dialog (highest priority)
3. File injection dialog
4. Onboarding dialog
5. Effort picker
6. Settings screen
7. Theme picker
8. ...

**Operant**: Uses flat `if self.xxx.visible { ... }` without explicit priority.

**Recommendation**: Extract dialog handling into priority-ordered helper or match on `DialogPriority`.

---

### 8. Suggestion Update Suggestions Update Throttling — **Operant Missing Guard**

**Claurst** (`app.rs:2767-2773`):
```rust
// Don't update suggestions while the injection dialog is open.
if !self.file_injection_dialog.visible {
    self.prompt_input.update_suggestions(...);
}
```

**Operant**: Updates suggestions unconditionally every frame.

**Fix**: Add guard around `update_suggestions()` call.

---

## Refactor Plan (Priority Order)

### Phase 1: Critical UX (Week 1)
| Task | Files | Effort |
|------|-------|--------|
| 1.1 Add `accept_suggestion_for_submit()` to `PromptInputState` | `prompt_input.rs` | 2h |
| 1.2 Wire into Enter key handler in `app.rs` | `app.rs` | 1h |
| 1.3 Auto-select first suggestion on Enter when visible | `app.rs` | 30m |
| 1.4 Add suggestions update guard (skip during dialogs) | `app.rs` | 30m |

### Phase 2: Navigation Polish (Week 1-2)
| Task | Files | Effort |
|------|-------|--------|
| 2.1 Add Alt+Up/Down for message boundary jumps | `app.rs` | 1h |
| 2.2 Add Context Menu system (`ContextMenuState`, render, nav) | `app.rs`, new `context_menu.rs` | 4h |
| 2.3 Auto-select first suggestion when suggestions appear | `prompt_input.rs` | 1h |

### Phase 3: Architecture (Week 2-3)
| Task | Files | Effort |
|------|-------|--------|
| 3.1 Add `KeyContext` enum and `current_key_context()` | `app.rs` | 2h |
| 3.2 Refactor dialog handling into priority-ordered system | `app.rs` | 3h |
| 3.3 Add dialog priority system (like claurst's ordered checks) | `app.rs` | 2h |

### Phase 4: Advanced Features (Week 3+)
| Task | Files | Effort |
|------|-------|--------|
| 4.1 Right-click → context menu | `app.rs`, `render.rs` | 3h |
| 4.2 Ctrl+Shift+M for context menu keyboard access | `app.rs` | 1h |
| 4.3 Visual selection improvements (Shift+arrows) | `prompt_input.rs` | 2h |

---

## Code References

### Claurst Key Files
- `/home/ishanp/Documents/GitHub/CLONED-REPOS/claurst/src-rust/crates/tui/src/app.rs` (4600+ lines)
- `/home/ishanp/Documents/GitHub/CLONED-REPOS/claurst/src-rust/crates/tui/src/prompt_input.rs` (196k lines)
- `/home/ishanp/Documents/GitHub/CLONED-REPOS/claurst/src-rust/crates/tui/src/lib.rs` (58k lines)

### Operant Key Files
- `/home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/operant/crates/operant-cli/src/tui/app.rs` (5200+ lines)
- `/home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/operant/crates/operant-cli/src/tui/prompt_input.rs` (175k lines)
- `/home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/operant/crates/operant-cli/src/tui/keybindings.rs`

---

## Validation Checklist

### Post-Phase 1
- [ ] Tab → Enter completes suggestion
- [ ] Enter with suggestions visible but none selected → auto-completes first
- [ ] Suggestions don't update while file_injection_dialog visible
- [ ] History navigation works correctly (iter-83 verified)

### Post-Phase 2
- [ ] Alt+Up jumps ~20 lines up
- [ ] Alt+Down jumps ~20 lines down
- [ ] Context menu opens on right-click / Ctrl+Shift+M
- [ ] Context menu navigable with Up/Down, selectable with Enter

### Post-Phase 3
- [ ] Key context correctly identifies active dialog
- [ ] Dialog priority prevents lower-priority dialogs from intercepting keys
- [ ] No key conflicts between overlapping dialogs

---

## Notes on "Dead Weight" (YAGNI)

**Don't port from claurst:**
- `rustle.rs` (startup animation) - operant has simpler banner
- `desktop_upsell_startup.rs`, `overage_upsell.rs` - monetization not needed
- `feedback_survey.rs` - no telemetry backend
- `memory_update_notification.rs` - operant uses different memory model
- `plugin_views.rs` - operant uses `plugins_hub.rs`

**Do port:**
- `accept_suggestion_for_submit()` — core UX
- Context menu — power user essential
- Alt+Up/Down — navigation efficiency
- Dialog priority system — prevents bugs

---

## Dependencies

- Phase 1 requires no new dependencies
- Phase 2 requires no new dependencies (uses existing ratatui widgets)
- Phase 3 is pure refactor
- Phase 4 may need `crossterm` mouse event handling (already in deps)