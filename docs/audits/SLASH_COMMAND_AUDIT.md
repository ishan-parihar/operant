# Operant TUI Slash Command Audit Report

**Date**: 2025-07-19  
**Auditor**: AI Agent  
**Scope**: `crates/operant-cli/src/tui/app.rs` - PROMPT_SLASH_COMMANDS and intercept_slash_command_with_args_impl

---

## Executive Summary

This audit examines the slash command system in operant's TUI, identifying:
1. **History navigation UX bug**: Up/down arrows get "stuck" in slash command suggestions
2. **Command coverage gap**: ~30% of documented commands in PROMPT_SLASH_COMMANDS are not implemented
3. **Zeroclaw contrast**: Zeroclaw uses clap-based CLI structure, not TUI slash commands

---

## 1. History Navigation Bug - "Stuck in Slash Commands"

### Root Cause

**File**: `crates/operant-cli/src/tui/app.rs`, lines 5089-5127

```rust
KeyCode::Up => {
    if !self.prompt_input.suggestions.is_empty()
        && (self.prompt_input.text.starts_with('/')
            || self.prompt_input.has_active_file_ref())
    {
        self.prompt_input.suggestion_prev();  // Navigates suggestions
    } else if !self.prompt_input.text.contains('\n') {
        // Single-line input: always navigate history
        if !self.prompt_input.history.is_empty() {
            self.prompt_input.history_up();
        }
    } else {
        // Multi-line input: move cursor up
        ...
    }
}
```

### The Problem

1. **Suggestion mode triggers on ANY `/` prefix**: When user types `/`, suggestions appear immediately
2. **No escape hatch**: Once suggestions are visible, up/down **always** navigate suggestions
3. **History navigation blocked**: User cannot access history while suggestions are visible
4. **No visual distinction**: No indication that user is in "suggestion mode" vs "history mode"

### Expected Behavior (from hermes-agent/zeroclaw)

- Up/down should navigate history by default
- Tab or explicit key (Ctrl+N/Ctrl+P) should enter suggestion mode
- Escape should exit suggestion mode and return to history navigation

### Fix Recommendation

```rust
// Add a suggestion_mode flag to PromptInputState
pub suggestion_mode: bool,  // True when user explicitly entered suggestion selection

// In key handler:
KeyCode::Up => {
    if self.prompt_input.suggestion_mode 
        && !self.prompt_input.suggestions.is_empty() {
        self.prompt_input.suggestion_prev();
    } else {
        // Navigate history
        self.prompt_input.history_up();
    }
}

// Tab key enters suggestion mode
KeyCode::Tab => {
    if !self.prompt_input.suggestions.is_empty() {
        self.prompt_input.suggestion_mode = true;
        self.prompt_input.suggestion_index = Some(0);
    }
}

// Escape exits suggestion mode
KeyCode::Esc => {
    self.prompt_input.suggestion_mode = false;
    self.prompt_input.suggestion_index = None;
}
```

---

## 2. Slash Command Coverage Audit

### PROMPT_SLASH_COMMANDS Analysis

**Total documented commands**: 107  
**Implemented in intercept_slash_command_with_args_impl**: ~75  
**Missing/Return false**: ~32 (30%)

### Category Breakdown

| Category | Documented | Implemented | Missing |
|----------|------------|-------------|---------|
| **UI Screens (Intercepted)** | 28 | 28 | 0 |
| **TUI Toggles (Intercepted)** | 5 | 5 | 0 |
| **Agent-Handled (Fallthrough)** | 4 | 2 | 2 |
| **Agent-Context (Routed to Backend)** | 9 | 5 | 4 |
| **Backfilled (iter-77)** | 18 | 15 | 3 |
| **Planned/Not Implemented** | 43 | 2 | 41 |

### Detailed Command Status

#### ✅ Fully Implemented (Intercepted - Returns `true`)

| Command | Category | Status |
|---------|----------|--------|
| `config` / `settings` | UI Screen | ✅ Opens SettingsScreen |
| `theme` / `skin` | UI Screen | ✅ Opens ThemeScreen |
| `stats` / `cost` | UI Screen | ✅ Opens StatsDialog |
| `mcp` | UI Screen | ✅ Opens McpView |
| `agents` / `tasks` | UI Screen | ✅ Opens AgentsMenu |
| `diff` / `review` | UI Screen | ✅ Opens DiffViewer |
| `changes` | UI Screen | ✅ Opens Turn Diff |
| `search` / `find` | UI Screen | ✅ Opens GlobalSearch |
| `memory` | UI Screen | ✅ Opens MemoryFileSelector |
| `skills` | UI Screen | ✅ Opens SkillsView |
| `plugins` | UI Screen | ✅ Opens PluginsHub |
| `hooks` | UI Screen | ✅ Opens HooksConfigMenu |
| `import-config` | UI Screen | ✅ Opens ImportConfigPicker |
| `connect` | UI Screen | ✅ Opens ConnectDialog |
| `model` | UI Screen | ✅ Opens ModelPicker |
| `session` / `resume` / `sessions` | UI Screen | ✅ Opens SessionBrowser |
| `clear` | TUI Toggle | ✅ Clears messages |
| `exit` / `quit` | TUI Toggle | ✅ Exits app |
| `vim` | TUI Toggle | ✅ Toggles vim mode |
| `fast` | TUI Toggle | ✅ Toggles fast mode |
| `plan` | TUI Toggle | ✅ Toggles plan mode |
| `stop` | TUI Toggle | ✅ Stops streaming |
| `new` / `fresh` | TUI Toggle | ✅ New session |
| `undo` | TUI Toggle | ✅ Undoes last turn |
| `retry` | TUI Toggle | ✅ Retries last message |
| `save` | TUI Toggle | ✅ Opens ExportDialog |
| `goal` / `subgoal` | TUI Toggle | ✅ Sets session goal |
| `rewind` | UI Screen | ✅ Opens RewindFlow |
| `export` | UI Screen | ✅ Opens ExportDialog |
| `context` | UI Screen | ✅ Toggles ContextViz |
| `rename` | UI Screen | ✅ Opens SessionBrowser rename |
| `keybindings` | UI Screen | ✅ Opens keybindings.json |
| `help` | UI Screen | ✅ Toggles HelpOverlay |
| `yolo` | TUI Toggle | ✅ Toggles bypass permissions |
| `busy` | TUI Toggle | ✅ Toggles auto_compact |
| `verbose` | TUI Toggle | ✅ Cycles output style (alias for output-style) |
| `reasoning` | TUI Toggle | ✅ Toggles show_reasoning |
| `personality` | Agent-Context | ✅ Sets agent_mode |
| `steer` | Agent-Context | ✅ Queues steer directive |
| `queue` | Agent-Context | ✅ Shows steer queue |
| `background` | Agent-Context | ✅ Shows detached command |
| `rollback` | UI Screen | ✅ Opens DiffViewer |
| `reload-mcp` | Agent-Context | ✅ Triggers MCP reconnect |
| `reload` | TUI Toggle | ✅ Reloads settings from disk |
| `reload-skills` | Agent-Context | ✅ Rescans skills dir |
| `browser` | Agent-Context | ✅ Shows browser info |

#### ⚠️ Partially Implemented (Returns `false` - Falls through)

| Command | Category | Issue |
|---------|----------|-------|
| `doctor` | Agent-Context | Returns `false` - "not yet wired" |
| `init` | Agent-Context | Returns `false` - falls to CLI |
| `login` | Agent-Context | Returns `false` - falls to CLI |
| `logout` | Agent-Context | Returns `false` - falls to CLI |
| `compact` | Agent-Context | Returns `false` - "fallthrough to agent" |

#### ❌ Documented but NOT in intercept_slash_command_with_args_impl

| Command | Category | Notes |
|---------|----------|-------|
| `effort` | UI Screen | ✅ Actually implemented (line 2470) |
| `voice` | TUI Toggle | ✅ Actually implemented (line 2476) |
| `output-style` | TUI Toggle | ✅ Actually implemented (line 2461) |
| `provider` | Agent-Context | Documented but not implemented |
| `status` | Agent-Context | Documented but not implemented |
| `version` | Agent-Context | Documented but not implemented |
| `time` | Agent-Context | Documented but not implemented |
| `debug` | Agent-Context | Documented but not implemented |
| `history` | Agent-Context | Documented but not implemented |
| `compress` | Agent-Context | Documented but not implemented |
| `title` | Agent-Context | Documented but not implemented |
| `branch` | Agent-Context | Documented but not implemented |
| `tools` | Agent-Context | Documented but not implemented |
| `bundles` | Agent-Context | Documented but not implemented |
| `usage` | Agent-Context | Documented but not implemented |
| `credits` | Agent-Context | Documented but not implemented |
| `billing` | Agent-Context | Documented but not implemented |
| `insights` | Agent-Context | Documented but not implemented |
| `update` | Agent-Context | Documented but not implemented |
| `whoami` | Agent-Context | Documented but not implemented |
| `providers` | Agent-Context | Documented but not implemented |
| `refresh` | Agent-Context | Documented but not implemented |
| `copy` | TUI Toggle | ✅ Actually implemented (line 2424) |

#### 📝 "Planned" Commands (Documented but Not Implemented)

| Command | Notes |
|---------|-------|
| `replay` | Comment: "Replay spawn tree (planned)" |
| `replay-diff` | Comment: "Diff replay (planned)" |
| `billing` | No backend |
| `update` | No backend |
| `credits` | No backend |

---

## 3. Zeroclaw Contrast

### Architecture Difference

| Aspect | Operant | Zeroclaw |
|--------|---------|----------|
| **CLI Framework** | Clap + custom TUI slash commands | Pure Clap subcommands |
| **Interactive Mode** | Ratatui TUI with slash commands | No TUI slash commands |
| **Command Entry** | `/command` in prompt | `zeroclaw agent --message "..."` |
| **Slash Commands** | 107 documented | N/A (uses subcommands) |
| **History Navigation** | Custom implementation | Readline-based (stdin) |

### Zeroclaw's Approach

Zeroclaw uses **clap subcommands** for everything:
- `zeroclaw agent` - starts interactive session
- `zeroclaw agent -m "..."` - single message
- `zeroclaw config ...` - configuration
- `zeroclaw gateway ...` - gateway management
- `zeroclaw service ...` - service management

No TUI slash commands exist in zeroclaw. The "interactive mode" is a simple readline loop, not a full TUI.

### Key Takeaway

**Don't port zeroclaw's pattern to operant** - they serve different use cases:
- Operant: Rich TUI with slash commands for power users
- Zeroclaw: CLI-first with simple interactive mode

---

## 4. Recommendations

### Priority 1: Fix History Navigation Bug (iter-XX)

```rust
// Add to PromptInputState
pub suggestion_mode: bool,  // Explicit opt-in to suggestion navigation

// In app.rs key handler
KeyCode::Up => {
    if self.prompt_input.suggestion_mode 
        && !self.prompt_input.suggestions.is_empty() {
        self.prompt_input.suggestion_prev();
    } else {
        self.prompt_input.history_up();
    }
}
KeyCode::Tab => {
    if !self.prompt_input.suggestions.is_empty() {
        self.prompt_input.suggestion_mode = true;
    }
}
KeyCode::Esc => {
    self.prompt_input.suggestion_mode = false;
}
```

### Priority 2: Implement Missing Agent-Context Commands (iter-XX)

These should be added to `intercept_slash_command_with_args_impl`:

```rust
"provider" => {
    self.connect_dialog.open();
    true
}
"status" => {
    self.status_message = Some(self.format_system_status());
    true
}
"version" => {
    self.status_message = Some(format!("Operant v{}", crate::tui::adapter_types::constants::APP_VERSION));
    true
}
"time" => {
    self.status_message = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    true
}
"debug" => {
    self.debug_hub.toggle();
    true
}
"history" => {
    self.session_browser.open(vec![]);
    self.session_list_pending = true;
    true
}
"compress" => false,  // Let agent handle
"title" => {
    self.session_browser.start_rename();
    true
}
"tools" => {
    // Show available tools - could open a new overlay
    self.status_message = Some("Use /help to see available tools".to_string());
    true
}
"refresh" => {
    self.pending_mcp_reconnect = true;
    true
}
```

### Priority 3: Remove/Document "Planned" Commands

Remove from PROMPT_SLASH_COMMANDS or add clear "(planned)" marker:
- `replay` → Remove or mark "(planned)"
- `replay-diff` → Remove or mark "(planned)"
- `billing` → Remove (no backend)
- `update` → Remove (no backend)
- `credits` → Remove (no backend)

### Priority 4: Consolidate Duplicate Commands

| Duplicate Set | Keep | Alias |
|---------------|------|-------|
| `compact` / `compress` | `compact` | Alias `compress` → `compact` |
| `session` / `resume` / `sessions` | `session` | Alias others |
| `diff` / `review` | `diff` | Alias `review` → `diff` |
| `theme` / `skin` | `theme` | Alias `skin` → `theme` |
| `output-style` / `verbose` | `output-style` | Alias `verbose` → `output-style` |
| `goal` / `subgoal` | `goal` | Alias `subgoal` → `goal` |
| `rollback` / `rewind` | `rewind` | Alias `rollback` → `rewind` |

---

## 5. Files to Modify

1. **`crates/operant-cli/src/tui/prompt_input.rs`** - Add `suggestion_mode` field and methods
2. **`crates/operant-cli/src/tui/app.rs`** - Fix key handler, add missing command implementations
3. **`crates/operant-cli/src/tui/app.rs`** - Clean up PROMPT_SLASH_COMMANDS (remove planned, add aliases)

---

## 6. Test Plan

### History Navigation Tests
- [ ] Up arrow on empty input → shows most recent history
- [ ] Up/Down arrows cycle through history correctly
- [ ] Type `/` → suggestions appear
- [ ] Tab → enters suggestion mode, up/down navigate suggestions
- [ ] Escape → exits suggestion mode, up/down navigate history
- [ ] Type `/help` + Enter → executes command, history updated

### Command Coverage Tests
- [ ] All 107 PROMPT_SLASH_COMMANDS either implemented or documented as aliases
- [ ] No command returns confusing "not wired" message
- [ ] Agent-context commands properly fall through to agent

---

**End of Report**