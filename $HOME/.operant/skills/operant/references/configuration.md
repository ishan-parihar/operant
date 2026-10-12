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

## Changing configuration at runtime

Operant lets the agent reconfigure itself from inside the agentic loop — the
same capability hermes has via its `/config` slash commands. Two paths, same
typed coercion (bool → int → float → string, validated before applying):

- **`config_manage` tool** (agentic loop) — `get agent.model`, `set agent.model "gpt-4o"`,
  `show` (effective config as JSON with secrets masked), `path`, `reload`.
- **CLI** — `operant config set agent.model "gpt-4o"`, `operant config show`,
  `operant config path`, `operant config check` (validate for issues).

Rules:

1. **Runtime vs persisted.** `config_manage set` / `operant config set` apply
   immediately to the running session but do NOT rewrite the TOML file.
   Persist a change by writing the file at the `path` returned by
   `config_manage path` (e.g. via the `file_write`/`patch` tools) or by
   re-running `operant config set` — disk edits apply on next launch.
2. **Secrets never go in TOML.** API keys, tokens, and credentials live in
   `~/.operant/.env` only (same file hermes uses under `$HERMES_HOME`).
   Non-secret settings go in `operant.toml`. No exceptions.
3. **Validate before persisting.** Run `operant config check` or round-trip
   the value through `config_manage set` (it rejects invalid changes rather
   than silently accepting them) before writing the file.

Common self-adjustments: switching the active model
(`set agent.model`), toggling toolsets (`[tools] disabled_tools`), changing
memory provider or context engine (`[memory]`, `[agent] context_engine`), and
gateway channel settings (`[gateway]`).

## Env overrides (hermes-compatible)

`HERMES_HOME` (operant home), `HERMES_SKILLS_DIR` (skills dir), `HERMES_LOG_LEVEL`, `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPERANT_OPEN_SKILLS_DIR`, `OPERANT_BUNDLED_SKILLS_DIR`.
