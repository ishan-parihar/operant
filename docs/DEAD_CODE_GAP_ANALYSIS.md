# Operant Dead Code & Implementation Gap Analysis

**Date:** July 23, 2026  
**Author:** Buffy (AI Agent)  
**Scope:** Full operant workspace vs hermes-agent reference implementation  
**Method:** Static analysis of `#[allow(dead_code)]` annotations, cargo warnings, and cross-reference with hermes-agent

---

## Executive Summary

| Metric | Count |
|--------|-------|
| `#[allow(dead_code)]` annotations | **197** |
| `#[allow(unused_*)]` annotations | **45** |
| **Combined suppressed warnings** | **242** |
| Unique crates affected | **10** |
| Cargo build dead_code warnings (unsuppressed) | **2** |

The codebase has **242 instances** where dead code warnings are explicitly suppressed. These fall into **6 categories** with different recommended actions. The largest categories are tool argument structs (necessary), learning graph mutations (need wiring), and MCP infrastructure (need completion).

---

## Dead Code by Crate

### `#[allow(dead_code)]` Breakdown

| Crate | Count | Severity |
|-------|-------|----------|
| `operant-core` | 52 | 🔴 High — core logic |
| `operant-cli` | 42 | 🟡 Medium — TUI/CLI |
| `operant-gateway` | 42 | 🟡 Medium — gateway |
| `operant-channels` | 27 | 🟡 Medium — channels |
| `operant-providers` | 13 | 🟡 Medium — providers |
| `operant-tools` | 9 | 🟢 Low — tools |
| `operant-runtime` | 6 | 🟢 Low — runtime |
| `operant-memory` | 4 | 🟢 Low — memory |
| `operant-infra` | 1 | 🟢 Low — infra |
| `operant-plugins` | 1 | 🟢 Low — plugins |

### `#[allow(unused_*)]` Breakdown

| Crate | Count | Type |
|-------|-------|------|
| `operant-runtime` | 26 | imports, mut, variables |
| `operant-memory` | 6 | imports |
| `operant-hardware` | 4 | imports, mut |
| `operant-channels` | 3 | imports, mut |
| `operant-providers` | 3 | imports |
| `operant-core` | 2 | assignments |
| `operant-config` | 1 | mut |

---

## Category Analysis

### Category A: Tool Argument Structs (🟢 LOW RISK — Keep as-is)
**~60 instances** across tool files

These are `#[derive(JsonSchema, Deserialize)]` structs for tool input arguments. The compiler can't detect runtime deserialization usage, so `#[allow(dead_code)]` is **correct and necessary**. These are wired up via the tool registry at runtime.

**Files:**
- `tools/notification_tool.rs` — `NotifyArgs`
- `tools/process_tool.rs` — `ProcessToolArgs`
- `tools/send_message_tool.rs` — `SendMessageArgs`
- `tools/spotify_tool.rs` — `QueueArgs`, `DevicesArgs`, `SearchArgs`
- `tools/session_search_tool.rs` — `SessionSearchArgs`
- `tools/sub_agent_tool.rs` — `DelegationTask`, `SubAgentArgs`
- `tools/feishu_tool.rs` — `FeishuDocArgs`, `FeishuDriveArgs`
- `tools/skills_tool.rs` — `SkillsListArgs`, `SkillViewArgs`
- `tools/browser_cdp_tool.rs` — `BrowserCdpArgs`
- `tools/home_assistant_tool.rs` — `HomeAssistantArgs`
- `tools/computer_use_tool.rs` — `CuaArgs`
- `tools/checkpoint_tool.rs` — `CheckpointArgs`
- `tools/tool_backend_helpers.rs` — `ToolBackendArgs`
- `tools/debug_helpers.rs` — `EchoArgs`, `CalcArgs`

**Recommendation:** ✅ **No action needed.** These are struct definitions used via serde at runtime.

---

### Category B: Learning Graph Mutation Functions (✅ RESOLVED)

**Status:** ✅ **Already wired up** via `LearningMutationTool` (`learning_manage` tool).

The `LearningMutationTool` in `tools/learning_mutation_tool.rs` wraps `delete_node` and `edit_node` from `learning_graph.rs` and is registered in `tools/builtin.rs` via `register_builtin_tools()`. The tool supports `delete` and `edit` actions for both skill and memory nodes.

**Verified:** Code review confirmed complete. No action needed.

---

### Category C: MCP Infrastructure (🟡 MEDIUM RISK — Complete or remove)
**~45 instances** across MCP-related files

| File | Items | Purpose | hermes-agent Equivalent |
|------|-------|---------|------------------------|
| `mcp.rs` | `McpClient`, `McpCapabilities`, `McpToolDefinition`, `McpStdioClient`, `McpSseClient` | MCP client types | `acp_adapter/server.py`, `acp_adapter/session.py` |
| `mcp_oauth.rs` | `OAuthError`, `OAuthToken`, `OAuthClientInfo`, `OAuthMetadata`, `MpOAuthConfig`, `TokenStorage` | OAuth for MCP | `acp_adapter/entry.py` |
| `mcp_tool.rs` | `McpManagementTool` | MCP management tool | `transports/hermes_tools_mcp_server.py` |
| `misc/schema.rs` | `GenerateSchema`, `GenerateInput` | JSON Schema generation | Various schema utilities |

**hermes-agent Reference:** `agent/transports/hermes_tools_mcp_server.py` has `_build_server()`, `_make_handler()`, `_dispatch()` for full MCP server support.

**Gap:** Operant has MCP client infrastructure but the **server-side MCP** (exposing operant's tools to external clients) is incomplete. The hermes-agent exposes its tools via MCP for external clients.

**Recommendation:** 🟡 **Complete or remove.** If MCP server support is planned, connect `_build_server()` equivalent. Otherwise, remove unused types.

---

### Category D: TUI Helper Methods (🟡 MEDIUM RISK — Wire up or remove)
**~30 instances** across TUI files

| File | Items | Purpose | hermes-agent Equivalent |
|------|-------|---------|------------------------|
| `voice_mode_notice.rs` | `update_voice_enabled`, `dismiss` | Voice mode UI | `agent/transcription_provider.py` |
| `notifications.rs` | `NotificationKind`, `Notification`, `NotificationQueue` | UI notifications | `agent/display.py` |
| `image_paste.rs` | `PastedImage`, clipboard functions | Clipboard images | No direct equivalent |
| `slash_usage.rs` | `UsageStat`, `UsageStore` | Command tracking | `gateway/slash_commands.py` |
| `stats_dialog.rs` | Model usage statistics | Stats display | `agent/insights.py` |
| `transcript_turn.rs` | Transcript rendering | Conversation display | `tui_gateway/render.py` |
| `messages/mod.rs` | Message rendering helpers | UI message display | `tui_gateway/render.py` |
| `dialogs.rs` | Dialog state management | UI dialogs | No direct equivalent |
| `overlays.rs` | History search, model usage stats | UI overlays | No direct equivalent |

**hermes-agent Reference:** `tui_gateway/render.py` handles all TUI rendering, `agent/display.py` handles notifications and display updates.

**Gap:** Operant has TUI helpers implemented but some are **not connected** to the TUI event system. The hermes-agent has a more integrated rendering pipeline.

**Recommendation:** 🟡 **Wire up or remove.** Connect voice mode, notifications, and stats dialogs to the TUI event system. Remove if features aren't planned.

---

### Category E: Agent Infrastructure (🟡 MEDIUM RISK — Wire up or document)
**~15 instances** across agent files

| File | Items | Purpose | hermes-agent Equivalent |
|------|-------|---------|------------------------|
| `turn_finalizer.rs` | 1 item | Turn finalization logic | `agent/turn_finalizer.py` |
| `background_review.rs` | 2 items | Background code review | `agent/background_review.py` |
| `llm_compressor.rs` | 1 item | LLM compression | `agent/context_compressor.py` |
| `turn_context.rs` | 1 item | `NotificationMode` enum | `agent/turn_context.py` |
| `insights.rs` | 1 item | Insights extraction | `agent/insights.py` |
| `message_safety.rs` | 2 items | `sanitize_surrogates`, `sanitize_messages_surrogates` | `agent/message_sanitization.py` |
| `mod.rs` | 3 items | Test helpers | Various test utilities |

**hermes-agent Reference:**
- `agent/turn_finalizer.py` has `_is_pure_tool_call_tail()`, `_drop_verification_continuation_scaffolding()`, `finalize_turn()`
- `agent/background_review.py` has `summarize_background_review_actions()`, `build_memory_write_metadata()`, `spawn_background_review_thread()`
- `agent/insights.py` has `InsightsEngine` with full analytics

**Gap:** Operant has these modules implemented but some functions are **not called** from the main agent loop. The hermes-agent integrates them more deeply.

**Recommendation:** 🟡 **Mixed.**
- `message_safety.rs` no-ops: ✅ **Keep** — API parity with hermes-agent
- Test helpers: ✅ **Keep** — legitimate test utilities
- `background_review.rs`, `insights.rs`, `turn_context.rs`: 🔴 **Wire up or remove** — features not yet connected

---

### Category F: Provider-Specific Code (🟢 LOW RISK — Keep or conditionally compile)
**~13 instances** in `operant-providers`

| File | Items | Purpose |
|------|-------|---------|
| `azure_openai.rs` | 2 items | Azure-specific auth types |
| `bedrock.rs` | 3 items | AWS Bedrock auth types |
| `compatible.rs` | 1 item | Compatible provider types |

**Recommendation:** 🟢 **Keep** — These are provider-specific types that are used conditionally based on the configured provider.

---

## Unsolved Cargo Warnings

| Warning | Location | Action |
|---------|----------|--------|
| `unused import: warn` | `operant-core` | Remove unused import |
| `unused variable: old_significator` | `operant-core` | Remove or prefix with `_` |

---

## Workspace Lint Results

| Check | Result |
|-------|--------|
| Errors | 0 (fixed git tracking issue) |
| Warnings | 0 |
| Info | 11 (orphaned root files — expected for Rust project) |

**Fixed:** Removed `__pycache__/workspace_lint.cpython-314.pyc` from git tracking.

---

## Priority Action Plan

### Phase 1: Quick Wins (Remove obvious dead code)
1. Remove unused `warn` import in operant-core
2. Remove unused `old_significator` variable in operant-core
3. Remove the 2 remaining test helpers in `agent/mod.rs` (if not used in tests)

### Phase 2: Learning Graph ✅ DONE
4. ~~Wire up `learning_graph.rs` mutation functions to the tool system~~ — Already done via `LearningMutationTool`

### Phase 3: MCP Infrastructure (Wire up or remove)
6. **Decision needed:** Complete MCP server support, or remove the unused protocol types
7. If wiring up: Connect `McpClient`, `McpCapabilities`, etc. to the MCP server

### Phase 4: TUI Features (Wire up or remove)
8. Wire up voice mode, notifications, image paste, and stats dialogs to the TUI event system
9. Or remove if these features are not planned for operant

### Phase 5: Background Features (Wire up or remove)
10. Wire up `background_review.rs` and `insights.rs` to the agent loop
11. Or remove if these features are not planned

---

## Scale Summary

| Category | Instances | Risk | Action |
|----------|-----------|------|--------|
| Tool argument structs | ~60 | 🟢 Low | Keep (serde runtime usage) |
| Learning graph mutations | 7 | 🔴 High | Wire up or remove |
| MCP infrastructure | ~45 | 🟡 Medium | Wire up or remove |
| TUI helpers | ~30 | 🟡 Medium | Wire up or remove |
| Agent infrastructure | ~15 | 🟡 Medium | Wire up or remove |
| Provider-specific | ~13 | 🟢 Low | Keep (conditional usage) |
| Message safety no-ops | 2 | 🟢 Low | Keep (API parity) |
| Unsolved cargo warnings | 2 | 🟢 Low | Remove |
| **Total** | **~242** | | |

---

## Implementation Iterations

### Iteration 1: Cleanup (Low Risk)
- Remove unused imports and variables
- Remove obviously dead test helpers
- Fix workspace lint issues

### Iteration 2: Learning Graph (Medium Risk)
- Wire up mutation functions as tools
- Add tests for tool integration
- Verify no regressions

### Iteration 3: MCP Completion (Medium Risk)
- Complete MCP server infrastructure
- Connect to external client support
- Add integration tests

### Iteration 4: TUI Integration (Medium Risk)
- Wire up voice mode, notifications, stats
- Connect to TUI event system
- Test all UI interactions

### Iteration 5: Agent Infrastructure (High Risk)
- Wire up background review
- Connect insights engine
- Integrate with agent loop

---

## References

- hermes-agent: `./hermes-agent/` (Python reference implementation)
- operant: `./operant/` (Rust port being built)
- Workspace lint config: `operant/workspace-lint.yaml`
