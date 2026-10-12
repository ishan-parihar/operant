# Operant MCP

- `[mcp] servers` in operant.toml: `name`, `transport` (`stdio`/`http`/`streamable-http`), `command`+`args`+`env` for stdio, `url`+`auth_token` for http, `deferred` flag, `enabled`
- `operant mcp list / add / remove / test / serve / login / configure` — manage servers
- Deferred servers (e.g. the injected agentmemory server) do NOT spawn at startup — they materialize tools on demand via `/mcp r` (TUI reconnect) or automatically when the agent loop connects them on first use
- `McpManager::sync_tools_to_registry` makes newly connected tools visible to the agent mid-session
- Native MCP server: `operant acp` / gateway exposes the agent as a server
