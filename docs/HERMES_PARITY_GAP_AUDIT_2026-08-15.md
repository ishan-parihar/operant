# Hermes-Agent Parity Gap Audit — Round 2

**Date:** August 15, 2026
**Scope:** New implementation gaps vs `hermes-agent` infrastructure **not** covered by
`HERMES_VS_OPERANT_AUDIT.md` (2026-07-23) or `AUDIT_2026-08-02.md`.
**Method:** File-level comparison of `hermes-agent/{tools,agent,hermes_cli,gateway}/` against
`operant/crates/` (core + cli + runtime + gateway + channels + providers). Every gap below was
verified by grep — 0 matches in the operant source for the feature's core symbol.
**Companion baseline:** Tier-1 infra (estop / cron suggestions / schedule normalization /
session cap) live-verified 16/16 PASS through the agentic loop on 2026-08-15.

---

## 1. Confirmed Gaps (operant missing)

### Tier 1 — Core loop & agent capability

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| G1 | **Auto session-title generation** | `agent/title_generator.py` (`generate_title`, `_persist_session_title`) | Operant stores titles (`database.rs:1495 update_session_title`, `memory.rs:107 Session.title`) but **never auto-generates** from the first user message (grep `generate_title|auto_title|first_user_message` → 0). Sessions stay generic until manually renamed. | 🟡 Medium — session list/show UX, trajectory hygiene |
| G2 | **Gateway reaction tool** | `tools/react_to_message_tool.py` (`react_to_message_tool(emoji, message_row_id, messages_back)`) + `agent/reactions.py` | Operant has `send_message_tool.rs` (text/media/webhook) but **no emoji-reaction API** on any platform (grep `react_to_message` → 0). | 🟢 Low-Med — Telegram/Discord/Slack expressiveness |
| G3 | **Verification harness** | `agent/verify/` (`environment.py`, `recipes.py`, `runner.py`) + `verification_evidence.py` (SQLite evidence ledger + retention) + `verification_stop.py` | **No verification tooling** (grep `verify_evidence|verification_evidence` → 0). Agent cannot self-verify an answer against a recipe/checklist. | 🟡 Medium — answer reliability on complex tasks |
| G4 | **MCP stdio watchdog** | `tools/mcp_stdio_watchdog.py` | Only manual `operant mcp restart` (CLI message string). **No auto-restart** of crashed/hung stdio MCP servers (grep `mcp_watchdog|stdio_watchdog` → 0). | 🟡 Medium — long-running agentic sessions lose MCP tools silently |
| G5 | **MoA (Mixture-of-Agents) turns** | `agent/moa_loop.py` + `moa_config.py` + `moa_trace.py` + `hermes_cli/moa_cmd.py` — `/moa` marks one turn; reference models generate context before each iteration | Only a setup-prompt mention (`post_setup.rs:155`). `auxiliary_models` powers **vision** (`vision_tool.rs:346`) and **memory review** (`agent/mod.rs:2518`) only — no reference-model aggregation. | 🟢 Low-Med — quality win on hard reasoning turns |

### Tier 2 — Gateway & session robustness

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| G6 | **Gateway lifecycle primitives** | `gateway/turn_lease.py`, `session_stall.py`, `drain_control.py`, `readiness.py`, `shutdown_flush.py`, `delivery_ledger.py`, `rich_sent_store.py`, `mirror.py` | Core gateway (`gateway/mod.rs`) has estop + session cap + handler, but **none** of: per-turn lease/ownership w/ stall detection, graceful drain on shutdown, delivery ledger (at-least-once/dedupe for platform sends), multi-channel mirroring (grep `turn_lease|session_stall|drain_control|delivery_ledger|rich_sent_store` → 0). | 🟡 Medium — long-running gateway reliability |
| G7 | **Session recap / recovery / filters** | `hermes_cli/session_recap.py`, `session_recovery.py`, `session_filters.py` | Operant has `cmd_sessions.rs` (list/show/search/export/prune/rename) but **no recap** (visible-turn counts, latest prompt/answer) and **no stale-session recovery** (grep `session_recap|session_recovery` → 0). | 🟢 Low-Med — ops ergonomics |
| G8 | **Delegation output schema + async delegation** | `tools/delegation_output_schema.py`, `tools/async_delegation.py` | `sub_agent_tool.rs` implements `delegate_task`/`spawn_subagent` + memory notification + TUI events, but **no JSON-schema-constrained subagent output** and **no async fire-and-forget delegation** (grep `output_schema|async_delegation` → 0). | 🟡 Medium — structured multi-agent results |

### Tier 3 — Security hardening

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| G9 | **Credential-file awareness** | `tools/credential_files.py` (`register_credential_file`, `get_credential_file_mounts`) | Operant has output redaction (`redaction.rs`), `pii.rs`, and context-file threat scanning (`context_files.rs`), but **no registry of protected credential files** the agent is told not to read/send (grep `credential_file` → only `operant-providers` Qwen OAuth path). | 🟡 Medium — agent-facing secret hygiene |
| G10 | **Environment exposure audit** | `tools/env_probe.py` | `env_passthrough.rs` only reloads `.env` (the old EnvPassthrough struct was deleted iter-126); **no proactive "which env vars look secret & are exposed" report**. Output scrubbing exists; pre-flight exposure audit does not. | 🟢 Low — redaction largely covers the risk |

---

## 2. Verified NOT Gaps (implemented since prior audits)

| Area | Evidence |
|------|----------|
| Prompt caching (`cache_control`) | `agent/clients/prompt_caching.rs` + `client.rs:531` + `operant-providers` anthropic/openrouter |
| Background review | `agent/background_review.rs` (ported from hermes), `write_origin.rs` guard |
| Streaming context scrubber | `agent/mod.rs:351` ("Ported from hermes-agent's StreamingContextScrubber pattern") |
| Error classifier / retry / fallback | `agent/error_classifier.rs` (ported), `agent/fallback.rs`, `operant-providers/reliable.rs` |
| Iteration budget + refund + turn retry state | `agent/iteration_budget.rs`, `agent/turn_retry_state.rs`, runtime R4 refund parity |
| Node detail / learning mutations | `agent/learning_graph.rs:161 node_detail`, wired via `learning_mutation_tool.rs` |
| Skills marketplace + open-skills sync | `skill_marketplace.rs`, `cmd_skills.rs` marketplace handlers, runtime `.operant-open-skills-sync` |
| Wake word / voice | `operant-channels/voice_wake.rs`, `voice.rs`, config `wake_word` |
| Webhooks (receive + send + audit) | `operant-gateway/lib.rs`, `send_message_tool.rs send_webhook`, runtime `webhook_audit` hook |
| Slash commands (TUI + gateway) | `commands.rs::tui_slash_commands`, `tui/input.rs`, `gateway_commands.rs::resolve_slash_command` |
| Tool-result truncation | `agent/mod.rs:3800 truncate_tool_result`, `max_tool_result_chars` config, runtime `history.rs` |
| Session tools / checkpoint / curator / dashboard / TTS / vision / interrupt | `session_search_tool.rs`, `checkpoint_tool.rs`, `curator/`, `cmd_dashboard.rs`, `tts_*.rs`, `vision_tool.rs`, `interrupt.rs` |
| URL safety (SSRF), threat patterns | `security.rs::check_url_safety` (DNS + IP class), `context_files.rs` + `skills_guard.rs` THREAT_PATTERNS |
| Sub-agent delegation (basic) | `sub_agent_tool.rs` (delegate_task/spawn_subagent) |
| Gateway channels | core: telegram/discord/slack/whatsapp; plus channels crate: line, wecom, clawdtalk, voice-call |

---

## 3. Redundancy / Drift Observations

1. **`HERMES_ENV_FILE` naming leak** — `env_passthrough.rs` reads `HERMES_ENV_FILE`, a hermes-agent name in operant code. Cosmetic, but contradicts the "operant-native" cleanup direction.
2. **Workspace doc drift** — the repo is now a multi-crate workspace (`operant-core`, `operant-cli`, `operant-runtime`, `operant-gateway`, `operant-channels`, `operant-providers`, `operant-config`, `operant-tools`, `operant-hardware`), while `AGENTS.md` still describes a two-crate layout. Docs should be updated to the real topology.
3. **Leftover idle operant processes** — old interactive runs (e.g. `--config /tmp/operant-live-test.toml`) can linger between sessions; a stale-process guard in `operant run` (refuse/single-instance) would prevent state contention on the same data dir.

---

## 4. Suggested Implementation Order

1. **G1 auto session titles** (small, self-contained; `database.rs` + `turn_finalizer.rs` hook) — best ROI.
2. **G4 MCP stdio watchdog** (small; restart loop around existing McpManager stdio transports).
3. **G3 verification harness** (medium; `verify/recipes`-style checklist + evidence SQLite, exposed as a tool).
4. **G8 delegation output schema** (medium; extend `sub_agent_tool.rs` with optional `output_schema` JSON and validation).
5. **G6 gateway lifecycle** (large; pick `turn_lease`+`drain_control` first — highest operational value).
6. **G2/G5/G7/G9/G10** — opportunistic, per user priority.
