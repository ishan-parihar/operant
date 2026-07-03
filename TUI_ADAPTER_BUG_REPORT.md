# TUI Adapter Bug Report

**Date:** 2026-06-21  
**Scope:** Full audit of the TUI adapter layer in `hermes-rs/crates/operant-cli/src/tui/`  
**Status:** 239 compilation errors, zero in non-TUI files

---

## Executive Summary

The adapter approach is **fundamentally broken**. `adapter_types.rs` (760 lines) was intended to bridge the gap between TUI code (written for claurst's types) and operant's actual types. Instead:

1. **Shadows the real TUI** — `main.rs` calls a stub `TuiApp::run()` that returns `Ok(())` immediately. The 6782-line real TUI in `app.rs` never launches.
2. **No bridge wiring** — Zero code converting `AgentEvent` → `QueryEvent`. The agent can't talk to the TUI.
3. **239 compilation errors** — ALL in TUI files. Zero in non-TUI files.
4. **Type mismatches** — `handle_query_event` references types/variants that don't exist in the adapter's definitions.

---

## Critical Bugs (P0 — System Won't Start)

### Bug 1: `TuiApp::run()` is a no-op stub
**File:** `adapter_types.rs:748-751`
```rust
pub async fn run(self) -> anyhow::Result<()> {
    Ok(())  // Does nothing
}
```
**Impact:** `main.rs` imports this stub (not `app::App`). The real TUI never executes.

### Bug 2: No AgentEvent→QueryEvent bridge
**Impact:** Even if the real TUI launched, there's no code converting agent events into TUI events. The agent runs in isolation.

### Bug 3: `QueryEvent::Stream` wraps wrong type
**File:** `adapter_types.rs:388` defines `QueryEvent::Stream(StreamEvent)` but `app.rs:5882` matches against `AnthropicStreamEvent` (a different enum from the `streaming` module). These are incompatible types.

### Bug 4: `QueryEvent::Status` variant missing
**File:** `app.rs:6001` matches `QueryEvent::Status(msg)` but the `QueryEvent` enum has no `Status` variant.

---

## High-Severity Bugs (P1 — Features Inert)

### Bug 5: 85 "no field" errors (E0609)
The TUI accesses flat fields on `AppConfig` that don't exist:
- `self.config.provider` → doesn't exist (should be `self.config.agent.model` for model)
- `self.config.model` → doesn't exist at top level
- `self.config.theme` → doesn't exist (should be `self.config.tui.theme`)
- `self.config.permission_mode` → doesn't exist
- `self.config.output_style` → doesn't exist
- `self.config.append_system_prompt` → doesn't exist

**Root cause:** `Config` is re-exported from `operant_core::config::AppConfig` (nested structure), but TUI expects claurst's flat config.

### Bug 6: 81 "no method" errors (E0599)
Missing methods on types:
- `AppConfig::effective_model()` — defined in adapter impl but E0116 (can't impl for foreign type)
- `AppConfig::resolve_api_key()` — same E0116 issue
- `Settings::effective_config()` — doesn't exist
- `ModelRegistry::load_cache()` — doesn't exist
- `ModelRegistry::get()` — doesn't exist
- `Message::assistant()`, `Message::assistant_blocks()` — don't exist
- `AuthStore::set()`, `AuthStore::api_key_for()` — don't exist
- `VoiceRecorder::start_recording()`, `stop_recording()` — async on tokio::Mutex, code uses std::MutexGuard

### Bug 7: Missing enum variants (E0599 for variants)
- `PermissionMode::Plan` — not defined (only `AcceptEdits`, `Default`, `BypassPermissions`)
- `ContentBlock::RedactedThinking` — not defined
- `KeyContext::DiffDialog`, `Select`, `Confirmation`, `ThemePicker`, `Help`, `HistorySearch` — not defined (only `Global`, `Chat`, `Overlay`, `Settings`)
- `Theme::Custom(String)` — Theme is a struct, not an enum
- `PreviewAction::Import`, `Replace`, `Keep` — not defined (has `Apply`, `Skip`, `Cancel`)
- `AtFileIssue::Unreadable`, `TooLarge(_)` — `TooLarge` is unit variant, code treats as tuple variant

### Bug 8: Struct vs enum mismatches
- `KeybindingResult` — is a struct but code matches on `Action`, `Pending`, `NoMatch`, `Unbound` variants
- `StoredCredential` — is a struct but code uses `StoredCredential::ApiKey{key}` and `OAuthToken{token}` as enum variants
- `Theme` — is a struct `{name: String}` but code uses `Theme::Custom(String)` as enum
- `ImageSource` — is an enum but `kitty_image.rs` accesses it as a struct with `.source_type`, `.url`, `.data`, `.media_type` fields

### Bug 9: `settings_screen.rs` writes to wrong config
**File:** `settings_screen.rs:209-233`
Writes to `self.config.permission_mode`, `self.config.output_style`, `self.config.theme`, `self.config.model`, `self.config.provider` — none of which exist on `AppConfig`.

---

## Medium-Severity Bugs (P2 — Broken Features)

### Bug 10: 38 type mismatch errors (E0308)
- `Option<u64>` where `Option<u32>` expected (`retry_secs`) — 6 occurrences
- `&usize` where `usize` expected (`n_hidden`) — 5 occurrences
- `&bool.unwrap_or()` — bool has no `unwrap_or` method — 8 occurrences
- `&tool_use_id` (`&String`) passed to `HashMap::get` expecting `&str` — 3 occurrences

### Bug 11: `messages/mod.rs` has 3 near-identical rendering functions
All three fail the same way — 47 errors total. Fix once, apply 3x.

### Bug 12: `ModelEntry` simplified too aggressively
**File:** `adapter_types.rs:616` — stub is `{id, display_name, description, is_current}` but TUI expects `release_date`, `info.id`, `info.name`, `info.context_window`, `cost_input`, `cost_output`.

### Bug 13: `TASK_STORE` is `Mutex<TaskStore>` but code uses DashMap API
**File:** `adapter_types.rs:727` — code calls `.get(&key)` which doesn't work on `Mutex`.

### Bug 14: `FileHistory::snapshots_for_turn()` returns `Vec<String>` but code treats items as structs
**File:** `diff_viewer.rs:283-296` — accesses `.path`, `.binary`, `.before_text`, `.after_text` on String.

---

## Low-Severity Issues (P3 — Cosmetic/Minor)

### Bug 15: 31 stale `claurst` references across 11 files
References to `claurst` in type names, comments, and imports.

### Bug 16: `ContextWindowForModel` returns `impl Future` but used as value
**File:** `app.rs:2565` — needs `.await`.

### Bug 17: `Theme` const items use `&str` where `String` expected
**File:** `adapter_types.rs:84-87` — `Theme { name: "dark" }` should be `Theme { name: "dark".to_string() }`.

---

## Architecture Problems

### God Struct
`App` in `app.rs` is 7624 lines with 80+ fields. This makes the codebase hard to maintain and reason about.

### Dead Code
`handle_query_event()` exists on the real `App` struct but is never called from anywhere. It's orphaned because the adapter `TuiApp` shadows it.

### Error Type Mismatch
`Settings::save_sync()` returns `Result<(), String>` but callers expect `Result<(), anyhow::Error>`.

---

## Event Flow Analysis

### AgentEvent (operant-core)
```rust
pub enum AgentEvent {
    Thinking { content: String },
    Reasoning { text: String },
    ToolStart { name: String, arguments: String },
    ToolComplete { result: ToolResult },
    ToolError { name: String, error: String },
    Content { text: String },
    Done { message: Message },
    IterationComplete { iteration: usize },
    Error { error: String },
    Usage { input_tokens: u32, output_tokens: u32, total_tokens: u32 },
    ToolPermissionRequest { tool_name, tool_id, description, danger_explanation, input_preview },
}
```

### QueryEvent (adapter_types.rs)
```rust
pub enum QueryEvent {
    Stream(StreamEvent),
    ToolStart { tool_name: String, tool_id: String, input_json: String },
    ToolEnd { tool_id: String, tool_name: String, result: String, is_error: bool },
    TurnComplete { turn: usize, stop_reason: String, usage: Option<UsageInfo> },
    Error(String),
    TokenWarning { state: TokenWarningState, pct_used: f64 },
}
```

### Required Bridge Mapping (DOES NOT EXIST)
| AgentEvent | → QueryEvent | Notes |
|------------|-------------|-------|
| `Content { text }` | `Stream(StreamEvent::ContentBlockDelta { delta })` | Wrap text in StreamEvent |
| `ToolStart { name, arguments }` | `QueryEvent::ToolStart { tool_name, tool_id, input_json }` | Missing tool_id — needs generation |
| `ToolComplete { result }` | `QueryEvent::ToolEnd { tool_id, tool_name, result, is_error }` | Result content extraction needed |
| `ToolError { name, error }` | `QueryEvent::ToolEnd { is_error: true }` | |
| `Done { message }` | `QueryEvent::TurnComplete { turn, stop_reason, usage }` | Need turn counter, stop_reason inference |
| `Error { error }` | `QueryEvent::Error(String)` | Direct map |
| `Usage { .. }` | `QueryEvent::TurnComplete { usage }` | Token data → UsageInfo |
| `Thinking { content }` | _(no direct map)_ | Could map to Stream thinking delta |
| `IterationComplete` | _(no direct map)_ | Used internally |
| `ToolPermissionRequest` | _(no direct map)_ | Missing from QueryEvent entirely |

### Intended Flow (Not Wired)
```
User presses Enter
    → App::handle_key_event() returns true
    → App::run() calls take_input() → returns String
    → App::run() returns Ok(Some(input))
    → [MISSING] Outer orchestration loop receives input
    → [MISSING] agent.run(input).await → OperantAgent::run()
    → Agent emits AgentEvent via channel
    → [MISSING] Bridge task translates AgentEvent → QueryEvent
    → [MISSING] app.handle_query_event(query_event)
```

---

## Root Cause Analysis

The adapter approach failed because it tried to make TUI work **without modifying TUI code**. But TUI was written for claurst's type system, which is fundamentally different from operant's:

| Aspect | claurst | operant |
|--------|---------|---------|
| Config | Flat (`config.provider`, `config.model`) | Nested (`config.agent.model`, `config.client.api_key`) |
| Theme | Enum (`Theme::Custom(String)`) | Struct (`Theme { name: String }`) |
| KeybindingResult | Enum (`Action/Pending/NoMatch/Unbound`) | Struct `{ action, context }` |
| StoredCredential | Enum (`ApiKey{key}`, `OAuthToken{token}`) | Struct `{ provider, kind }` |
| PermissionMode | 4 variants (`+Plan`) | 3 variants (no Plan) |
| ContentBlock | 5 variants (`+RedactedThinking`) | 4 variants |
| ImageSource | Flat struct (`source_type`, `url`, `data`, `media_type`) | Enum (`Clipboard`, `File`, `Url`, `Paste`) |

The adapter tried to paper over these differences but couldn't add fields to foreign types (E0116) and couldn't add enum variants to existing enums. The result is a 760-line file that compiles 239 errors and launches nothing.

---

## Recommended Fix Plan

### Phase 1: Make It Compile (eliminate 239 errors)
1. **Fix adapter_types.rs type definitions** to match what app.rs actually expects
2. **Add missing enum variants** (Plan, RedactedThinking, KeyContext::*, etc.)
3. **Fix struct vs enum mismatches** (Theme, KeybindingResult, StoredCredential)
4. **Fix AppConfig field access** — either add flat fields to adapter or change TUI code to use nested paths
5. **Fix type mismatches** (u64 vs u32, &bool.unwrap_or, etc.)

### Phase 2: Make It Run (wire the bridge)
1. **Remove adapter TuiApp stub** — wire main.rs to use `app::App` directly
2. **Build AgentEvent→QueryEvent bridge** — a tokio task that maps events
3. **Add ToolPermissionRequest handling** to QueryEvent
4. **Wire handle_query_event()** into the main loop

### Phase 3: Make It Work (fix stubs)
1. **Wire Settings save/load** to actual disk
2. **Wire ModelRegistry** to real model discovery
3. **Wire AuthStore** to real credential storage
4. **Wire voice recorder** to real audio capture
5. **Wire keybinding resolver** to real keybinding parsing

### Phase 4: Clean Up
1. **Remove 31 claurst references**
2. **Extract bridge.rs** from adapter_types.rs
3. **Consider App god-struct decomposition** (low priority, high risk)
