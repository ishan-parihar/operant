# process architecture audit: gateway server session management

Dated-frozen: audited `2026-11-12`; findings mark verified vs asserted.

Base commit: `8347a316` (`fix(iter-2095)`). Every finding below was verified by reading the
cited lines in this tree; the audit is read-only and changed no code.

## Contents

- [Scope](#scope)
- [The architecture, in one picture](#the-architecture-in-one-picture)
- [Findings](#findings)
- [Also spotted, not numbered](#also-spotted-not-numbered)
- [Test coverage of the risky surfaces](#test-coverage-of-the-risky-surfaces)
- [Verdict](#verdict)

## Scope

The **process architecture** of operant: how an agent session is created, held, persisted and
resumed; how the gateway daemon turns platform messages into agent turns; which OS processes and
child processes exist; what a restart preserves; and what the operator can see.

**Load assumed** (the operator's stated target): one machine, one operator, the gateway daemon plus
a handful of CLI sessions, scaling to 100–1000 concurrent agent sessions.

**Method.** One direct read of the runtime surfaces, plus three independent read-only deep-reads
(session lifecycle; server/process/deploy surfaces; MCP + subprocess ownership) whose findings were
re-verified line by line before merging. Two further delegated passes (gateway runtime; tests+deploy)
failed without output; those scopes were covered by direct reads instead.

**Measured surface.** 7 crates, ~493,000 lines of Rust (operant-cli 224,040 in 411 files;
operant-core 212,074 in 301 files) · 52 CLI subcommands · 64 builtin tools · 7 platform adapters ·
48 SQLite tables · 172 config keys in a 927-line `operant.example.toml` · one 57 MB release binary
that is the only artifact — every host is that binary started as its own OS process.

**Not read.** The TUI render layer, tool implementations, provider adapters, Holon org semantics, the
non-webhook platform adapters' internals, the kernel sidecar's Python side. Not measured: live RSS
per session, real child-process counts, real queue-wait numbers (no operant process was running on
the audit host). Not checked: the Windows path (no `flock`, no SIGTERM).

## The architecture, in one picture

| Surface | Process | Owns | Shares with other sessions? |
|---|---|---|---|
| Gateway daemon | 1 per machine (flock on `<home>/gateway.lock`) | 7 platform adapters, webhook HTTP server (`:8080`), read-only session API (`:8731`), Holon organs, 2 agents, cron scheduler | MCP children **yes**; AFT bridge + kernel sidecar **no** (one pool per registry build) |
| TUI chat / `operant run` | 1 per CLI session | own agent, own registry, app+cron+kanban DB handles, own AFT bridge, own kernel sidecar, own MCP children | **nothing** — no attach mode exists |
| ACP server | 1 per client, stdio | fresh agent **per prompt**, fresh tokio runtime **per prompt**, fresh child fleet **per prompt** | nothing |
| `operant mcp serve` | 1 per MCP client | own registry, own MCP manager, own children | nothing |
| Dashboard | 1 | opens the kanban DBs directly; never talks to the gateway | reads files only |

| Concurrency slot | Real limit today | Knob |
|---|---|---|
| Gateway agent turns | **1 at a time, machine-wide** (`turn_lock`) | none |
| Gateway in-flight sessions | unlimited by default; the cap counts *turns*, not sessions | `max_concurrent_sessions` (default `None`) |
| Gateway queues | **unbounded** inbound channel + `VecDeque` | none |
| In-process fan-out (sub-agents) | 6 workers, depth 1, 3 children | depth/children only — the worker cap has **no** config path |
| Cron jobs | 1 at a time inside the 60 s tick | none |
| MCP stdio reads | 30 s registry timeout against a 60 s client read → **the registry always cancels first** | none |

**The operator's four requirements, answered.**

1. **Gateway and server as one integrated system — NO.** They are separate processes over one SQLite
   file. The gateway's own API is read-only (3 GET routes); the dashboard never calls it and always
   reports the gateway as stopped.
2. **A restart drains active sessions safely — NO.** The *conversation* survives (transcript
   rehydrated, tool-call ids repaired on load). The *turn* does not: it is killed mid-tool, and
   nothing records that it was in flight.
3. **100–1000 parallel agents on one machine — NO.** One turn at a time on the gateway; N full
   processes on the CLI path.
4. **Shared MCP/CLI tools — HALF.** Shared within a process (`McpManager`, `Sourcehound::global` is a
   process singleton). Never across processes, because there is no server to attach to.

## Findings

- **[verified]** **RT-507 (M) — `operant service install` writes a unit that can never start.**
  `crates/operant-cli/src/cmd_service.rs:170-195` writes
  `~/.config/systemd/user/operant.service` with `ExecStart={exe} daemon`; the CLI has no `daemon`
  subcommand (no `Daemon` variant in `crates/operant-cli/src/main.rs:200-600`). `systemctl --user
  start operant` exits 2, and `Restart=on-failure` + `RestartSec=5` turns that into an endless
  restart loop. It is also a second unit beside the real one (`operant gateway install` writes
  `operant-gateway.service` with `ExecStart={bin} gateway run`, `cmd_gateway.rs:500-523`).
  *Fix:* delegate `operant service install` to `cmd_gateway::cmd_install` and delete the duplicate
  installer.

- **[verified]** **RT-508 (M) — the gateway runs one agent turn at a time for the whole machine.**
  Every chat's turn awaits one global `tokio::sync::Mutex<()>` (`gateway_runner.rs:2226-2235`,
  taken at `:2562`). One busy chat blocks every other chat, every platform and the cron agent; 100
  sessions produce a 99-deep queue and the queue is unbounded. The code names the upgrade path
  (one agent per seat via `create_runtime_agent_with`, `main.rs:2131`) and a starvation counter
  (`queue_waits`, `gateway_runner.rs:1963`) — but the counter is not observable from
  `gateway status --deep` (`cmd_gateway.rs:296-336` reads config, pid file, systemd and process
  memory only), so the documented "measure first" gate cannot be read when the decision is made.
  *Fix:* one agent per seat, keeping one shared `McpManager` and one registry per seat.

- **[verified]** **RT-509 (M) — no server drains; a restart aborts the turn in flight.** SIGTERM →
  `stop_gateway()` (`cmd_gateway.rs:607-626`) → `Gateway::stop()` (`gateway/mod.rs:365-390`) →
  adapters' no-op `stop()` (`gateway/webhook.rs:306-309`, `telegram.rs:576-580`) → `main` returns →
  the tokio runtime drops every spawned turn and every in-flight request. No
  `axum::serve(...).with_graceful_shutdown(...)` exists anywhere in the tree. The dashboard has no
  signal handler at all and leaves its pid file behind (`cmd_dashboard.rs:109-113`). The agent
  already has a graceful interrupt path (`agent/run.rs:1154` checks the flag; `agent/events.rs:244`
  commits the interrupted tail) — the gateway never triggers it, so tool side effects already
  happened while the answer never arrives.
  *Fix:* on SIGTERM trigger the interrupt flag, await the in-flight turn with a bound, then stop
  adapters; give each listener a `oneshot` shutdown channel.

- **[verified]** **RT-510 (M) — nothing records that a turn was in flight.** The schema has no turn
  table (`database.rs:30-93`); `session_events` receives only a throttled activity heartbeat
  (`agent/builders.rs:296-328`). The assistant's tool-call row is written before the tool runs
  (`agent/run.rs:2099-2111`) and the tool-result row only after it returns (`:2814-2841`), with no
  journal and no idempotency key. A kill between the two leaves an assistant turn with `tool_calls`
  and no results; at load, `drop_orphaned_tool_messages` strips those calls and deletes the message
  if it had no prose (`agent/message_safety.rs:588-632`). The model is told the tool never ran, so it
  re-issues a write that already happened. The turn is neither re-executed nor marked skipped: it is
  silently lost.
  *Fix:* write a placeholder tool row carrying the `tool_call_id` before execution ("pending"), then
  rewrite it with the result; a resume reports an identified-but-unfinished call instead of deleting
  it.

- **[verified]** **RT-511 (M) — the interrupted marker is written by exactly one code path, and it is
  the wrong one.** `write_marker` / `mark_session_interrupted` are reached only from the TUI's exit
  teardown (`tui/app/mod.rs:1541`, `:1558`, `:1588` — the only non-test writers). SIGKILL, SIGTERM, a
  panic and every gateway exit write nothing. After a kill there is no marker, so the next launch
  offers no resume notice, the session renders as `Idle`, and `operant sessions list --interrupted`
  prints "No interrupted sessions" — the exact question the feature exists to answer, answered
  wrongly. The only surviving trace is the presence JSON, which no boot path reads.
  *Fix:* write the marker at turn start and clear it on a clean exit, so a SIGKILL leaves it behind.

- **[verified]** **RT-512 (M) — a dropped turn lease wedges that chat forever.**
  `gateway/lifecycle.rs:171-180`: `TurnLeaseGuard::drop` removes the key with `try_lock`; on
  contention the removal is skipped and the key stays in the map. `try_acquire` refuses any present
  key, so that session answers "⏳ Still working on your previous message" for every future
  message. The comment "a stale lease self-heals on the next turn" is false — nothing implements it.
  *Fix:* hold the key set in a `std::sync::Mutex` so `Drop` can remove without awaiting.

- **[verified]** **RT-513 (M) — every CLI session is a whole operant process; there is no server to
  attach to.** `operant chat`, `operant run`, `operant mcp serve` and every ACP client start their
  own process, which builds its own tool registry (`main.rs:1185-1225`), opens the app DB plus
  `operant_cron.db` plus `operant_kanban.db`, creates its own AFT bridge pool and kernel sidecar,
  and connects its own MCP children. 100 CLI sessions = 100 processes × 57 MB × that child set;
  nothing is shared. ACP already exists as the protocol and already owns session ids.
  *Fix:* one daemon surface owning the registry, DB handles, AFT pool, kernel runtime and
  `McpManager`, serving ACP; the CLI becomes a thin client.

- **[verified]** **RT-514 (M) — ACP rebuilds an agent, a runtime and the child fleet per prompt.**
  `cmd_acp.rs:70-100` runs `Runtime::new()` inside `spawn_blocking`; `:110-150` then builds
  `Database::init` (full migrations), `McpManager::new`, `build_registry` (AFT bridge + detached
  warmup + kernel sidecar + MCP children) and a new model client. Ten prompts = ten runtimes, ten
  AFT spawns, ten migration runs, ten MCP handshakes — all discarded. The MCP server already does it
  right: it builds the registry **once** (`mcp_serve.rs:83-101`).
  *Fix:* build the agent and runtime once per ACP server process; widen the handler seam to take the
  session id.

- **[verified]** **RT-515 (M) — the 30 s tool timeout cancels a non-cancel-safe MCP read and kills the
  MCP surface for the session.** `ToolRegistry::execute_with_timeout` wraps every tool in
  `tokio::time::timeout` and **drops** the future on expiry (`tools.rs:475-486`); the registry
  default is 30 s (`config.rs:1966`). The MCP client reads with `io.stdout.read_line`
  (`mcp.rs:1144-1148`), which tokio documents as not cancel-safe ("some data may have been partially
  read, and this data is lost"), under its own 60 s budget (`mcp.rs:1395-1399`) — so the registry
  always cancels first. One server that takes 35 s to answer leaves the stream one response behind;
  every later call fails `serde_json` → `ReqError::Protocol` (`mcp.rs:1414-1419`), and the
  auto-reconnect fires only on `ReqError::Transport` (`mcp.rs:1243`), the timeout counter is not
  bumped, and the watchdog only restarts a child whose *process* exited. One slow server disables
  every MCP tool for the rest of the session, with no error the model can act on.
  *Fix:* make the read cancel-safe (`read_until(b'\n')` + a manual UTF-8 check), treat a parse or
  desync failure as transport-level so one reconnect is attempted, and keep the client's read budget
  below the registry timeout.

- **[verified]** **RT-516 (M) — removing an MCP server respawns its child outside the manager.**
  `mcp_tool.rs:202-220` calls `McpManager::remove_server` (`mcp.rs:2537-2545`), which kills the
  child and drops the transport but never re-syncs the registry, so the `mcp_<server>/<tool>` entries
  stay registered holding a clone of the dead client. That client's `io` is `None` while
  `had_session` is still true, so the next call takes the transport branch and **auto-reconnects** —
  spawning a fresh child for the server the operator just removed (`mcp.rs:1243-1272`). The new child
  is not in `McpManager::servers`, so the watchdog never watches it, `/mcp` does not show it, and
  `remove_server` can never kill it again.
  *Fix:* unregister the removed server's namespaced tools from every registry holding them, and reset
  `had_session` in `disconnect()`.

- **[verified]** **RT-517 (S) — every registry build duplicates its child fleet, and the gateway runs
  two watchdogs over one manager.** `build_registry` creates a fresh `AftBridgePool`
  (`main.rs:1264`) and calls `kernel::register`, which builds a fresh `KernelRuntime` whose
  `GLOBAL_RUNTIME.set` silently fails on the second call (`tools/kernel/mod.rs:213-237`). The AFT
  pool also fires a detached warmup per root that is designed to outlive operant and is deduped only
  per pool (`aft_bridge.rs:1068-1090`, `:1093-1149`). The gateway builds two agents
  (`gateway_runner.rs:3894-3908`, `:4030-4050`) over one shared `McpManager` (`:3993`), and
  `build_registry` spawns an MCP watchdog per build (`main.rs:1501-1512`) — the watchdog is **on by
  default** (`config.rs:1305`, 30 s). Two watchdogs sweep the same server map, and `reconnect()` is
  not serialised against itself: `kill_child` then `connect`, with `child` and `io` written under two
  separate locks (`mcp.rs:906-923`, `:982-986`), so an interleaving can leave `child` pointing at one
  process and `io` at another's pipe.
  *Fix:* hoist one `AftBridgePool` and one `KernelRuntime` per process (the memory-pair hoist at
  `gateway_runner.rs:3837-3845` is the proven pattern), start the watchdog once per manager, and hold
  one mutex across the kill→spawn→re-handshake sequence.

- **[verified]** **RT-518 (S) — the dashboard is a disconnected process.** `gateway_running` reads the
  *dashboard's own* process-global runner static (`dashboard_server.rs:661`, `:788`; the static is
  `gateway_runner.rs:3336-3340`), so the page always shows "gateway: stopped" even with the daemon
  live. Its handlers do synchronous `read_dir` and SQLite I/O on the async runtime
  (`dashboard_server.rs:689-724`) and `KanbanDb::init` sets no `busy_timeout`
  (`kanban/db.rs:149-159`), so a concurrent writer fails instantly and `Err(_) => continue` drops that
  board from the response (`dashboard_server.rs:712-714`). Its cookie is `operant_dashboard_token`
  (`:801`); the gateway API accepts only `operant_dashboard_session` (`gateway_api.rs:50`, `:67`) and
  mints an independent token, so the API's documented cross-surface use cannot authenticate.
  *Fix:* read `gateway.pid` + `systemctl --user is-active operant-gateway` as `cmd_gateway.rs:305-308`
  does; wrap handler bodies in `spawn_blocking`; add `PRAGMA busy_timeout` to `KanbanDb::init`; accept
  both cookie names.

- **[verified]** **RT-519 (S) — the gateway session API re-runs migrations per request, and `?cap`
  neither bounds the read nor returns the newest messages.** Each GET calls `Database::init`
  (`gateway_api.rs:123`, `:183`), which opens the connection, sets four pragmas, tightens three file
  permissions, reconciles columns, runs the migration runner and creates six tables plus the FTS
  index (`database.rs:137-186`, `:208-245`) — every request takes the SQLite **write** lock against
  the daemon's own turn writes (500 ms busy timeout), so a client polling `/api/sessions` every
  second can return `500 {"error":"database is locked"}`. The `cap` is applied *after*
  `get_session_messages` selects the whole transcript `ORDER BY timestamp ASC`
  (`database.rs:898-919`), so a 50,000-message session materialises 50,000 structs per request and
  returns the **first** 100, while the comment says the cap exists "so a curl cannot stream the whole
  history".
  *Fix:* hold one `Arc<Database>` in `GatewayApiState`; push the cap into SQL
  (`ORDER BY timestamp DESC, id DESC LIMIT ?`, then reverse).

- **[verified]** **RT-520 (S) — a webhook bind failure is invisible, and status prints an address
  nothing listens on.** The bind happens inside `start_with_channel` (`gateway/webhook.rs:828-838`);
  an error is logged and handed to the retry supervisor, which retries every 5 s → 300 s forever
  (`gateway/mod.rs:429-469`). `cmd_status` prints "Webhooks: enabled (listen on localhost:0)" from
  config alone (`cmd_gateway.rs:376-389`) while the adapter's real default is `127.0.0.1:8080`
  (`gateway/webhook.rs:256`) — a common port. If anything holds it, inbound events never arrive, the
  only evidence is one journal line, and every status surface says enabled.
  *Fix:* treat a bind failure as a hard start failure for inbound adapters, print the effective
  address in status, and probe the listener.

- **[verified]** **RT-521 (S) — unbounded growth under load, and the cap counts turns rather than
  sessions.** The inbound channel is `mpsc::unbounded_channel::<IncomingMessage>()`
  (`gateway_runner.rs:4183`); the per-session FIFO is an unbounded `VecDeque` (`:5174`, `:5624`); the
  `max_concurrent_sessions` lock is taken per message and released when the turn ends
  (`gateway/mod.rs:642-668`), so it caps in-flight turns — and the default is `None`
  (`config.rs:1563`, `:1613`). `ProcessRegistry` prunes only *finished* entries (`MAX_PROCESSES = 64`)
  and has no `Drop` and no `kill_on_drop` (`process_registry.rs:144-256`, `:243`), so running
  background children are unbounded and outlive the agent. Ten thousand 1 MB webhook POSTs while the
  agent is slow ≈ 1 GB of queued messages → OOM, then `Restart=always` loses the queue.
  *Fix:* a bounded channel with a typed drop, a per-session queue cap, a real default cap, and a
  `Drop` for `ProcessRegistry` that group-kills running entries.

- **[verified]** **RT-522 (S) — the presence registry is never claimed, and its races are unhandled.**
  The TUI claims a session at boot guarded on a session id that does not exist yet (the id is minted
  by the first turn) and never claims again on `/new` or a hub switch
  (`tui/adapter_types/tui_app.rs:308-316`, `:1411`, `:1446`), so in practice **no process ever claims
  a session** and the liveness guard is inert. `register_presence` overwrites unconditionally
  (`active_sessions.rs:227-234`), liveness is "/proc/<pid> exists" (`:314-316`), and the lock
  filename is the lossy `sanitize` of the id (`:142-152`), so distinct ids differing only in `:` vs
  `_` share one lock file. `acquire()` also scans the whole lock directory — a `stat` per entry — on
  every inbound message (`:47-99`, `:112-139`): ~2000 syscalls per message at 1000 live sessions.
  *Fix:* claim on session switch and `/new`; refuse a claim whose current owner's pid is alive;
  record the owner's start time to defeat pid reuse; hash the id for the filename; keep the live
  count in an `AtomicUsize` and prune on a timer.

- **[verified]** **RT-523 (S) — the daemon keeps no state file, and the API prints its token before it
  binds.** The delivery ledger (500 entries) and the queue-wait stats are RAM-only and die with the
  process (`gateway/mod.rs:375-387`); `spawn_if_enabled` prints the bearer token and *then* binds,
  logging a bind failure as a warning (`gateway_api.rs:246-254`). After a restart, "how many replies
  failed?" and "how many turns were cut off?" are unanswerable; a port conflict on `:8731` prints the
  token line followed only by a WARN, so the operator believes the API is up.
  *Fix:* one `gateway.status.json` the daemon writes on every turn end and on shutdown; bind the API
  before printing the token and log the failure at ERROR.

- **[verified]** **RT-524 (S) — one persistence rule applied four different ways.** The live path
  writes full rows with `tool_call_id`/`tool_calls`/`tool_name` (`agent/events.rs:210-242`), but the
  rewind fork (`agent/rewind.rs:179-200`) and the interrupted-tail commit
  (`agent/events.rs:312-324`) use the short writer that drops them — so a rewound branch has **no tool
  history** and the operator's "what did you change?" has no answer. Mid-turn steers and
  reasoning-only nudges are never persisted at all (`agent/run.rs:3116-3127`, `:2051-2067`), so the
  live conversation and the DB transcript are different histories. Post-run autocompaction writes only
  to the resident copy, has no production caller, and its generation counter disagrees with the
  durable metadata it writes (`session/autocompact.rs:546-576`, `:717-726`).
  *Fix:* write every transcript path through `save_message_full`; persist steer/nudge rows at
  injection; make the compaction marker durable and wire the scheduler to it — or delete the module.

- **[verified]** **RT-525 (N) — about 1,350 lines of implemented, tested, unwired machinery.**
  `session/autocompact.rs` (766 lines) is driven only by its own integration test;
  `persistence_seam.rs` (217 lines) is documented UNWIRED (RT-359) and referenced only by
  `pub mod` in `lib.rs`; `classify_turn_error` and `SessionStallTracker` have zero production callers
  (`gateway/lifecycle.rs:14-26`, `:188-236`). The modules honestly document their own unwired state,
  which means the next reader assumes they run.
  *Fix:* wire the stall detector into the drain of RT-509 (it is exactly the "this turn is wedged"
  signal a drain needs) and the compaction into the scheduler; delete `persistence_seam` — or delete
  all three.

- **[verified]** **RT-526 (N) — the daemon lock file is unlinked on drop, and the docs describe a
  gateway that does not exist.** `flock` protects the open inode, but `Drop` removes the path
  (`gateway_runner.rs:1892-1897`, `:1918-1935`), so a process holding the old inode and a new starter
  on a new inode can both hold "the lock" — the duplicate daemon the flock was added for. The docs
  claim "a singleton gateway instance persists across CLI commands in one process" and cite a
  `gateway status --memory` flag that does not exist (`docs/library/gateway.md:44`, `:49`), and
  `docs/library/operability.md:59-62` points at the broken unit of RT-507.
  *Fix:* do not unlink the lock file; correct the three sentences.

## Also spotted, not numbered

Eleven further defects were verified and are held for the next sweep: a deliberate `/exit` clears the
interrupted marker and the teardown immediately rewrites it (`tui/app/commands.rs:761-779` +
`tui/app/mod.rs:1770-1783`); the CDP browser page is a process-wide `OnceCell` shared by the gateway
agent and the cron agent, which runs outside the turn lock (`sourcehound_cdp.rs:400-408`);
`McpManager::registered_tool_names` is one manager-wide list shared by two registries
(`mcp.rs:2326-2343`, `:2586-2615`); two independent sourcehound children exist when sourcehound is
also configured as an MCP server (`sourcehound.rs:172-182` + `mcp.rs:2494-2513`); the agent's
rehydrate is bounded to `min(2000 messages, 2 MiB)` while `sessions show`, `sessions resume` and the
hub preview load every row (`session/mod.rs:96-108` vs `cmd_sessions.rs:227-251`); pending approvals
and staged writes are process-local (`write_approval.rs:9-11`); the TUI's standing `/goal` is
process-global, unpersisted and not cleared on a session switch (`tui/app/commands.rs:895-936`); the
warm cache has a load-outside-the-lock TOCTOU (`session/mod.rs:279-308`, `:413-431`);
`messages.active` is declared, indexed and never honoured (`database.rs:90-91`); no child-owning type
has a `Drop` impl and MCP children get no process group or `PDEATHSIG`, so a parent SIGKILL orphans
every child (`mcp.rs:932-945`); Ctrl+C to a foreground gateway orphans Holon-supervised bodies
(`operant-os/src/supervisor.rs:135-139`).

## Test coverage of the risky surfaces

The unit layers are well covered; the integration layers this audit is about are not.

| Risky logic | Owner file | Unit tests | Integration test |
|---|---|---|---|
| Turn lease / lifecycle | `gateway/lifecycle.rs` | 11 | none |
| Active sessions + presence | `active_sessions.rs` | 14 | none |
| Interrupted marker | `session_interrupted.rs` | 6 | none |
| Turn-state journal | `gateway_runner/turn_state.rs` | 9 | none |
| MCP client | `mcp.rs` | 30 | none |
| Database | `database.rs` | 81 | — |

The gateway tests prove the lock and the start/stop wiring
(`test_gateway_start_stop_with_disabled_platforms`, `second_gateway_daemon_lock_is_refused_until_released`,
`the_daemon_lock_survives_the_start_functions_return`, `gateway_runner/tests.rs:945`, `:1047`,
`:1108`) — the first starts the gateway with platforms **disabled**, so no turn ever runs. Three
gaps, each matching a finding above: no restart-with-in-flight-turn test (RT-509/510/511); no test
of the registry timeout against a live MCP server (`registry_timeout_secs` appears in zero test
files — RT-515); no multi-process contention test (RT-522).

## Verdict

Fix RT-507, RT-508, RT-509, RT-510 and RT-513 first. RT-507 is a two-minute fix that currently does
active damage. RT-508 (one turn at a time), RT-509 (no drain), RT-510 (no in-flight record) and
RT-513 (no shared server) are one workstream: the drain needs per-seat agents to drain into, the
per-seat agents need a shared server to be a per-machine resource rather than an N-fold copy, and the
in-flight record is what makes a resumed turn honest instead of silent. The persistence layer
underneath is the surprise — transcripts survive, tool-call ids are repaired on load, the session
store and the session API work. What is missing is the *process* layer that would let many sessions
share one runtime and survive its death. After those, RT-515 and RT-516 are the two defects that
bite a long-running daemon hardest.
