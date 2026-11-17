# Session server — the socket a terminal attaches to

Owns: the session socket — the UNIX socket the gateway daemon binds, the JSON
lines protocol that runs over it, the thin client that attaches to it, and the
handler that maps a wire message onto the daemon's own agent slots.
Not for: the channel adapters themselves ([gateway.md](gateway.md)), the agent
loop behind a turn ([agent-loop.md](agent-loop.md)), or the session transcript
store ([sessions.md](sessions.md)).

Source: `crates/operant-core/src/session_server.rs` (bind, accept, one task per
connection), `crates/operant-core/src/session_protocol.rs` (the wire types and
the encode/decode pair), `crates/operant-core/src/session_client.rs` (the thin
client and the default path), `crates/operant-core/src/socket_auth.rs` (the
peer-authentication gate every UNIX-socket server here uses),
`crates/operant-cli/src/cmd_acp.rs` (the ACP daemon and its thin client),
`crates/operant-cli/src/gateway_runner.rs`
(`start_gateway`, the `GatewaySessionHandler`, `SessionBus`, the slot counts,
and `gateway_runner/turn_state.rs` — the per-channel turn-state JSON for
auto-continue recovery),
`crates/operant-cli/src/cmd_gateway.rs` (`operant gateway run --socket`),
`crates/operant-cli/src/main.rs` (`operant chat --attach`).

Counts on this page are as of the commit this file was written against.

## The rule

> One process owns the runtime — the tool registry, the MCP children, the AFT
> bridge pool, the database handles, the agent slots. A client that attaches
> builds NONE of that: it opens a socket, says hello, sends what the operator
> typed, and renders the events the server sends back.

This is the operant side of RT-542, and it is the second server operant runs.
The first is the ACP daemon (`operant acp server`, `acp/server.rs`); the
session socket is the one every `operant gateway run` binds by default. The
session protocol lives in its own module so it can be tested without a socket,
and the server lives in its own so it can be tested without a client — the
`session_protocol` tests never open a socket, and the `session_server` tests
never build an agent.

## What binds the socket, and when

`operant gateway run` (`cmd_gateway.rs`) resolves a socket path
(`--socket <PATH>`, defaulting to `session_client::default_socket_path()`),
passes it as `Option<PathBuf>` to `gateway_runner::start_gateway`, and
`start_gateway` binds it only AFTER the message handler exists and while this
process holds the daemon flock. That ordering is load-bearing:

1. the handler owns the agent slots, and a prompt on the socket runs a turn in
   the slot that owns the session, so a socket bound before the handler would
   serve prompts into nothing;
2. the daemon flock is the single-instance guard, so a second daemon refuses to
   start at all and the socket is never left orphaned by a losing process.

`run_session_socket` (the core function) does three things before it accepts a
single connection, and all three are load-bearing:

1. **It binds through `bind_unless_live`, which REFUSES a path a live server
   already answers on.** The original bind began with an unconditional
   `remove_file(path)`, which made the socket stealable: `operant acp server
   --socket <the gateway's socket>` could unlink the live daemon's inode and
   bind its own JSON-RPC listener at the same path, after which every
   `operant chat --attach` spoke to ACP while the daemon kept serving the
   orphaned inode and logging nothing. So the rule is: something LISTENING on
   the path is a refusal, and only a path that provably has nobody behind it is
   cleared — which is what keeps a restart after SIGKILL working. The residual
   race between the probe and the bind is real and stated: the gateway daemon's
   own flock closes it for `gateway run`, and the ACP server has no such lock,
   so two ACP starts can still race.
2. **It tightens the socket to 0600 through `socket_auth`, and a failure to do
   so ABORTS startup rather than warning.** The socket carries full control of
   an agent and its tools, and the default umask gives a socket 0755 — an
   operator who bound `/tmp/session.sock` would otherwise get a world-reachable
   handle to their agent plus one line of advice in a log nobody reads.
3. **Per connection, on Linux unix, the PEER's uid must equal ours**
   (`SO_PEERCRED`, via `socket_auth::authorize_peer`); a refused peer is dropped
   without being read, so it never reaches the handler. Filesystem permissions
   are not the whole story — a same-uid process is inside the boundary the
   socket mode draws, and on a multi-agent host "same uid" is not a trust
   boundary.

Both gates live in `crates/operant-core/src/socket_auth.rs` rather than in this
module because there was ONE gate and TWO sockets: the ACP server binds through
the same `bind_unless_live` with strictly MORE authority (an arbitrary
`command` reaching the whole agent runtime, plus `stop`) and had neither gate.
Two copies of a credential check is how the two drift, so both servers call the
one module and cannot.

## The path

`default_socket_path()` is `platform::operant_home().join("operant-session.sock")`
— the one home resolver (`HERMES_HOME` then the sticky active profile then
`~/.operant`), so the socket follows the operator's profile and two clients on
the same profile name the same file.

## The protocol, in one paragraph

JSON lines, one message per line in both directions, every line carrying the
protocol version in an `Envelope`. `PROTOCOL_VERSION` is `1`; `decode` refuses
a line that speaks a different version before it looks at the message, and
`encode` writes the version the caller passes. A line that is not JSON, or
carries no version, is a `ProtocolError` with a name — the server answers with
a `WireMessage::Error` rather than going silent, so the client learns why.

The message set is 12 variants in two groups:

- **client to server**: `Hello`, `Prompt`, `Steer`, `Queue`, `Cancel`
  (with an optional `turn_id`), `Approve` / `Deny` (a pending tool call, with
  an optional refusal reason), `Switch` (move to another session),
  `LoadHistory`.
- **server to client**: `HelloOk` (the sessions the daemon currently holds),
  `Event` (one `AgentEvent` as it happened), `History` (a session's existing
  messages), `Error`.

`WireMessage::session_id()` is the seam that makes the subscription work: the
server reads it on every message and, when the named session differs from the
one this connection is on, it calls `SessionHandler::subscribe` for the new
one. Without that re-subscription the first session a client touched owned the
connection for its whole life, and `Switch` plus a second session were
unreachable.

## How a turn flows

1. The client connects and sends `Hello`. The server answers `HelloOk` with the
   live session ids the handler reports (`GatewayMessageHandler::live_session_ids`).
2. The client sends `Prompt { session_id, text }`. The handler spawns the turn
   and returns NO replies: the answer arrives as `Event` lines on this client's
   subscription, which is why the connection stays free to receive them.
3. The gateway handler runs the turn in the slot that owns the session, under
   the same gates a channel turn obeys: ESTOP, the presence registry (another
   live process owning the session refuses the prompt), and the concurrency cap.
4. `LoadHistory` reads the slot's transcript through `agent.transcript_lines`.
   It is the ONLY message dispatched on the blocking pool, because it is a
   SQLite read behind a mutex and a burst of them would park a runtime worker
   at a thousand attached clients.

The server's per-connection loop is a `tokio::select!` between the client's
next line and the next event on the subscribed session, so a live turn streams
without the client asking again. A client that falls behind is told
(`Lagged(n)` becomes an `Error` message naming the count) rather than being fed
stale filler.

## The gateway handler's rules

`GatewaySessionHandler` is where the protocol meets the daemon. Its behaviour
is the part an operator sees:

- **Commands first, before any gate.** `/stop` and `/sessions` are handled
  before ESTOP or the cap, because `/stop` is what an operator reaches for when
  a turn is stuck, and "operant is paused" is not an answer to that. A command
  is a slash plus a KNOWN name — a prompt about `/etc/hosts` is a prompt.
- **The cap bounds concurrent turns, counted not reserved.** A prompt is
  admitted while fewer than `max` turns run and takes its lease only once it
  holds a slot. Holding the lease while queued let one busy slot consume the
  whole cap: four sessions behind one long turn refused the fifth while three
  slots sat idle. `SessionLease` releases on drop, so a finished turn, an error
  and a panic all give the slot back.
- **A cancel is an interrupt, not an abort.** Dropping the future at its next
  await skips the turn's tail — no interrupted marker, no clean commit, and an
  in-flight tool row left saying its side effects are unknown, which the reload
  path then strips so the model re-issues a write that already happened.
- **`Approve`/`Deny` are matched by tool-call id**, because a socket client has
  no channel and the pending permission map is keyed by channel.
- **The queue is bounded** at 32 messages per session: a client that can park
  a million messages owns the daemon's memory otherwise.

## Configuration

There is no config section for the socket itself — it is part of the daemon, not
an opt-in. The one server-level knob is the slot count, which is the same
number as the concurrency cap:

`[gateway] max_concurrent_sessions` (via `slot_count_for`) is clamped to
`1..=256`; the default with no setting is 4. A slot is a full agent with its own
conversation, so the count is a RAM decision. Measured on the operator's box
with this build: a daemon at 1 / 4 / 16 / 64 slots is 37 / 43 / 69 / 173 MiB
RSS — about 2.1 MiB per idle slot — so the ceiling is set by the file-descriptor
and scheduler budget, not by RAM, and an ACTIVE turn costs more than an idle
slot (a number nobody has measured yet).

The queue bound and the subscription ring are constants, not config:
`MAX_QUEUED_PER_SESSION = 32` and the per-session broadcast channel's 256 slots
(`SessionBus::sender_for`).

## Surfaces

```bash
operant gateway run                 # binds the default path in the daemon
operant gateway run --socket <path> # bind it somewhere else
operant chat --attach <path> [PROMPT]
```

`operant chat --attach` is the thin client. Without a prompt it prints the
server's live sessions and parks until the server closes the socket; with a
prompt it sends it to the first live session (or a fresh id when there is none)
and prints every event until the server closes. Under `[tui] rich_output` the
TUI takes the same socket instead and renders the server's stream, so the
attach path and the local path share one renderer.

## Failure modes

| What breaks | What you see |
|---|---|
| No daemon is listening | `cannot reach the server at <path>` from `SessionClient::connect` |
| A second `operant gateway run` | the daemon flock refuses: "Gateway is already running on this machine" |
| Another process owns the session | the prompt is refused with the session id and the advice to close it first |
| Too many concurrent sessions | the turn is not started; the message names the limit |
| A too-slow client | an `Error` message naming how many events it skipped |
| A line this build cannot read | an `Error` naming the protocol problem, never a silent drop |
| `operant gateway run` under ESTOP | the prompt is refused with "operant is paused — send /unpause or run `operant resume`" |
| The path is already served by a live process | startup fails with "is already served by another process — refusing to take it over" |
| The socket cannot be made 0600 | startup fails before the accept loop; clients get "connection refused" |
| A client connects as a different uid (Linux) | the connection is dropped with a warning, without being read |

The stale socket file from a KILLED server is not a failure mode: a path with
nobody behind it is cleared, which is what lets a restart take the same path.

## Intention

The socket exists so a terminal can drive a session that already lives in a
daemon without rebuilding one. Operant's other surfaces (`operant chat`,
`operant run`, the TUI) each build their own agent, registry, database handles,
AFT bridge, kernel sidecar and MCP children — measured at 13 full `operant`
processes, 792 MiB RSS, each owning a separate AFT bridge. The socket is the
first surface that does not.

It is deliberately NOT a general API: it is the gateway's own agent slots over a
UNIX socket, with the same gates a channel turn obeys, a 0600 socket, and a
peer-uid check. The read-only HTTP session API (`gateway_api`) is the separate
surface for the non-terminal consumers, and it shares the same daemon flock. A
future unified server (one process serving both the socket and ACP) is the
thin-client plan's end state; this page describes the socket half that exists
today.