# Operant MCP

- `[mcp] servers` in operant.toml: `name`, `transport` (`stdio`/`http`/`streamable-http`), `command`+`args`+`env` for stdio, `url`+`auth_token` for http, `deferred` flag, `enabled`
- `operant mcp list / connect / disconnect` — manage servers
- Deferred servers (e.g. the injected agentmemory server) do NOT spawn at startup — they materialize tools on demand via `/mcp r` (TUI reconnect) or `operant mcp connect`
- `McpManager::sync_tools_to_registry` makes newly connected tools visible to the agent mid-session
- Native MCP server: `operant acp` / gateway exposes the agent as a server
