# Backend-TUI Integration Audit

**Date**: 2026-07-06
**Scope**: Why operant uses a bridge/adapter layer between the agent backend and the TUI, what bugs it causes, and how to fix the architecture.

## Executive Summary

The TUI has a **2,663-line adapter layer** (`adapter_types.rs`) that duplicates types, converts between them, and introduces stubs that return empty data. This layer exists because the TUI was originally ported from a different codebase (operant-agent, a TypeScript/Ink TUI) and the types were copied rather than wired to the real operant-core types. The bridge (`bridge.rs`, 111 LOC) translates `AgentEvent` → `QueryEvent` — a pure 1:1 mapping that adds no value but introduces bugs when the two event types drift.

**The fix**: eliminate the adapter layer. Use `operant_core` types directly in the TUI. Remove the bridge — have `App` receive `AgentEvent` directly. This is a large refactor (~2,000 LOC deletion + ~500 LOC of import changes) but it eliminates an entire class of bugs permanently.

## Why the Adapter Layer Exists (Historical Context)

The TUI was ported from operant-agent (a TypeScript/Ink TUI). The original port copied the TypeScript type definitions into Rust as `adapter_types::types::*`. At the time, `operant_core` didn't exist as a separate crate — the agent logic was inline in `cli.py` (Python). When `operant_core` was extracted, the TUI's adapter types were never reconciled with the new core types.

## What the Adapter Layer Costs

### 1. Duplicate Types (11 duplicates)

| adapter_types type | operant_core equivalent | Divergence |
|---|---|---|
| `adapter_types::types::Message` | `operant_core::client::Message` | adapter_types has `MessageContent` enum (Text/Blocks); core has flat `content: String` + `reasoning: Option<String>` + `tool_calls`. The TUI can't represent tool_calls natively. |
| `adapter_types::types::Role` | `operant_core::client::Role` | Identical enum, just duplicated. |
| `adapter_types::types::ContentBlock` | No core equivalent | TUI-only type for rendering. Should stay in the TUI but be built FROM core::Message, not alongside it. |
| `adapter_types::config::Config` | `operant_core::config::AppConfig` | Config wraps AppConfig (`inner` field) but duplicates `provider`, `model`, `theme`, `permission_mode`, `output_style`. Two sources of truth → drift. |
| `adapter_types::config::Settings` | No core equivalent | TUI-only JSON settings. Legitimate — but should be the ONLY TUI-specific config, not layered on top of Config which layers on top of AppConfig. |
| `adapter_types::cost::CostTracker` | No core equivalent | TUI-only. Legitimate. |
| `adapter_types::file_history::FileHistory` | No core equivalent | TUI-only. But it's a stub — `snapshots_for_turn` returns `vec![]`, `latest_turn_index` returns `None`. Dead code. |
| `adapter_types::mcp::McpManager` | `operant_core::mcp::McpManager` | TUI's version is a **stub** — `all_tool_definitions` returns `vec![]`, `server_status` returns `Disconnected`. The real McpManager lives in core but the TUI uses the stub. |
| `adapter_types::history::SessionRecord` | `operant_core::database::DatabaseSession` | adapter_types was a stub (returned fake "Current Session") until iter-82 wired it to the real Database. Still has its own type instead of using DatabaseSession. |
| `adapter_types::tools::UserQuestionEvent` | `operant_core::user_question::UserQuestionRequest` | Duplicated. The TUI's version lacks `reply_tx`. iter-97 added the core type; the TUI type is now dead. |
| `adapter_types::tools::TaskStore` | No core equivalent | TUI-only. But it's never used — `TASK_STORE` static is declared but no code reads from it. Dead code. |

### 2. The Bridge (bridge.rs) — Pure Overhead

The bridge translates `AgentEvent` → `QueryEvent` in a 1:1 mapping:

| AgentEvent | QueryEvent | Value added |
|---|---|---|
| `Thinking { content }` | `Stream(ThinkingDelta { delta: content })` | None — just renames `content` to `delta` |
| `Reasoning { text }` | `Stream(ThinkingDelta { delta: text })` | None — merges two variants into one (OK) |
| `Content { text }` | `Stream(ContentBlockDelta { delta: text })` | None — just renames |
| `ToolStart { tool_call_id, name, arguments }` | `ToolStart { tool_name, tool_id, input_json }` | None — just renames fields |
| `ToolComplete { result }` | `ToolEnd { tool_id, tool_name, result, is_error }` | Slight — extracts success/error. But the TUI could do this itself. |
| `Done { message }` | `TurnComplete { turn: 0, stop_reason: "end_turn", usage }` | **Bug**: always sets `turn: 0` and `stop_reason: "end_turn"` regardless of actual values. Drops the `message` field entirely. |
| `Error { error }` | `Error(error)` | None |
| `Usage { ... }` | (stores in pending_usage, sent with TurnComplete) | Slight — batches usage with completion. But loses `total_tokens` and `total_cost`. |
| `IterationComplete { iteration }` | `None` (dropped!) | **Bug**: iteration count is lost. The TUI's "iter N" pill (iter-78) never works via the bridge. |
| `ToolPermissionRequest { ... }` | `Status(format!("Permission needed: ..."))` | **Bug**: converts a structured permission request into a plain status string. The TUI can't show an approve/deny dialog from this. |

### 3. Bugs Caused by the Adapter Layer

| # | Bug | Cause | Iteration fixed |
|---|---|---|---|
| 1 | Thinking content appeared as `[thinking]` literal text | Bridge prefixed `content` with `[thinking]` instead of using a separate event type | iter-113 |
| 2 | `Done { message }` dropped the final message | Bridge converted Done to TurnComplete but discarded `message` | Never fixed — the TUI relies on `flush_streamed_assistant_message` instead |
| 3 | `IterationComplete` dropped | Bridge returned `None` for iteration events | iter-78 worked around it by using `current_turn` AtomicUsize |
| 4 | `ToolPermissionRequest` converted to status string | Bridge lost the structured request | iter-82 wired a separate permission_rx channel, but the bridge still drops the event |
| 5 | `Usage` loses `total_tokens` and `total_cost` | Bridge constructs `UsageInfo` with `total_cost: 0.0` | Never fixed — cost_tracker uses its own calculation |
| 6 | `turn: 0` always | Bridge hardcodes `turn: 0` in TurnComplete | Never fixed — the TUI ignores this field |
| 7 | TUI's `McpManager` is a stub returning empty data | adapter_types defines its own McpManager that returns `vec![]` | iter-82 attached the stub; iter-93 attached the real one via `core_mcp_manager` |
| 8 | TUI's `FileHistory` is a stub | adapter_types defines its own FileHistory with `snapshots_for_turn` returning `vec![]` | iter-82 attached a stub; never wired to real data |
| 9 | `UserQuestionEvent` lacked `reply_tx` | adapter_types defined its own type without the reply channel | iter-97 added core type; TUI type is now dead |
| 10 | `Config` duplicates `AppConfig` fields | `provider`, `model`, `theme`, `permission_mode` exist in both Config and Config.inner (AppConfig) | iter-71, iter-112 added sync logic — but the duplication is the root cause |

### 4. Stubs in adapter_types.rs That Return Fake Data

| Stub | What it returns | Impact |
|---|---|---|
| `McpManager::all_tool_definitions()` | `vec![]` | /mcp shows no tools |
| `McpManager::server_status()` | `Disconnected` | /mcp shows all servers disconnected |
| `FileHistory::snapshots_for_turn()` | `vec![]` | /changes shows no changes |
| `FileHistory::latest_turn_index()` | `None` | /changes can't determine the current turn |
| `tips::select_tip()` | Was `None` (fixed in iter-106) | Welcome screen showed "Edit AGENTS.md" |
| `history::list_sessions()` | Was fake "Current Session" (fixed in iter-82) | /resume showed one fake session |
| `spinner::random_face()` | `"●"` | Static spinner face |
| `sample_spinner_verb()` | Was "thinking" (fixed in iter-109) | No variety |
| `sample_completion_verb()` | Was "done" (fixed in iter-109) | No variety |

## Recommended Architecture

### Phase 1: Eliminate the Bridge (iter-114)

**Delete `bridge.rs` entirely.** Have `App::handle_query_event` become `App::handle_agent_event` that takes `AgentEvent` directly.

Changes:
- `App::query_event_rx` → `App::agent_event_rx: Option<mpsc::Receiver<AgentEvent>>`
- `handle_query_event(event: QueryEvent)` → `handle_agent_event(event: AgentEvent)`
- Move the bridge's translation logic inline (it's trivial — 1:1 field renames)
- Fix the bugs the bridge introduced: don't drop `Done.message`, don't drop `IterationComplete`, don't convert `ToolPermissionRequest` to a string

**Estimated LOC change**: -111 (bridge.rs deleted) + ~50 (inline translation in handle_agent_event) = net -61 LOC

### Phase 2: Use Core Types Directly (iter-115)

**Delete `adapter_types::types::*` and use `operant_core::client::*` directly.**

Changes:
- Replace `adapter_types::types::Message` with `operant_core::client::Message`
- Replace `adapter_types::types::Role` with `operant_core::client::Role`
- Replace `adapter_types::types::ContentBlock` — keep as a TUI-only rendering type, but build it FROM `core::Message` not alongside it
- Delete `adapter_types::types::MessageContent` — use core::Message's `content` + `reasoning` + `tool_calls` fields
- Delete `adapter_types::query::{QueryEvent, StreamEvent, UsageInfo}` — use `AgentEvent` directly (Phase 1 already does this)
- Delete `adapter_types::mcp::McpManager` stub — use `operant_core::mcp::McpManager`
- Delete `adapter_types::file_history::FileHistory` stub — either wire to real data or remove the feature
- Delete `adapter_types::tools::UserQuestionEvent` — use `operant_core::user_question::UserQuestionRequest`
- Delete `adapter_types::tools::TaskStore` — dead code

**Estimated LOC change**: -~800 LOC (deleted stubs + duplicate types) + ~200 LOC (import changes + ContentBlock construction from core::Message) = net -600 LOC

### Phase 3: Unify Config (iter-116)

**Delete `adapter_types::config::Config` wrapper. Use `operant_core::config::AppConfig` directly.**

Changes:
- `App.config: Config` → `App.config: AppConfig`
- Move `provider`, `model`, `theme`, `permission_mode`, `output_style` to `Settings` (settings.json) only — don't duplicate them in Config
- `Config::from(AppConfig)` conversion → deleted
- `Settings::load_sync()` overlay → simplified (only reads settings.json, doesn't merge with Config)

**Estimated LOC change**: -~200 LOC (Config struct + From impl + conversion logic) = net -200 LOC

### Total Impact

| Phase | LOC deleted | LOC added | Net | Bugs eliminated |
|---|---|---|---|---|
| 1: Eliminate bridge | 111 | 50 | -61 | 6 bridge bugs |
| 2: Use core types | 800 | 200 | -600 | 4 stub bugs + all type drift |
| 3: Unify config | 200 | 0 | -200 | 1 config duplication bug |
| **Total** | **1,111** | **250** | **-861** | **11 bugs** |

## What Stays

These adapter_types types are legitimate TUI-only types that should stay:
- `Settings` (settings.json persistence) — TUI-specific
- `CostTracker` — TUI-specific
- `AuthStore` — TUI-specific credential management
- `VoiceRecorder` — TUI-specific audio
- `banner` — TUI-specific rendering
- `tips` — TUI-specific welcome screen
- `spinner` — TUI-specific animation
- `EffortLevel` / `EffortPickerState` — TUI-specific picker

## Verdict

The adapter/bridge layer is **pure technical debt**. It exists because of a porting history that was never reconciled. Every bug it has caused (thinking prefix, dropped messages, dropped iterations, stub McpManager, stub FileHistory, duplicate config) traces directly to the duplication. Eliminating it is the single highest-leverage refactor for TUI stability.
