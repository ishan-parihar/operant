# Persistent-gateway session and memory management — jcode contrast audit

Dated-frozen: audited `2026-10-26`; findings mark verified vs asserted.
Investigator: operator agent, ponytail-audit mode (whole-repo over-engineering +
operational-gap scan, scoped to the persistent gateway/server).

## Scope

One question: **what does the jcode reference implementation
(`../external-repos/jcode`) do at the server/session/memory layer that operant's
persistent gateway does not, and which of operant's existing shapes are
over-engineered or fragmented for that goal?** Everything below is static
analysis with file:line citations plus one live measurement (the deployed
gateway daemon on this box). No fixes applied — ponytail-audit is one-shot.

The user-facing symptoms this audit answers to:

1. each new spawned session must not duplicate the running operant instance
   (memory bloat),
2. server-level session management across TUI + gateway (telegram, slack, etc),
3. background agentic runs,
4. memory-management gaps and fragmentation, so everything can be
   operationalized.

## The architectural contrast (one paragraph each side)

**jcode** is a two-process model. ONE long-lived `jcode serve` daemon owns
every agent: a `SessionAgents = Arc<RwLock<HashMap<String, Arc<Mutex<Agent>>>>>`
map keyed by session id (`crates/jcode-app-core/src/server.rs:121`), where each
client connection either creates or claims a session agent
(`server/client_lifecycle.rs:626-630`). TUIs, SDK clients, and the harness API
are THIN CLIENTS over a Unix socket (`server/socket.rs:7-99`); `spawn_server`
(`src/cli/dispatch.rs:1231-1410`) is idempotent — probe the socket, take a
flock spawn lock, double-check, spawn once; a second server refuses to start
("Another jcode server process is already running", `socket.rs:310-320`, the
`flock` held for the daemon's whole lifetime at `socket.rs:284-306`). Memory
is managed as a first-class subsystem: idle-session hibernation discards
rebuildable caches after 30 min (`server/idle_hibernation.rs`, threshold
`jcode-config-types/src/session.rs:44`), an idle worker reaper shuts down
abandoned swarm agents (`server.rs:278-366`), a stale-socket reaper uses the
daemon lock as the liveness witness (`socket.rs:137-180`), the allocator is
tuned at startup (jemalloc decay / glibc `M_ARENA_MAX=4` + pinned mmap
threshold, `src/main.rs:5-79`) and `allocator::trim()` (debounced
`malloc_trim`) runs after every client disconnect
(`jcode-harness-api-server/src/allocator.rs:1-40`), and a memory-incident
classifier attributes RSS growth to a cause
(`server/debug_server_state.rs:361-403`).

**operant** is a one-process-per-surface model. The gateway daemon
(`operant gateway run`) owns exactly ONE `Arc<OperantAgent>` shared by every
platform chat (`gateway_runner.rs:3003`, `GatewayMessageHandler.agent`), with a
process-global `turn_lock` serializing ALL chats to one turn at a time
(`gateway_runner.rs:1758-1766`, "ponytail: one turn at a time"). Session state
is persisted (SQLite session store + 90-day retention sweep,
`gateway_runner.rs:398-417`) but the in-process maps are unbounded:
`session_channels` (insert-only, `gateway_runner.rs:4898`, zero removals),
`exit_reasons` (insert-only, `:2322/:3692`), the gateway `SessionStore`
metadata map (`gateway/types.rs:321-336` — `close_session` exists at `:421-425`
but has ZERO production callers), `session_pins`, and the per-thread
stream/usage maps. The TUI, `operant run`, `operant chat`, cron, and the
autonomous mode each build their OWN agent + `McpManager` + memory pair in
their OWN process (`main.rs:2892`, `chat_non_tui`; `main.rs` agent builds;
`gateway_runner.rs:2974` gateway+cron agents share one memory pair but a
second `McpManager`). There is NO daemon lock: duplicate-prevention is a pid
file (`gateway_runner.rs:5256-5261`) consulted only by `gateway status/stop`
(`cmd_gateway.rs:248-256`) — `gateway run` itself never checks it, so two
gateways run happily side by side, double-polling Telegram and double-consuming
memory. There is no hibernation, no idle reaper, no allocator tuning, no
`malloc_trim` anywhere in daemon paths (verified: zero hits for `malloc_trim`
outside the vendored TUI snapshot module that only markdown hooks read).

## Ponytail-audit findings — ranked, biggest cut first

### Gap findings (operant is MISSING these; ranked by operational value)

1. **gap: no single-instance enforcement.** A second `operant gateway run`
   starts unconditionally: `start_gateway` (`gateway_runner.rs:2828+`) checks
   only the in-process `RUNNER` static (`:2846-2852`), never the pid file, and
   `cmd_run` (`cmd_gateway.rs:513+`) just calls it. Two daemons double-poll
   Telegram/Discord (the long-poll adapters fight over getUpdates offsets),
   double-hold model credentials, and double the RSS. jcode's fix is ~30 lines:
   `flock` beside the socket/pid, refuse if held (`jcode-harness-api-server/
   src/lib.rs:179-229`). The pid file already exists (`gateway.pid`,
   `gateway_runner.rs:1206-1208`) — it is written (`:5256-5261`) and read for
   status, but never consulted as a lock. This is THE dedup gap the user's
   "each new spawned session does not duplicate the running operant instance"
   names, and it is the cheapest close.
2. **gap: no session-agent map — one shared agent serializes the whole
   gateway.** jcode: N sessions, N agents, per-session busy refusal
   (`send_agent_busy_error`, `client_lifecycle.rs:233-253`). operant: one
   agent, one global turn lock (`gateway_runner.rs:1758`), every other chat
   QUEUES behind it with only a typing indicator. The turn lease
   (`gateway/lifecycle.rs:116-157`) already exists per session key at the
   GATEWAY layer — the handler just never uses an agent per lease. The
   upgrade path is already written down at the lock site ("an agent per seat
   (the create_runtime_agent seam)"); the gateway session swap
   (`set_session_id`, `agent/events.rs:748-795`) already rehydrates/persists
   per session correctly, so an `agents: HashMap<session_key, Arc<OperantAgent>>`
   keyed the same way `turn_lease` keys would carry the same correctness
   contract and lift the serialization. Until a busy chat starves others, the
   ponytail verdict is: measure queue wait first (the upgrade comment says
   exactly this).
3. **gap: no idle RAM management.** jcode hibernates idle sessions' caches
   (30 min default, `idle_hibernation.rs:17-80`), reaps abandoned swarm
   workers (`server.rs:278-366`), trims the allocator after disconnects
   (`allocator.rs`), and tunes arenas at startup (`src/main.rs:30-79`).
   operant: the warm session cache is bounded (32, `session/mod.rs:74`) and
   that is the ONLY bound. The insert-only per-session maps above, the
   metadata-only gateway `SessionStore`, and the `PersistentSessionStore`
   in-memory `entries` map (bounded only by the 90-day DB sweep,
   `gateway_session.rs:1900-1940`) grow for the daemon's lifetime. A
   `malloc_trim` call after each turn + a periodic `session_channels`
   sweep of dead sessions is the 80% of jcode's subsystem at ~5% of its code.
4. **gap: no socket/client protocol — every surface is a separate process.**
   jcode's TUI is a thin client; operant's TUI builds a full agent + MCP
   manager + memory pair per invocation (`tui_app.rs:181-190`, `main.rs`
   agent builders). The user's "consolidates several running TUI, or gateway
   sessions" needs this seam eventually — but a full socket protocol is a
   BIG build. The ponytail call: do findings 1-3 first; they deliver the
   memory-bloat relief without the protocol. The thin-client TUI is the
   multi-week item to schedule deliberately, not the first move.
5. **gap: no memory-incident observability.** jcode classifies RSS growth
   (`classify_memory_incident`, `debug_server_state.rs:361-403`) and exposes
   a `server memory` payload. operant's vendored `process_memory.rs` (ported
   FROM jcode, `tui/operant_app/process_memory.rs:1-16`) is read by NOTHING
   in daemon paths — only the TUI markdown hooks touch it. A `gateway status
   --memory` line reusing that module is ~20 lines and makes finding 3
   measurable instead of guesswork.

### Over-engineering findings (delete:/shrink:/yagni: — things to NOT build or to cut)

- **yagni:** do not port jcode's swarm persistence, channel subscriptions, or
  group-transfer machinery to close this gap. operant has no swarm members;
  the 6,665-line jcode server surface (`server/`, 90+ files) is that big
  BECAUSE of swarm/reload/upgrade orchestration operant does not have. The
  minimal gateway needs findings 1-5 only. Target: ~150 lines total, not a
  66K-line port.
- **delete:** the second `McpManager` in the gateway process
  (`gateway_runner.rs`, `cron_mcp_manager`) — the D-5 memory-pair hoist
  (iter-98) did this exact consolidation for memory; MCP servers are
  subprocesses, and two managers mean two stdio MCP server PROCESSES per
  configured server. Same shape as the D-5 fix, one field.
- **shrink:** the pid-file timestamp block (`gateway_runner.rs:5256-5261`)
  computes `SystemTime` then never uses it (comment admits "not timestamp").
  Dead ceremony around the one line that matters; fold into the flock fix
  (finding 1).
- **yagni:** `ActiveSessionTracker` (`active_sessions.rs:25-80`) — a
  file-lock-based cross-process session cap that is OFF by default
  (`max_concurrent_sessions: None`, `config.rs:1310`) and overlaps the
  in-process `TurnLease`. Keep ONE mechanism; the lease is the live one.
- **shrink:** the four insert-only per-session maps
  (`session_channels`, `exit_reasons`, `session_pins`, stream/usage maps)
  share one lifecycle question (when does a session's entry die?) answered
  in four places. One bounded per-session state struct with a TTL sweep
  replaces all four answers — and finding 3's sweep needs exactly that
  struct.

## What is already right (do not touch)

- The session swap (`set_session_id`) is correct and bounded: warm cache 32,
  LRU eviction, DB write-through (`session/mod.rs:222-270`, `events.rs:748-795`).
- The 90-day retention sweep finally calls `prune_old_entries`
  (`gateway_runner.rs:398-417`) — the growth hole the DB side had is closed.
- The per-session turn lease + stall tracker + delivery ledger
  (`gateway/lifecycle.rs:116-195`) are live, tested, and the right primitives
  for finding 2's agent-per-seat keying.
- The turn_lock serialization (plan 06A) is CORRECT for the current
  one-agent design — it fixed real cross-session corruption. Finding 2 is
  the upgrade path, not a bug fix.

## Live measurement (the deployed daemon, 2026-10-26)

`operant-gateway.service` (pid 3825510, up 52 min, 27 threads): VmRSS 10.7 MB,
VmHWM 35.5 MB. The one-agent design is currently CHEAP — because it is
serialized and the warm cache is small. The bloat risk is not the running
daemon today; it is (a) duplicate daemons (finding 1), (b) unbounded map
growth over a 90-day retention window at multi-chat scale (finding 3), and
(c) per-process duplication when TUI/cron/autonomous run beside the gateway
(finding 4, each a full agent + MCP subprocesses + memory pair).

## Operationalization order (the fix plan this audit feeds)

1. flock single-instance guard at `gateway run` (refuse when held; pid file
   stays for status). ~30 lines + test. Closes the dedup gap outright.
2. Trim + sweep: `malloc_trim` (debounced, jcode's `allocator.rs` shape, ~40
   lines vendored) after each turn; TTL sweep for the insert-only maps.
3. `gateway status --memory` over the vendored snapshot module. Makes 2
   observable.
4. Agent-per-seat at the `create_runtime_agent` seam, keyed by the existing
   turn-lease session key, busy-refusal like jcode. Only if queue-wait
   measurement (step 3's observability) shows starvation.
5. Thin-client TUI over a socket — schedule deliberately; biggest build,
   last.

net: +~150 lines to close findings 1-3 and 5 (the daemon-side value),
against a 66K-line reference surface deliberately NOT ported.
