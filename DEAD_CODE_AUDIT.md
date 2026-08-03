# Dead Code Audit Report

Generated: $(date)
Total #[allow(dead_code)] annotations: 63

## Category 1: KEEP (API Parity / Provider-Specific / Deserialization) — 39 items

These are false positives or intentional API parity with hermes-agent.

### Tool Args Structs (used via serde deserialization — compiler can't detect usage)
- spotify_tool.rs: DevicesArgs, QueueArgs, SearchArgs (3)
- feishu_tool.rs: FeishuDocArgs, FeishuDriveArgs (2)
- notification_tool.rs: NotifyArgs, ApprovalRequestArgs (2)
- skills_tool.rs: SkillsListArgs, SkillViewArgs (2)
- sub_agent_tool.rs: DelegationTask, SubAgentArgs (2)
- browser_cdp_tool.rs: BrowserCdpArgs (1)
- checkpoint_tool.rs: CheckpointArgs (1)
- computer_use_tool.rs: CuaArgs (1)
- home_assistant_tool.rs: HomeAssistantArgs (1)
- process_tool.rs: ProcessToolArgs (1)
- send_message_tool.rs: SendMessageArgs (1)
- session_search_tool.rs: SessionSearchArgs (1)
- tool_backend_helpers.rs: ToolBackendArgs (1)

Total: 19 tool args structs

### TUI Infrastructure (used via external calls / state management)
- adapter_types.rs: Message::user, ensure_provider_defaults, Task, Thinking.signature (4)
- device_auth_dialog.rs: interval field, DeviceAuthEvent (2)
- image_paste.rs: PastedImage fields (2)
- session_browser.rs: selected_session, stats fields (2)
- notifications.rs: dismiss, tick, current (2)
- overlays.rs: Error variant, debug helpers (2)
- stats_dialog.rs: model usage stats fields (2)
- voice_mode_notice.rs: dismiss, update_voice_enabled (2)
- diff_viewer.rs: set_turn_diff (1)
- mcp_view.rs: MCP connection status (1)
- prompt_input.rs: input helpers (1)
- debug_helpers.rs: debug utilities (1)

Total: 20 TUI infrastructure items

## Category 2: WIRE UP (Functional Gaps) — 4 items → 0 remaining

Ported from hermes-agent. All four resolved in Phase 10 (8797c093): verified
live in production and the stale `#[allow(dead_code)]` suppressions removed.

- turn_finalizer.rs: file_mutation_verifier_footer (1) ✅ WIRED — called at agent/mod.rs:1655
- background_review.rs: NotificationMode enum (1) ✅ WIRED — compared in summarize_review_actions
- insights.rs: SessionRow.id field (1) ✅ WIRED — read by compute_tool_breakdown_from_db (insights.rs:375)
- llm_compressor.rs: summary_text field (1) ✅ TEST SEAM — carries `#[expect(dead_code, reason = "exposed for tests/inspection of the compression outcome")]`

## Category 3: REMOVE (YAGNI) — 20 items

These are genuinely unused with no clear future use.

- message_safety.rs: sanitize_surrogates, sanitize_messages_surrogates (2) — documented no-ops
- slash_usage.rs: recency_rank, frequency_rank, rank_commands (3) — test-only helpers
- transcript_turn.rs: helper methods (3)
- dialogs.rs: dialog helpers (3)
- skills_tool.rs: unused fields (2)
- sub_agent_tool.rs: unused fields (2)

Total: 15 items to remove
