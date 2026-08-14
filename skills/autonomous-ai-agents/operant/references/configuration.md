# Operant Configuration

Config lives at `~/.operant/operant.toml`; secrets live in `~/.operant/.env` only.

## Key sections

- `[client]` — base_url, api_key, timeout, additional_api_keys (credential rotation)
- `[agent]` — model, fallbacks, max_iterations, tool_timeout_secs, context_window, stream, context_engine
- `[memory]` — provider: `agentmemory` (default, hybrid BM25+vector+graph, auto-spawns via npx) or `builtin` (MEMORY.md/USER.md files); `enabled`
- `[tools]` — igs_enabled, aft_enabled, disabled_tools, disabled_toolsets, browser provider
- `[skills]` — root_dir (default `~/.operant/skills`), autoload, template_vars
- `[mcp]` — servers (stdio/http, deferred flag), autoload
- `[gateway]` — telegram/discord/slack/whatsapp tokens + enable flags
- `[checkpoints]` — opt-in snapshot store
- `[credential_pool]` — multi-key rotation

## Memory provider

- `agentmemory` (default): hybrid search; session/start on init, observe after every turn, recall before turns. Server auto-spawns (`npx @agentmemory/mcp`); graceful degradation to builtin when node is missing.
- `builtin`: file-backed MEMORY.md/USER.md via `memory_store` / `memory_search` / `memory_recall`.

## Gateway channels

`operant gateway start` runs the messaging gateway. `operant channel` configures each platform. Delivery uses the same agent loop with full tool access, not just chat.

## Env overrides (hermes-compatible)

`HERMES_HOME` (operant home), `HERMES_SKILLS_DIR` (skills dir), `HERMES_LOG_LEVEL`, `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPERANT_OPEN_SKILLS_DIR`, `OPERANT_BUNDLED_SKILLS_DIR`.
