# Server and process architecture: the scale audit

**Date:** 2026-11-13
**Scope:** the operant process architecture, read against two reference implementations
(`external-repos/jcode`, `external-repos/codex`) that already solve the same problem.
**Evidence strength:** every finding below is single-pass and hand-verified against the
live surfaces (the running process list, the running gateway, the source). Two
background readers mapped jcode and codex; their claims were checked against the
files they cited, and every operant claim was checked by reading the code and by
measuring the machine. No finding was cross-checked by a second reader.

## What this repo does:

Operant is a coding agent. It answers on chat channels (Telegram, Discord, Slack,
WhatsApp, email, SMS, webhooks) and in a terminal (a full-screen TUI, a plain
`chat`, a headless `run`). One `operant` process is the whole product: the agent
loop, the tool registry, the model client, the MCP child servers, the databases and
the channel adapters all live inside it.

**The load assumed:** one operator, one machine, many sessions at once — the goal
recorded in the repo is 100–1000 concurrent agent sessions. The number that matters
is RAM per session, because the sessions are the thing that multiplies.

**What I measured on this box, right now:**

| Measure | Value |
|---|---|
| Full `operant` processes running | 13 |
| Their RAM, total / average | 792 MiB / 60.9 MiB |
| The biggest one | 175.6 MiB |
| The gateway daemon (`operant gateway run`) | 99.9 MiB |
| AFT bridge children (one per full binary) | 16, 124 MiB total, 7.8 MiB each |
| Sourcehound (MCP) children | 4, 18 MiB |
| Full binaries that own their own AFT bridge | **13 of 13** |

The same table for the two references, from their own repos:

| | First session | Each extra session | Where it is written down |
|---|---|---|---|
| jcode | 71.3 MB | **~3.3 MB** | `jcode/README.md:60-70`, measured by `jcode/scripts/bench_headless_memory.py` |
| codex | 398.6 MB | ~194.1 MB | the same jcode table, measuring codex CLI |
| operant (measured here) | ~61 MiB average, 175.6 MiB worst | **~61 MiB + ~8 MiB bridge** | this audit |

The headline: **operant pays the whole stack once per session. jcode pays it once
per machine.** At 100 sessions that is roughly 6.9 GB for operant against roughly
0.4 GB for jcode — a factor of about 15, from the same shape of work.

## The three systems, side by side

The shared rule all three are trying to keep:

> **The expensive things — the model client, the tool registry, the MCP child
> servers, the child process fleet, the database handles — belong to ONE process.
> A session is a conversation inside that process, not a process.**

| | jcode | codex | operant (today) |
|---|---|---|---|
| The server | `jcode serve` (`jcode-app-core/src/server.rs:552-607`) | `codex-app-server` (`codex-rs/app-server/src/lib.rs:1025-1030`) | `operant gateway run` **and** `operant acp server --socket` — two of them |
| Who owns the agent loop | the daemon | the daemon (`ThreadManager`, `codex-rs/core/src/thread_manager.rs:582-611`) | the process that starts it — so every TUI owns its own |
| Who owns the MCP children | the daemon, one pool (`jcode/crates/jcode-base/src/mcp/pool.rs:51-54`) | the daemon, one pool + per-chat leases (`codex-rs/codex-mcp/src/connection_pool.rs:46-108`) | the process — 13 of 13 binaries own one (RT-517 made it one *per process*, not one per machine) |
| Channels | Telegram/Discord run **inside** the daemon (`jcode/crates/jcode-app-core/src/channel.rs:25-62`) | n/a (no chat channels) | inside the gateway — **correct already** |
| The TUI | a thin client: it dials the socket and holds an `InertRuntimeProvider` that refuses live completions (`jcode/crates/jcode-tui/src/tui/app.rs:48-56`) | a thin client over `AppServerTarget::LocalDaemon` (`codex-rs/tui/src/lib.rs:584-644`) | **a full stack in its own process** (`main.rs:3484/3512/3748` → `TuiApp::enter`) |
| Two clients on one live session | yes — a second client reuses the live runtime (`jcode/crates/jcode-app-core/src/server/client_session.rs:1339-1362`) | yes — `thread/resume` joins a live thread (`codex-rs/app-server/src/request_processors/thread_lifecycle.rs:4675-4979`) | no — two clients are two processes |
| Turn concurrency | one runtime per session; turns serialise inside a session | one thread per session; `active_turn` mutex per session (`codex-rs/core/src/session/session.rs:112`) | 4 agent slots for every session on the gateway (`gateway_runner.rs:2419`) |
| Daemon restart | sessions are drained, then the process `exec`s the new binary on the same socket (`jcode/crates/jcode-app-core/src/server/reload.rs:186-299`) | threads are snapshotted and restored on the next start (`codex-rs/app-server/src/daemon_thread_recovery.rs:1-49`) | the gateway restarts and the sessions are lost |

Both references keep the same three properties, and all three are missing in operant
today:

1. **One server per machine.** jcode: one `jcode serve` owns the model client, the
   session map, the MCP pool and the channels. codex: one `codex-app-server` owns
   the `ThreadManager`. operant: the gateway owns the channels, and a *second*
   daemon (`operant acp server`) owns one agent, and every TUI owns a third.
2. **The client is thin.** jcode's TUI cannot even run a model call — its provider
   refuses. codex's TUI picks `Embedded` or `LocalDaemon` at launch
   (`codex-rs/tui/src/lib.rs:1098-1135`). operant's TUI builds the agent, the
   registry, the MCP children and the AFT bridge itself.
3. **The session outlives the client.** jcode keeps a live session for 30 s after
   the client disconnects, then hands it to the next client
   (`jcode/crates/jcode-app-core/src/server/client_disconnect_cleanup.rs:19-20`).
   codex keeps the thread loaded for its idle timeout and re-joins it with
   `thread/resume`. operant: the session dies with the process.

## Must fix

### 1. The TUI and `operant chat` / `operant run` build the whole stack per session
(`crates/operant-cli/src/main.rs:3484`, `:3512`, `:3748`; `crates/operant-cli/src/main.rs:1260` `build_registry`, `:2178` `create_runtime_agent`)

- **What this is:** every interactive entry point calls `TuiApp::enter`, which
  builds a fresh agent: its own tool registry, its own app/cron/kanban database
  handles, its own AFT bridge, its own kernel sidecar, its own MCP child servers.
- **Problem:** this is the exact shape the scale target cannot survive. Measured
  above: 13 full binaries, 792 MiB, and **13 of 13 own a separate AFT bridge**.
  One more `operant chat` is one more ~61 MiB process and one more child fleet.
  The gateway — the one process that *could* serve everyone — is already 99.9 MiB
  and idle.
- **Fix:** give the interactive surfaces an attach mode: the TUI connects to a
  running server over a socket and drives the session there (the plan below).
  The local build stays for `operant run` and for the first connection.
- **If we skip it:** the 100-session target needs ~7 GB of RAM and 100 child
  fleets; on the box this audit ran on, that is the whole machine.

### 2. There are two daemons, they do not share anything, and each spawns a child fleet
(`crates/operant-cli/src/gateway_runner.rs:2419` and `:4416`; `crates/operant-cli/src/cmd_acp.rs:223` `AcpRuntime::start`, `:323` `build_acp_agent`)

- **What this is:** `operant gateway run` owns the channels and four agent slots.
  `operant acp server --socket` owns one agent. The two are separate programs with
  separate runtimes, separate registries, separate MCP children and separate AFT
  bridges.
- **Problem:** run both on one machine and you pay two stacks to serve one user.
  The gateway also has **no client socket at all** (`gateway_runner.rs` has no
  `UnixListener`), so there is nowhere for a TUI to attach even though the ACP
  daemon can be told to.
- **Fix:** one server. The gateway already owns the slots, the registry, the MCP
  manager and the database handles — add the client socket there, and let the ACP
  daemon be a mode of that server rather than a second program.
- **If we skip it:** "the gateway is the server" stays true only for channels. The
  operator's own terminal keeps paying per session.

### 3. The plan and a code comment promise the scale story that does not exist
(`docs/plans/2026-11-12-process-architecture-integrated-gateway-server-safe-drain-sh.md:87`; `crates/operant-cli/src/gateway_runner.rs:2417-2418`)

- **What this is:** the plan's row 7 says "`operant chat`/`run` attach to it
  instead of building their own runtime", and the gateway's slot comment says "The
  CLI's scale story is the daemon (RT-513): many clients, ONE process, one child
  set."
- **Problem:** neither is true. The ledger row for RT-513 is honest — it says
  "NOT done here: `operant chat` and `operant run` do not yet take an `--attach`
  flag" — but the plan and the code comment still say the larger thing, and the
  comment is what a future reader trusts.
- **Fix:** the plan's row 7 text and the `AGENT_SLOT_COUNT` comment must state
  what shipped (a daemon and a one-shot thin client), and the plan must carry the
  remaining work as its own row.
- **If we skip it:** the next agent reads the comment, believes 100 sessions cost
  one process, and sizes the machine wrong.

## Should fix

### 4. The ACP protocol cannot carry an interactive session
(`crates/operant-core/src/acp/mod.rs:544-625`, `:450-464`)

- **What this is:** `session/prompt` awaits the whole turn and then emits one
  `agent_message_chunk` with the final text. There is no stream of tool events,
  no permission round-trip, and `session/cancel` acknowledges without interrupting
  (the code says so itself: "the prompt-interrupt itself is unwired").
- **Problem:** a TUI client on this protocol would show nothing until the turn
  ended, could not ask the operator to approve a tool, and could not stop a turn
  that was going the wrong way.
- **Fix:** the wire protocol in the plan below — turn events one way, control the
  other, cancel actually cancels.
- **If we skip it:** the TUI can only be attached for one-shot commands, which is
  not the product.

### 5. The agent's event type cannot cross a socket
(`crates/operant-core/src/agent/mod.rs:285`)

- **What this is:** `AgentEvent` has 25 variants — thinking, reasoning, tool start,
  tool complete, tool error, content, done, iteration, usage, cost — and it
  derives only `Debug` and `Clone`.
- **Problem:** this enum *is* the TUI's whole rendering contract. Without serde on
  it, there is no way to send a turn to a client.
- **Fix:** derive `Serialize`/`Deserialize` on it and on the types it carries
  (`ToolResult`, `TurnExitReason`, `Message`). Mechanical, but it touches the
  types the gateway also uses, so it belongs in one iteration with its own test.
- **If we skip it:** every attempt to attach a client re-invents a parallel event
  type that drifts from the one the local TUI renders.

### 6. The thin client has no caller
(`crates/operant-cli/src/cmd_acp.rs:79-119` `cmd_attach`)

- **What this is:** `operant acp attach --socket … --command …` is the one command
  in operant that opens a socket and builds no runtime. Nothing in the product
  calls it — only the tests.
- **Problem:** the command that would save the RAM exists, is documented, and is
  unreachable from the surfaces an operator actually uses.
- **Fix:** an `--attach` flag on `operant chat` and on the TUI, defaulting to the
  local build when no server is running.
- **If we skip it:** every operator session keeps paying the full stack.

### 7. The gateway admits 32 sessions and runs 4 agent slots
(`crates/operant-core/src/config.rs:1748` `max_concurrent_sessions: Some(32)`; `crates/operant-cli/src/gateway_runner.rs:2419` `AGENT_SLOT_COUNT = 4`)

- **What this is:** two different numbers for the same question. The cap admits 32
  live sessions; there are 4 agent slots, and a slot owns its agent.
- **Problem:** 32 sessions share 4 agents, so at most 4 turns run at once and two
  sessions in one slot serialise on that slot's lock. The comment says four is
  right because "a gateway serves a handful of seats" — but the config says 32.
- **Fix:** one model, stated in one place. Either the cap is 4 (and the config
  says 4), or the slots grow and the cap follows. jcode and codex both give every
  session its own runtime and serialise turns *inside* the session, not across
  sessions.
- **If we skip it:** an operator sets 32 sessions and gets four at a time, with no
  log line saying so.

### 8. Three programs each build their own full runtime
(`crates/operant-cli/src/main.rs:1260` `build_registry`; `crates/operant-cli/src/cmd_acp.rs:323` `build_acp_agent`; `crates/operant-cli/src/gateway_runner.rs:4416`)

- **What this is:** the CLI's normal path, the ACP daemon and the gateway each
  assemble their own agent, registry and child fleet.
- **Problem:** RT-517 made the *pool*, the *kernel runtime* and the *watchdog*
  one-per-process, but "one per process" still means three programs = three child
  fleets. A configuration change to the registry has to be applied in three
  places.
- **Fix:** one builder that takes the process's config and returns the runtime
  every entry point uses; the three call it.
- **If we skip it:** the child fleet keeps multiplying by the number of programs
  the operator runs.

## Nice to have

### 9. `session/cancel` lies about cancelling
(`crates/operant-core/src/acp/mod.rs:450-464`)

- **What this is:** the method acknowledges the cancel and leaves the turn
  running. The code names this (RT-275) and leaves it open.
- **Problem:** a client that cancels waits for a turn that never stops.
- **Fix:** give the ACP runtime an abort handle per in-flight turn and trip it.
  The plan's protocol work needs this anyway.
- **If we skip it:** the first remote client that tries to stop a turn hangs.

### 10. Nothing documents what a session costs
(`docs/`)

- **What this is:** jcode publishes a benchmark table and a script
  (`jcode/scripts/bench_headless_memory.py`); operant publishes nothing.
- **Problem:** the 100–1000 target has no number behind it, so nobody can tell
  whether a change made it better or worse.
- **Fix:** a `scripts/bench_session_memory.py` that starts N sessions and reports
  the RSS of the process tree, the way jcode's does.
- **If we skip it:** the scale claim stays a wish.

### 11. The gateway's session API is read-only
(`crates/operant-cli/src/gateway_api.rs:231-234`)

- **What this is:** three GET routes (`/api/sessions`, `/api/sessions/{id}/messages`,
  `/api/subagents`) on `127.0.0.1:8731`.
- **Problem:** there is no way to send a message to a gateway session through it,
  so it cannot be the client protocol either.
- **Fix:** the socket protocol in the plan replaces it for the client path; the
  HTTP API stays for the dashboard.
- **If we skip it:** it is harmless, but it is a second "session surface" a reader
  has to learn and a future change could grow into.

## Verdict

**Not healthy for the scale goal, and the gap is one feature, not thirty.** The
channels side is right: the gateway is already a server, with per-seat agents,
one MCP child set and one database per process. The interactive side is the
inverse of that: every TUI is a full stack. Fix findings 1, 2, 4 and 5 — one
attach path, one server, one wire protocol, one serialisable event type — and
operant has the shape jcode and codex both have.

**Lean:** no deletions proposed; this audit found dead weight only in the two
daemons (finding 2) and the three runtime builders (finding 8), and both are
fixed by *merging* the paths rather than by cutting code. No dependency can be
dropped on this evidence.

**Not checked:** the TUI's 336 render/input modules (only its entry point and its
event contract were read); the seven channel adapters beyond how they connect;
the kernel sidecar's Python runtime (its tests need the main tree's virtualenv);
the provider adapters; the Holon org semantics; the release/deploy path beyond
the one binary this audit deployed. The jcode and codex readings are second-hand
from two background readers whose citations I checked but did not re-derive
line by line; operant's findings are the ones I verified myself.
