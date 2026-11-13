# Session server — the server that owns the agent

Owns: the process that owns the agentic runtime — the tool registry, the MCP
child servers, the AFT bridge pool, the kernel runtime and the database
handles — and the client protocol that lets a terminal drive a session that
lives there.
Not for: the channel adapters themselves
([gateway.md](gateway.md)), the agent loop behind a turn
([agent-loop.md](agent-loop.md)), or the session transcript
([sessions.md](sessions.md)).

Source: `crates/operant-cli/src/gateway_runner.rs` (the daemon, its agent
slots, the channel adapters), `crates/operant-cli/src/cmd_acp.rs` (the ACP
daemon and its thin client), `crates/operant-core/src/acp/mod.rs` and
`crates/operant-core/src/acp/server.rs` (the ACP protocol and its socket
transport), `crates/operant-core/src/agent/mod.rs` (the agent and its event
type), `crates/operant-core/src/gateway/mod.rs` (the gateway's own dispatch and
session cap), `crates/operant-core/src/active_sessions.rs` (which process owns
a session).

## The rule

> The expensive things — the model client, the tool registry, the MCP child
> servers, the child process fleet, the database handles — belong to ONE
> process. A session is a conversation inside that process, not a process.

This is the same rule jcode and codex keep, and the audit that checked operant
against them is
`docs/audit/2026-11-13-server-process-architecture-scale-audit.md`. The plan
that implements it is
`docs/plans/2026-11-13-thin-client-session-server-plan.md`, and the board that
tracks it is
`docs/kb/workflows/thin-client-session-server.md`.

## What is true today

**The gateway IS a server.** `operant gateway run` runs the channel adapters
in-process (`crates/operant-core/src/gateway/`), owns one tool registry, one
MCP manager with its child servers, one AFT bridge pool, one kernel runtime and
one set of database handles. Its agent slots
(`crates/operant-cli/src/gateway_runner.rs`) each own one agent and one
conversation, and a turn in one slot cannot retarget another slot's turn.

**The ACP daemon is a second server.** `operant acp server --socket <path>`
builds its own agent (`crates/operant-cli/src/cmd_acp.rs`) and serves JSON-RPC
over a unix socket (`crates/operant-core/src/acp/server.rs`). Its client,
`operant acp attach --command`, is the one command in operant that opens a
socket and builds no runtime of its own.

**The interactive surfaces are not clients.** `operant chat`, `operant run` and
the TUI each build their own agent, registry, database handles, AFT bridge,
kernel sidecar and MCP children
(`crates/operant-cli/src/main.rs`). Measured on the operator's box: 13 full
`operant` processes, 792 MiB RSS, and 13 of 13 owning a separate AFT bridge.

## The gap the plan closes

| | operant today | the plan's end state |
|---|---|---|
| the client | a full stack in its own process | a socket connection |
| the server | the gateway for channels; a second daemon for ACP | one server for both |
| a session | dies with its client | outlives its client |
| two clients on one session | two processes, two conversations | one conversation, two views |

## What the protocol carries

The event type is the agent's own `AgentEvent`
(`crates/operant-core/src/agent/mod.rs`) — 25 variants, now serialisable, because
a turn has to reach a client in another process. `TurnExitReason`
(`crates/operant-core/src/agent/turn_finalizer.rs`), `Message` and `ToolResult`
are wire types too. The test
`the_agent_event_round_trips_through_json` walks every variant: a new variant
added without a round-trip case fails it, so the wire type and its test grow
together.

## The protocol, in one paragraph

JSON lines over a unix socket, the shape the ACP server already uses. Two
directions: the server sends the agent's events as they happen; the client
sends control — prompt, steer, queue, cancel, approve, deny, switch session,
load history. The event type is the agent's own `AgentEvent`
(`crates/operant-core/src/agent/mod.rs`), so the terminal renders exactly what
it renders today.

## The cost rule to keep

A client that attaches must build NO runtime: no registry, no database
handles, no MCP children, no AFT bridge. If an attach path builds one, the
scale story is false again, and the regression test for that is part of the
plan's iteration list.
