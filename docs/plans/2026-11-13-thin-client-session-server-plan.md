# The thin-client session server: implementation plan

**Date:** 2026-11-13
**From:** `docs/audit/2026-11-13-server-process-architecture-scale-audit.md`
**Goal:** one server per machine owns the agent, the tool registry, the MCP child
servers and the databases. A terminal client connects to it and drives a session
there, instead of building the whole stack in its own process.

**Done means:** `operant chat` and the TUI have an `--attach` mode. With a server
running, a new client costs a socket connection and a few MiB — not a 61 MiB
process and a second child fleet. `operant run` (headless) keeps the local build;
it is a one-shot job, not a long-lived client.

**The shape to copy:** jcode's `jcode serve` + `run_tui_client`
(`jcode/crates/jcode-app-core/src/server.rs:552-607`,
`jcode/crates/jcode-tui/src/tui/backend.rs:419-433`), and codex's `app-server` +
`AppServerTarget` (`codex-rs/app-server/src/lib.rs:1025-1030`,
`codex-rs/tui/src/lib.rs:584-644`). Both are the same design: a daemon owns the
runtime, a client subscribes to a session, the session survives the client.

**The rule every iteration obeys:** one iteration = one commit + one push + one
named regression test, verified with `cargo fmt --all`, a scoped `cargo check`, the
scoped tests, `cargo clippy`, `python3 scripts/audit-ledger-index.py` and
`./scripts/genome lint`. The iteration label comes from `origin/main` at commit
time, never from local history.

## The design in one picture

```
                     ┌──────────────────────── operant gateway (ONE process) ─────────────────────┐
                     │                                                                            │
 telegram ─┐        │   agent slots (one agent + one conversation each)                        │
 discord  ─┼─► channels ──►  │  slot 0   slot 1   slot 2   slot 3   …  (grow with the cap)     │
 slack    ─┤        │        │      └──────────┬──────────┘                                    │
 email    ─┘        │        │                 │                                               │
                     │        │   ONE tool registry · ONE McpManager (children) · ONE AFT pool  │
                     │        │   ONE kernel runtime · ONE MCP watchdog · ONE DB handle set     │
                     │        │                                                                            │
 webhooks ─┐        │        │   session server: a unix socket, one connection per client      │
 TUI   ────┼─► attach client ──►  │   ┌──────────────────────────────────────────────────────┐    │
 chat  ────┤        │        │   │   │ the wire protocol (below)                            │    │
 run   ────┘        │        │   │   └──────────────────────────────────────────────────────┘    │
                     │        └────────────────────────────────────────────────────────────────────┘
```

The wire protocol is JSON lines over a unix socket, the same shape as the ACP
server that already exists (`crates/operant-core/src/acp/server.rs:45`), with two
directions:

* **events** (server → client): the agent's `AgentEvent`s, as they happen.
* **control** (client → server): prompt, steer, queue, cancel, approve, deny,
  switch session, load history.

## Iteration order

Each iteration is independent and can be reviewed on its own. The order is:
the protocol first (so the client has something to speak), then the server, then
the client, then the numbers.

### 1. `AgentEvent` becomes a wire type
**What changes:** derive `Serialize`/`Deserialize` on `AgentEvent`
(`crates/operant-core/src/agent/mod.rs:285`) and on the types it carries
(`ToolResult`, `TurnExitReason`, `Message`, `ContentBlock`). Add a `version`
field to the wire envelope.
**Why first:** every later iteration sends these. Doing it once, here, avoids two
event types that drift.
**Test:** `the_agent_event_round_trips_through_json` — every variant survives a
serialise/deserialise round trip unchanged, and the version survives with it.

### 2. The session protocol, as data
**What changes:** a new module `crates/operant-core/src/session_protocol.rs`
holding the wire types: `ClientCommand` (prompt / steer / queue / cancel /
approve / deny / switch / load_history) and `ServerEvent` (an `AgentEvent` plus
the session id). One encode function and one decode function, both pure.
**Why:** the protocol is then testable without a socket, a server or an agent.
**Test:** `the_protocol_round_trips_a_prompt_and_its_events` — a prompt in, the
same prompt out; an event in, the same event out; a malformed line is rejected
with a named error, not a panic.

### 3. The gateway serves a client socket
**What changes:** the gateway binds a unix socket (default
`$OPERANT_HOME/gateway.sock`, overridable with `--socket`) and serves each
connection on its own task, the way `run_socket_server` already does
(`crates/operant-core/src/acp/server.rs:45-82`). The daemon lock and the stale
socket removal follow the existing pattern in the ACP server.
**Why:** the server side first, so the client has something to talk to.
**Test:** `the_gateway_serves_two_clients_on_one_socket` — two connections, two
`ping` replies, one process, one MCP child set (the fake MCP server records its
connections in a file, so the count is on disk).

### 4. A session on the server that a client can drive
**What changes:** the gateway's slots become the sessions the protocol drives:
`prompt` runs a turn in the slot that owns that session id, and the turn's events
go to every client attached to that session. The client can `switch` to another
session, and the second client on the same session sees the same events.
**Why:** this is the "the session outlives the client" property.
**Test:** `two_clients_share_one_live_session` — client A prompts, client B
receives the same events, and the transcript is written once.

### 5. The TUI can attach
**What changes:** the TUI gets a backend that reads events from the socket
instead of from a local agent, and a `--attach <socket>` flag on `operant chat`
and on the TUI entry. Without the flag, or when the socket is absent, the local
build stays exactly as it is.
**Why:** the operator-facing half.
**Test:** `the_tui_renders_events_from_a_socket` — a fake server sends three
events, the TUI renders the three rows, and no agent was built (the registry's
build counter stays at zero).

### 6. Approve, deny and cancel actually cross the socket
**What changes:** a tool that needs approval sends a request to the attached
client and waits for the answer; `cancel` aborts the in-flight turn (this is the
abort handle RT-275 says is missing).
**Why:** without this the attached TUI is a read-only viewer.
**Test:** `an_attached_client_can_approve_a_tool_and_cancel_a_turn`.

### 7. The client dies, the session lives
**What changes:** the server keeps the session (and its conversation) for a grace
window after the last client disconnects, the way jcode keeps it for 30 seconds
(`jcode/crates/jcode-app-core/src/server/client_disconnect_cleanup.rs:19-20`).
A new client attaches to the live session and sees the history.
**Why:** this is the property that makes a server worth having.
**Test:** `a_session_survives_its_client_and_a_new_client_joins_it`.

### 8. One server, one child fleet
**What changes:** the ACP daemon (`operant acp server --socket`) stops being a
second program with its own runtime; it becomes a mode of the gateway, or it is
retired in favour of the gateway's socket. The three runtime builders
(`main.rs:1260`, `cmd_acp.rs:323`, `gateway_runner.rs:4416`) become one builder
the three call.
**Why:** this is the audit's finding 2 and 8 — the duplication that makes "one
server" untrue.
**Test:** `one_server_one_child_fleet` — the gateway and a client, one MCP child
set between them.

### 9. The slot count and the session cap agree
**What changes:** one number, stated in one place. Either the cap is the slot
count, or the slots grow to the cap; and the config default, the example toml
and the comment in `gateway_runner.rs:2415-2419` all say the same thing.
**Why:** the audit's finding 7 — 32 sessions over 4 slots is a silent throttle.
**Test:** `the_session_cap_and_the_slot_count_agree`.

### 10. The number, published
**What changes:** `scripts/bench_session_memory.py` — start N attached sessions,
report the RSS of the process tree, the way `jcode/scripts/bench_headless_memory.py`
does. The README gets the table, and the repo's own target row gets the measured
number instead of the word "target".
**Why:** the audit's finding 10 — a scale claim with no number behind it.
**Test:** the script runs and prints a table (a smoke check, not a unit test).

## What this plan does not do

- It does not change the channel side. The gateway already owns the channels,
  the slots, the registry, the MCP children and the databases; that part is right.
- It does not remove `operant run`'s local build. A headless one-shot job is
  simpler without a socket, and jcode and codex both keep the same exception
  (`jcode/src/cli/commands.rs:2627-2674`, `codex-rs/exec/src/lib.rs:1097-1101`).
- It does not add a framework, a dependency or a config file. The socket server
  pattern is already in the repo (`crates/operant-core/src/acp/server.rs`), the
  event type is already in the repo, and the lock file is already in the repo.

## Risks, named

| Risk | Why it is real | What keeps it small |
|---|---|---|
| The wire protocol becomes a second API | operant already has an ACP protocol and an HTTP session API | the protocol is the ACP transport with two extra directions; the ACP server's own request/response methods stay as they are |
| The TUI's 336 modules all assume a local agent | the TUI was written against `OperantAgent` directly | the backend is one seam: a local agent and a socket agent both produce `AgentEvent`s, so the render layer does not change |
| Two clients on one session fight over the conversation | two clients can both steer a session | the session has one owner at a time (the lease idea codex uses: `codex-rs/codex-mcp/src/connection_pool.rs:143-180`); a second client attaches read-only until it takes the lease |
| A stale socket file blocks the server | a killed daemon leaves the file | the ACP server already removes a stale socket before binding (`crates/operant-core/src/acp/server.rs:46-53`); the same rule applies |
| The daemon restart loses the live sessions | the gateway restart is the operator's restart | the interrupted marker and the transcript already survive a kill (RT-510, RT-511); the client reconnects and reloads history |

## The order of the money

If only two iterations happen, do **3** and **5**: the socket server and the
attaching TUI. Together they turn one interactive session from a 61 MiB process
with its own child fleet into a few MiB of client against the server that was
already running. That is the difference the 100–1000 target lives or dies on.
