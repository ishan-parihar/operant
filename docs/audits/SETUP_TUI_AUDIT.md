# Setup & TUI-Backend Integration Audit

**Date**: 2026-07-06
**Scope**: `operant setup` flow vs hermes-agent, config source proliferation, TUI-backend integration bugs

## Executive Summary

Operant has **9 config sources in 3 formats** (YAML, TOML, JSON) + env vars. Hermes has **3 sources in 2 formats** (YAML, .env). The config layering is the root cause of the "setup doesn't remember my config" bug — the same provider+model can be stored in 3 different files with unclear priority.

The setup wizard has **6 steps in full mode, 3 in quick mode** — but even quick mode has redundant sub-steps (fallback keys, rotation strategy, auxiliary models) that should only appear in `--reconfigure` mode. The setup flow also has a **config-write bug**: it writes to `~/.operant/operant.toml` but the TUI reads from `settings.json` first, creating a mismatch.

The TUI-backend integration has **5 remaining issues** after the bridge elimination (iter-114): duplicate Message types, duplicate Config wrapper, stub McpManager, 3-file config conflict, and hardcoded Config defaults that ignore AppConfig values.

## Section A: Setup Flow Audit

### A1: Config Source Proliferation (BLOCKER)

| # | Source | Format | Written by | Read by | Priority |
|---|--------|--------|------------|---------|----------|
| 1 | `~/.operant/config.yaml` | YAML | CliConfig (legacy) | main.rs CliConfig::load() | 1 (highest) |
| 2 | `~/.operant/.env` | dotenv | CliConfig | main.rs load_dotenv_file() | 2 |
| 3 | `HERMES_*` / `OPENAI_*` env vars | env | shell | main.rs apply_operant_env_overrides() | 3 |
| 4 | `~/.operant/operant.toml` | TOML | operant setup | main.rs load_app_config() | 4 |
| 5 | `~/.operant/settings.json` | JSON | TUI /connect, /model | TuiApp::enter | 5 (TUI only) |
| 6 | `~/.operant/auth.json` | JSON | TUI /connect | TuiApp::enter AuthStore | 6 (TUI only) |

**Problem**: Provider+model can be stored in sources 1, 4, AND 5. The priority is:
- `operant setup` writes to #4 (operant.toml) + syncs to #5 (settings.json)
- TUI reads #5 (settings.json) and overlays on #4 (operant.toml via Config::from)
- But main.rs merges #1 (config.yaml) into #4 BEFORE the TUI sees it
- If config.yaml has `model: gpt-4`, it overrides the operant.toml model

**Fix**: Consolidate to 2 sources:
1. `~/.operant/operant.toml` (TOML) — the single config file (replace config.yaml)
2. `~/.operant/.env` — API keys only (keep as-is)

Delete settings.json as a config source. The TUI should read/write operant.toml directly. auth.json stays for credential management.

### A2: Setup Flow Steps (HIGH)

| Step | Quick mode | Full mode | Hermes equivalent | Issue |
|------|-----------|-----------|-------------------|-------|
| Provider selection | ✅ | ✅ | TUI /model (no wizard) | OK |
| Model selection | ✅ | ✅ | TUI /model | OK |
| API key (K/R/C) | ✅ | ✅ | .env file | OK |
| Fallback & rotation | ❌ (skipped) | ✅ | None | Power-user — should be `--reconfigure` only |
| Auxiliary models | ❌ (skipped) | ✅ | None | Power-user — should be `--reconfigure` only |
| Gateway platforms | ❌ (skipped) | ✅ | config.yaml | OK but verbose |
| Terminal prefs | ❌ (skipped) | ✅ | config.yaml | OK |
| Tools & behaviour | ❌ (skipped) | ✅ | config.yaml | OK |
| TTS | ❌ (skipped) | ✅ | config.yaml | OK |
| Browser & skills | ❌ (skipped) | ✅ | None | OK |
| Agent behaviour | ❌ (skipped) | ✅ | config.yaml | OK |
| Post-setup summary | ✅ | ✅ | None | Too verbose — should be 3 lines |

**Problem**: The user reported "a lot of redundant options" — the full wizard asks 11+ questions. Hermes doesn't have a wizard at all; it uses the TUI's `/model` command for provider setup and config.yaml for everything else.

**Fix**: 
- Quick mode: provider → model → API key (3 steps, already done in iter-112)
- Full mode (`--reconfigure`): all steps, but with smart defaults from existing config
- Add `operant setup --section <name>` for individual section reconfiguration (already exists)
- Post-setup: trim to 3 lines (config saved, run `operant chat`, run `operant setup --reconfigure` for full config)

### A3: Setup Config Persistence (BLOCKER — already fixed in iter-112)

| Issue | Status | Fix |
|-------|--------|-----|
| persist_config wrote to CWD operant.toml instead of ~/.operant/operant.toml | ✅ Fixed (iter-112) | Always write to ~/.operant/operant.toml |
| TUI settings.json overrode operant.toml unconditionally | ✅ Fixed (iter-112) | settings.json only overrides when TOML has defaults |
| Setup didn't sync to settings.json | ✅ Fixed (iter-112) | sync_to_settings_json() called after persist_config |

### A4: Setup vs Hermes Comparison

| Dimension | Operant | Hermes | Gap |
|-----------|---------|--------|-----|
| Setup mechanism | Interactive wizard (1517 LOC) | Shell script + TUI /model (462 LOC + TUI) | Operant's wizard is overengineered |
| Config format | TOML + YAML + JSON + .env | YAML + .env | Operant has 3 formats — should be 1 |
| Config location | ~/.operant/operant.toml + config.yaml + settings.json | ~/.hermes/config.yaml | Operant has 3 files — should be 1 |
| Provider switching | Setup wizard OR TUI /connect | TUI /model only | Operant has 2 paths — confusing |
| API key management | .env + auth.json + settings.json | .env only | Operant has 3 stores — should be 1 |
| First-run experience | Auto-opens /connect (iter-108) | TUI /model prompt | Equivalent |

## Section B: TUI-Backend Integration Audit

### B1: Duplicate Message Type (HIGH)

| adapter_types::types::Message | operant_core::client::Message | Divergence |
|---|---|---|
| `role: Role` | `role: Role` | Identical (duplicated enum) |
| `content: MessageContent` (enum: Text/Blocks) | `content: String` | Incompatible — TUI can't hold flat text + reasoning |
| No `reasoning` field | `reasoning: Option<String>` | TUI misses reasoning |
| No `tool_calls` field | `tool_calls: Option<Vec<ToolCall>>` | TUI can't represent tool calls natively |
| No `tool_call_id` field | `tool_call_id: Option<String>` | TUI can't represent tool results |

**Impact**: The TUI's `flush_streamed_assistant_message` manually constructs ContentBlocks from `streaming_text` + `streaming_thinking` because it can't use core::Message directly. This is fragile and was the source of the "thinking content as [thinking] text" bug (iter-113).

**Fix**: Phase 2 of BACKEND_TUI_AUDIT.md — delete adapter_types::types::Message, use core::client::Message directly. Keep ContentBlock as a TUI-only rendering type built FROM core::Message.

### B2: Duplicate Config Wrapper (HIGH)

`adapter_types::config::Config` wraps `AppConfig` with 14 duplicate fields:
- `provider`, `model`, `theme`, `permission_mode`, `output_style` — all exist in both Config AND Config.inner (AppConfig)
- `file_autocomplete_limit`, `file_injection_max_size`, `compact_threshold`, `max_tokens` — hardcoded in Config::from() and Config::default(), ignoring AppConfig values

**Impact**: Two sources of truth for provider+model. The TUI's `config.provider` can disagree with `config.inner.client.base_url`. The hardcoded defaults (max_tokens: 8192, compact_threshold: 0.8) ignore what the user configured in operant.toml.

**Fix**: Phase 3 of BACKEND_TUI_AUDIT.md — delete Config wrapper, use AppConfig directly. Move TUI-only fields (theme, permission_mode, output_style) to Settings.

### B3: Stub McpManager (MEDIUM)

`adapter_types::mcp::McpManager` is a stub:
- `all_tool_definitions()` → `vec![]`
- `server_status()` → `Disconnected`
- `server_catalog()` → `None`

The real `operant_core::mcp::McpManager` is attached separately via `core_mcp_manager` (iter-93). The TUI's `mcp_manager` field holds the stub and is used by `load_mcp_servers()` in app.rs — which returns empty data.

**Impact**: `/mcp` shows no tools and all servers as Disconnected, even when the agent is actively using MCP tools.

**Fix**: Delete the stub. Make `mcp_manager` field use `Option<Arc<operant_core::mcp::McpManager>>`. Wire `load_mcp_servers()` to call the real McpManager's `server_names()` + `all_servers()`.

### B4: Settings.json vs operant.toml Conflict (BLOCKER)

The TUI reads `settings.json` and overlays it on the TOML config. But `operant setup` writes to operant.toml AND syncs to settings.json. If the user manually edits operant.toml, settings.json becomes stale and overrides it.

**Current mitigation** (iter-112): settings.json only overrides when TOML has default values (`model == "gpt-4"`). But this is a heuristic — if the user sets model to "gpt-4" intentionally, settings.json won't override it even if it should.

**Fix**: Eliminate settings.json as a config source. The TUI should read/write operant.toml directly for provider+model. Settings.json should only hold TUI-specific prefs (theme, vim, effort_level).

### B5: Config::from() Hardcodes Defaults (MEDIUM)

```rust
impl From<AppConfig> for Config {
    fn from(inner: AppConfig) -> Self {
        Self {
            file_autocomplete_limit: 50,    // hardcoded — ignores inner.tui.*
            file_injection_max_size: 1024,  // hardcoded
            compact_threshold: 0.8,         // hardcoded — ignores inner.agent.*
            max_tokens: 8192,               // hardcoded
            ...
        }
    }
}
```

**Impact**: User configures `max_tokens = 32000` in operant.toml, but the TUI uses 8192 because Config::from() hardcodes it.

**Fix**: Read from `inner` (AppConfig) instead of hardcoding.

## Section C: Prioritized Fix List

| # | Fix | Priority | Effort | Impact |
|---|-----|----------|--------|--------|
| 1 | Consolidate config to operant.toml + .env only | P0 | ~400 LOC | Eliminates config confusion permanently |
| 2 | Delete settings.json as provider+model source | P0 | ~100 LOC | TUI reads operant.toml directly |
| 3 | Delete adapter_types::types::Message, use core::Message | P1 | ~300 LOC | Eliminates Message type drift |
| 4 | Delete Config wrapper, use AppConfig directly | P1 | ~200 LOC | Eliminates config field duplication |
| 5 | Delete stub McpManager, use core McpManager | P1 | ~50 LOC | /mcp shows real data |
| 6 | Fix Config::from() hardcoded defaults | P1 | ~20 LOC | TUI respects user config |
| 7 | Trim post-setup summary to 3 lines | P2 | ~30 LOC | Less verbose |
| 8 | Make CliConfig (config.yaml) read-only (deprecated) | P2 | ~50 LOC | Migration path to TOML-only |

## Verdict

The setup flow works (iter-112 fixed the persistence bug), but the architecture is fragile because of 6 config sources. The TUI-backend integration has 5 remaining issues after the bridge elimination — the biggest is the duplicate Config wrapper with hardcoded defaults. The fix is Phase 2+3 of BACKEND_TUI_AUDIT.md: use core types directly and eliminate the Config wrapper.
