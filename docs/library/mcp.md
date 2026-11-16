# MCP — both sides of the protocol

Owns: operant's Model Context Protocol surfaces — the client (plain
HTTP, streamable HTTP, stdio, SSE; OAuth 2.1 + PKCE) and the server
(`operant mcp serve`).
Not for: the sourcehound / AFT subprocess bridges (they ride MCP-shaped
stdio but are integrations — [web-tools.md](web-tools.md),
[tools.md](tools.md)), WASM plugins ([plugins.md](plugins.md)), the
tool-search deferred bridge ([tools.md](tools.md)), provider-account
OAuth ([providers.md](providers.md)).

Source: `crates/operant-core/src/mcp.rs` (the clients and manager),
`crates/operant-core/src/mcp_oauth.rs` (OAuth 2.1 + PKCE for servers),
`crates/operant-cli/src/mcp_serve.rs` (the server),
`crates/operant-cli/src/cmd_mcp.rs` (the management CLI),
`crates/operant-core/src/tools/mcp_tool.rs` (`mcp_management`),
`crates/operant-core/src/config.rs` (`[mcp]` + `McpServerConfig`).

## The client (`mcp.rs`)

Four transports, three client types:

| kind | client | how it speaks |
|---|---|---|
| **stdio** | `McpStdioClient` | spawns `<command> <args…>` with piped stdin/stdout/stderr; newline-delimited JSON-RPC |
| **plain HTTP** | `McpClient::new` | POST + JSON-RPC; `Accept: application/json` |
| **streamable HTTP** | `McpClient::streamable` | POST + JSON-RPC; `Accept` also offers `text/event-stream`, echoes `Mcp-Session-Id`, reads `data:` lines out of an SSE-shaped response |
| **SSE** (legacy) | `McpSseClient` | long-lived GET; server's `endpoint` event names the POST URL; responses arrive on the stream |

The plain-HTTP and streamable-HTTP clients are the SAME struct with a
constructor flag, because they differ in exactly three ways: the `Accept`
header (a streamable server answers `406 Not Acceptable` to a request
that does not offer `text/event-stream`), the `Mcp-Session-Id` echo
(without it every request starts a new session and the handshake never
completes), and the response body shape. A `McpTransport` enum wraps the
three for the manager.

**Version negotiation.** `MCP_SUPPORTED_VERSIONS = ["2025-06-18",
"2024-11-05"]`; the client sends the newest and accepts the server's
answer if it matches exactly or by prefix (`2025-06-18-draft`). An
unsupported version is a warning, not a failure — the connection
continues at the newest supported.

**Namespacing.** A server's tools enter the registry as
`mcp_<server>-<tool>` with a LOSSLESS escape (`-` → `--`, `/` → `//`, the
two components joined by one `-`), because the registry's `register`
REPLACES on name collision and the teardown removes BY NAME. The old
naive join was not injective: server `a` + tool `b_c` and server `a_b` +
tool `c` both produced `mcp_a_b_c`, so registering the second deleted the
first's live tool. An escaped run of `-` is always even, so the join is
unambiguous; `github` + `create_issue` stays readable as
`mcp_github-create_issue`.

**stdio robustness.** `McpStdioClient` carries a lot of state because a
stdio server is a hostile-ish child:

- `kill_on_drop(true)` so a dropped transport never orphans a child
  (tokio's default is `false`).
- stderr is **drained**, not piped and forgotten (a chatty server fills
  the 64 KB OS buffer and blocks on its next write).
- non-JSON stdout lines (a startup banner, a status line) are skipped up
  to `MCP_BANNER_SKIP_LIMIT` (256) before the call fails.
- the first read of a request gets `MCP_STDIO_FIRST_READ_SECS` (60); once
  a FOREIGN response has arrived, the drain uses
  `MCP_DRAIN_READ_TIMEOUT_SECS` (5) so a mismatched server fails bounded
  rather than 60 s per line.
- out-of-order responses are PARKED (`buffered`, cap
  `MCP_BUFFERED_RESPONSE_CAP` = 64) rather than dropped, so the call that
  owns a response can still claim it; the loop gives up after
  `MCP_MAX_RESPONSE_DRAIN` (64) foreign responses.
- a parse failure sets `desynced`; the NEXT request reconnects first
  (the failed call is not re-issued — the server may have executed it).
- two consecutive read timeouts count as a hang; the NEXT request
  force-reconnects the wedged child.
- a transport-level failure (broken pipe / EOF) triggers ONE reconnect +
  retry, which is safe because a dead child provably did not run the tool.

`MCP_TOOL_TIMEOUT_SECS` (90) is deliberately ABOVE the client's own 60 s
read budget: the registry cancels a tool future at ITS timeout, and a
cancelled read loses the bytes already consumed and desynchronises the
stream for the rest of the session (RT-515). So the client times out
first, cleanly, and the registry never cancels a read.

## Server-initiated requests: sampling and elicitation

`advertised_capabilities(transport)` decides what the client tells the
server it can do, and it is **per transport**:

- **stdio** advertises `roots` and `sampling`, because the stdio read loop
  sees every message on the same stream and can spot a server-initiated
  request (a message with `method` rather than `result`).
- **HTTP, streamable HTTP and SSE advertise neither.** The HTTP path
  parses every body as the response to its own request, so a
  `sampling/createMessage` would be read as a mismatched-id response and
  the server's request would hang unanswered. Advertising a capability
  the transport cannot honour is worse than not advertising it.

The stdio handler answers `sampling/createMessage` with a JSON-RPC error
(`-32601`, "sampling not supported by this client") and
`elicitation/create` likewise. The agent is NOT wired in: there is no
callback path from a server's sampling request to an operant model call.
`notifications/progress` and `notifications/cancelled` are logged and
ignored. Both error writes are checked and logged on failure (RT-459: the
failures were once discarded, so a server waiting for an answer never got
one and the log said nothing).

## The manager

`McpManager` holds `HashMap<String, McpTransport>` plus the bookkeeping
that makes removal and crash recovery correct:

- `add_server` / `add_streamable_server` / `add_sse_server` /
  `add_stdio_server` each connect eagerly and record a CONNECT FAILURE
  with the error's own text (`connect_failure(name)`,
  `all_connect_failures()`), so the MCP view can show WHY a server is
  down instead of a bare "error".
- `sync_tools_to_registry(registry)` unregisters the manager's previously
  synced names, registers the connected servers' tools with namespaced
  names, and sets each one's timeout to `MCP_TOOL_TIMEOUT_SECS`.
  Registries are held in a LIST (RT-517) so one manager can serve several
  registry builds (the CLI's, the gateway's), and `remove_server` takes
  ONE server's tools out of ALL of them (RT-516) — otherwise a removed
  server's tool stays registered and the call-time auto-reconnect respawns
  the child the operator just removed.
- `spawn_watchdog(interval)` is one-per-manager (an
  `AtomicBool` swap), polls every connected stdio child with
  `process_alive()` (`try_wait`), and on a dead child reconnects and
  re-syncs into every registry. `interval_secs == 0` disables it.
- `McpManagementTool` (registered as `mcp_management`) is the
  agent-facing half: list servers, list tools, call a tool, add an HTTP
  server (URL validated http/https with a non-empty host; loopback is
  allowed because local dev servers are legitimate), remove a server.

## The server (`operant mcp serve`)

`crates/operant-cli/src/mcp_serve.rs` is the other direction: operant AS
an MCP server over **stdio only**. One JSON-RPC object per line on stdin,
one per line on stdout, until EOF or a `shutdown` request.

Handled methods: `initialize`, `tools/list`, `tools/call`,
`resources/list`, `resources/read`, `ping`, `shutdown`. Everything else
is a `-32601` "Method not found". Notifications (no `id`) are consumed
with no response. A tool call that is not in the registry is a
`-32602` "Unknown tool: <name>"; a tool that runs but reports failure
comes back as a `content` array with a single text block beginning
"Error: ", not as a JSON-RPC error.

`initialize` negotiates the version the same way the client does
(`SERVER_SUPPORTED_VERSIONS = ["2025-06-18", "2024-11-05"]`, else the
newest, best-effort) and advertises `serverInfo.name = "operant-mcp"` at
`CARGO_PKG_VERSION`, with `tools.listChanged = false` and
`resources.subscribe = false`.

Tool discovery is the SAME registry the agent uses:
`build_tool_registry` attaches the memory backend (so `memory_search` /
`memory_store` work over this surface too — iter-1309 fixed their absence
here), connects the configured MCP servers when `[mcp] autoload`, and
syncs their tools in. `tools/list` maps each `ToolSchema` to
`{name, description, inputSchema}`.

Resources: `resources/list` advertises `operant://tools`,
`operant://status`, `operant://config`; `resources/read` answers
`operant://status` with the real version/status JSON and the other two
with a pointer to `tools/list` ("Resource '<uri>' is dynamic."). There is
no `resources/subscribe`.

## OAuth 2.1 + PKCE (`mcp_oauth.rs`)

A separate code path from provider-account OAuth (`oauth/`,
[providers.md](providers.md)). `OAuthProvider` implements the
authorization-code flow for MCP servers:

1. **Discovery** — `<server_url>/.well-known/oauth-authorization-server`;
   a failure falls back to `<server_url>/authorize` and `/token` with a
   warning.
2. **Client info** — a pre-registered `client_id` if configured, else a
   cached one on disk, else dynamic registration (RFC 7591). `token_endpoint_auth_method`
   is `client_secret_post` when a secret exists, `none` otherwise.
3. **PKCE** — S256 challenge, a `state` token, a localhost callback
   server on `redirect_port` (0 = a free ephemeral port). The state is
   checked on return (a mismatch is a possible-CSRF warning).
4. **Exchange** — code → access + refresh tokens, persisted to
   `~/.operant/mcp-tokens/<sha256(server_url)>/tokens.json` together with
   an absolute `expires_at` so a restarted process reconstructs the real
   remaining TTL.
5. **Refresh** — `refresh_token` grant; a 401 from a server is surfaced
   as an actionable error ("Run `operant mcp login <server>`") — the
   client does NOT hold an `OAuthManager`, so auto-refresh-and-retry on
   the HTTP path is explicitly not wired (`McpClient::send_request`).

`OAuthManager` is a process-wide singleton with a per-server provider
cache, mtime-based cache invalidation (`invalidate_if_disk_changed`) and
`handle_401`, but nothing in the client path calls the latter two today.
`get_manager()` is the accessor; `cmd_mcp` uses it for `login`.

The stored tree is under `platform::operant_home()` (the one resolver),
never a private copy of the home logic.

## Configuration

`[mcp]` (`McpSettings`):

| key | default | what it changes |
|---|---|---|
| `mcp.autoload` | `true` | connect `enabled` servers at registry build |
| `mcp.deferred_loading` | `true` | master switch for deferred MCP tool disclosure; `false` forces eager loading AND sets `tools.tool_search.enabled = "off"` |
| `mcp.watchdog_interval_secs` | `30` | stdio crash watchdog interval; `0` disables |
| `[[mcp.servers]]` | `[]` | the server list |

`McpServerConfig` per entry: `name`, `transport` (`http` | `stdio` |
`streamable-http`; default `http`), `url`, `command`, `args`, `env`,
`auth_token`, `enabled` (default `true`), `deferred` (default `false`).
`deferred = true` keeps a server out of the eager autoload (the injected
agentmemory server uses it so an `operant` invocation never spawns
`npx @agentmemory/mcp` unless the user actually connects it); it stays
connectable on demand via `operant mcp`.

`[tools.tool_search]` (`enabled` `"auto"`/`"on"`/`"off"`, `threshold_pct`
5.0, `listing` `"auto"`, `listing_max_tokens` 4000, `search_default_limit`
10, `max_search_limit` 25) is the deferred-disclosure budget; MCP tools
are the unbounded family that rides it (`McpNamespacedTool::exposure` is
`Deferred` by type).

## Surfaces

- CLI: `operant mcp list [--json]`, `add <name> (--url <URL>
  | --command <CMD>) [--transport KIND] [--args …] [--env K=V …]`,
  `remove <name>`, `test <name>`, `serve [-v]`, `login <name>
  [--auth-url …] [--client-id …] [--scope …] [--timeout …]
  [--redirect-port N]`, `configure <name> [--auth-token …] [--url …]
  [--command …] [--args …]`. `add` tests the connection BEFORE printing
  config instructions; a duplicate name is refused.
- Tool: `mcp_management` (actions `list_servers` / `list_tools` /
  `call_tool` / `add_server` / `remove_server`).
- TUI: the MCP panel (`tui/mcp_view.rs`) groups servers by
  `McpTransportKind::transport_label()` and shows connect failures.
- Server: point any MCP client at `operant mcp serve` (stdio).
- Environment: `mcp-tokens/` under the operant home holds the OAuth
  tokens; `HERMES_HOME` moves the whole home.

## Failure modes

- **A configured server cannot connect** → not fatal. The registry still
  builds; the failure is recorded on the manager and printed as a
  `tracing::warn!` ("MCP server failed to connect; continuing without
  it"). One stale URL no longer takes down `operant tools list` and every
  other registry-building command.
- **A stdio child dies** → one reconnect + one retry on the failing call;
  after that the error names both failures. The watchdog reconnects it in
  the background.
- **A stdio server hangs** → the call times out at 60 s with
  "did not respond within 60s (possible hang; consecutive timeouts: N)".
  At two consecutive timeouts the next call force-reconnects.
- **A stdio server prints a banner on stdout** → skipped up to 256 lines,
  then "produced 256 non-JSON lines without a JSON-RPC response".
- **An out-of-order or duplicate response** → parked for its owner (up to
  64), or discarded if it belongs to an already-completed request.
- **A streamable server answers `406`** → the server was configured as
  plain `http`; re-add it with `--transport streamable-http`.
- **A streamable server forgets the session** → the `Mcp-Session-Id` was
  not echoed, which is the plain-HTTP client's behaviour; same fix.
- **A 401 from the server** → the token is stale; the error says to run
  `operant mcp login <server>`. No silent refresh on this path.
- **A server sends `sampling/createMessage`** → a `-32601` error response
  (stdio only; the other transports never advertised it). The server
  decides what to do with the refusal.
- **The tool the model calls is an MCP tool the registry never synced**
  → it is not advertised at all (deferred tools appear through the
  `tool_search` bridge, not the request).

## Intention

MCP is the interoperability bet: rather than N bespoke integrations, one
protocol surface lets operant both consume external capability and be
consumed. The client/server split is deliberate — each direction was
built for a different reason (extend the agent; let other agents use this
one) but they share one tool registry, so a tool registered for the agent
is automatically servable and a tool learned from a server is just a tool.

The transport spread is deliberate too: four client transports because
the ecosystem genuinely runs four (legacy SSE, plain HTTP, spec
streamable HTTP, and stdio for local servers), while the server is
stdio-only because that is what the clients that consume it (Claude
Desktop, VS Code, another operant) actually launch. Capability advertising
is per transport because advertising a capability you cannot honour
converts a missing feature into a hung request — the same reasoning that
makes the stdio client park out-of-order responses instead of dropping
them, and makes every connect failure a recorded, printable line instead
of a fatal `?`.
