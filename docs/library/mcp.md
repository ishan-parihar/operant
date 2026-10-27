# MCP — both sides of the protocol

Owns: operant's Model Context Protocol surfaces — the client
(HTTP/stdio/SSE, OAuth) and the server (`operant mcp serve`).
Not for: the sourcehound/AFT subprocess bridges (they *use* MCP-shaped
stdio but are integrations — [web-tools.md](web-tools.md),
[tools.md](tools.md)), plugins ([plugins.md](plugins.md)).

Source: `crates/operant-core/src/mcp.rs` (client),
`crates/operant-core/src/mcp_oauth.rs` (server OAuth),
`crates/operant-cli/src/mcp_serve.rs` (server),
`crates/operant-cli/src/cmd_mcp.rs` (management CLI),
`crates/operant-core/src/tools/mcp_tool.rs` (tool adapter).

## The client (`mcp.rs`)

Connects external MCP servers to extend the agent with tools and
resources. Three transports:

- **stdio** — spawned subprocess, JSON-RPC over stdin/stdout; sampling
  and elicitation (server-initiated requests) are handled (iter-66).
- **HTTP** (streamable) — remote servers.
- **SSE** — `McpSseClient` with a background reader task + oneshot
  response routing (iter-68 — the last hermes parity gap closed).

External tools are adapted to `OperantTool` and registered like
builtins (`tools/mcp_tool.rs`). Tool-group exposure is filtered by
`*`-only globs (`crates/operant-api/src/glob.rs`, one matcher for the
loop-support and webhook-audit call sites). Deferred servers exist:
an operant invocation never spawns a server the user has not actually
connected.

## The server (`mcp_serve.rs`)

Operant AS an MCP server over stdio: any MCP client (Claude Desktop, VS
Code, …) can discover and call operant's tools. Reads JSON-RPC request
lines from stdin, writes responses to stdout, full lifecycle
(initialize → tools/list → tools/call). Tool discovery walks the same
registry the agent uses — one toolset, served both ways. This surface
is why README's "MCP client and server" row is true — the client lives
in `operant-core/src/mcp.rs`, the server here (the iter-1000 reversion
story: a too-narrow search removed "and server" from the README once).

## OAuth (`mcp_oauth.rs`)

The MCP-server OAuth surface — a separate code path from the
provider-account OAuth (`oauth/`, [providers.md](providers.md)):
authorization-code + PKCE and device flows for servers that demand
them, with stored tokens.

## Management

`operant mcp list/add/remove/test/serve` (`cmd_mcp.rs`) — the operator
surface; `operant doctor` probes configured servers. The TUI's MCP
panel (`tui/mcp_view.rs`) is the interactive view.

## Intention

MCP is the interoperability bet: rather than N bespoke integrations,
one protocol surface lets operant both consume external capability and
be consumed. The client/server split is deliberate — each direction was
built for a different reason (extend the agent; let other agents use
this one) but shares the tool registry, so a tool registered for the
agent is automatically servable, and a tool learned from a server is
just a tool.
