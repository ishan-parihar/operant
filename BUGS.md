# Operant Bugs & Issues

**Last Updated**: 2026-06-19
**Status**: MVP Deployment Phase

---

## Critical (Blocks Deployment)

- [ ] **Compilation errors in test_phases_expanded.rs** - Multiple unresolved imports and missing functions (84+ errors)
- [ ] **Provider.rs lifetime mismatch** - `fetch_models` method has lifetime mismatch between trait and impl
- [ ] **Skin.rs .cloned() error** - Cannot call `.cloned()` on MutexGuard
- [ ] **Gateway test failure** - `test_gateway_start_stop_with_disabled_platforms` fails due to schema migration issue

---

## High (Affects Functionality)

- [ ] **Memory provider defaults not set** - TDG memory provider exists but not set as default in config
- [ ] **TTS provider defaults not set** - Kokoro TTS exists but not set as default in config
- [ ] **Web dashboard not copied** - React frontend from operant-agent not ported yet
- [ ] **Dashboard API endpoints missing** - Axum backend lacks REST API endpoints
- [ ] **WebSocket support missing** - No real-time event streaming in dashboard
- [ ] **Session resume/switch missing** - Cannot resume or switch sessions
- [ ] **File operations incomplete** - Missing patch tool, dedup, staleness detection
- [ ] **Tool executor missing** - No dispatch, error handling, retries
- [ ] **Turn context missing** - No message role alternation tracking
- [ ] **Delivery router missing** - No platform routing or truncation
- [ ] **Slash command dispatch missing** - Commands registered but not executed in gateway

---

## Medium (Enhancement)

- [ ] **Prompt caching not implemented** - No cache-aware prompts
- [ ] **Context compressor partial** - Auto-compaction not fully working
- [ ] **Conversation compression missing** - No manual /compress command
- [ ] **Auxiliary client missing** - No cheap side-LLM for summaries
- [ ] **Platform adapters incomplete** - Only 4 of 20+ ported
- [ ] **Memory provider implementations missing** - Honcho, Mem0, SuperMemory not implemented
- [ ] **Gemini provider missing** - Native adapter not implemented
- [ ] **Bedrock provider missing** - AWS SigV4 not implemented
- [ ] **Model catalog missing** - No model listing or switching
- [ ] **MCP catalog missing** - No MCP server catalog
- [ ] **Credential persistence missing** - OAuth tokens not persisted
- [ ] **TODO tool missing** - No task list management
- [ ] **Clarify tool missing** - No user clarification requests
- [ ] **URL safety missing** - No SSRF protection
- [ ] **Tool guardrails missing** - No risk-based tool approval
- [ ] **Schema memoization missing** - No per-call cache
- [ ] **Error sanitization missing** - No framing token stripping
- [ ] **Plugin hooks missing** - No pre/post tool call hooks
- [ ] **check_fn TTL missing** - No cached availability checks

---

## Low (Nice to Have)

- [ ] **Web dashboard not ported** - React SPA not copied from operant-agent
- [ ] **Desktop app missing** - No Electron app
- [ ] **i18n missing** - No internationalization
- [ ] **Batch processing missing** - No parallel batch runner
- [ ] **Trajectory saving missing** - No conversation export
- [ ] **Checkpoint system missing** - No filesystem snapshots
- [ ] **Background tasks missing** - No terminal background management
- [ ] **PTY support missing** - No interactive CLI tools
- [ ] **Sudo handling missing** - No password prompting
- [ ] **Environment persistence missing** - No shell state between calls
- [ ] **Malformed DB recovery missing** - No schema repair
- [ ] **Parent-child sessions missing** - No session chains
- [ ] **Token counting missing** - No usage tracking
- [ ] **Cost tracking missing** - No billing information
- [ ] **Session archiving missing** - No recursive lineage
- [ ] **Managed mode missing** - No NixOS/Homebrew support
- [ ] **Install detection missing** - No docker/nixos detection
- [ ] **Container/WSL/Termux detection missing** - No platform detection
- [ ] **Secure permissions missing** - No 0700/0600 files
- [ ] **Corruption backup missing** - No timestamped backups

---

## Notes

- **Compilation errors** are blocking test execution - must fix first
- **Web dashboard** is highest priority for visual parity
- **Memory/TTS providers** have config defaults but need to be set in operant.example.toml
- **Session management** is critical for gateway operation

---

*Created by Sisyphus - 2026-06-19*
