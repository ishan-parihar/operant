# process architecture: integrated gateway server, safe drain, shared session runtime

Dated-frozen: published `2026-11-12`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Audit this plan implements: `docs/audit/2026-11-12-process-architecture-audit-gateway-server-session-management.md`.
Board that tracks it: `docs/kb/workflows/process-architecture.md` (20 items, one per finding
RT-507…RT-526). This plan is the execution order for that board.

## Mechanism

**Target architecture.** One long-lived **operant runtime daemon** per machine owns everything that
is expensive and everything that must survive a restart:

```
operant runtime daemon (one per machine; flock + pid file + status file)
├── tool registry            ONE, shared by every session
├── AftBridgePool            ONE (per project root), shared
├── KernelRuntime            ONE, shared
├── McpManager               ONE — one child per configured MCP server
├── SQLite handles           app + cron + kanban, opened once
├── session store            durable transcripts + in-flight turn records
├── agents                   one per seat/session (pooled, bounded)
├── gateway adapters         telegram, discord, …, webhook HTTP server
├── session API              read + WRITE (turn submission) over loopback
└── ACP server               the attach point for every CLI session
```

A CLI session (`operant chat`, `operant run`, an editor) becomes a **thin client**: it speaks ACP to
the daemon over a unix socket, holds no registry, spawns no child, and keeps only view state. The
daemon is the single writer for every session row; the client never opens the database.

**Safe drain.** On SIGTERM the daemon: (1) stops accepting new turns (the ingress turns read-only),
(2) marks every in-flight turn `draining` durably, (3) triggers each agent's interrupt flag so the
turn commits its partial tail through the existing graceful path, (4) waits up to a bounded window
for the turns to finish, (5) writes `gateway.status.json` (in-flight count, queue depth, delivery
totals, drain outcome), and (6) exits. A turn that does not finish in the window is recorded as
`interrupted` with its `tool_call_id`s, so the next start resumes it as an **identified unfinished
call** — never as a tool that never ran.

**Concurrency.** One agent per seat, bounded by a semaphore; the per-session turn lease stays (one
turn per session), the global `turn_lock` goes. The MCP children, the registry, the AFT bridge and
the kernel sidecar are shared by every seat.

## Where each finding landed

The iteration numbers in the table below are the labels this plan ASSIGNED. Peers
took those labels while the work was in flight, so the numbers that actually
carry the commits are in the ledger's index table
(`docs/audit/2026-06-19-BUGS.md`). Every finding below is CLOSED with a named
regression test:

| Finding | Shipped in | Finding | Shipped in |
|---|---|---|---|
| RT-507 installer | iter-2101 | RT-516 MCP respawn | iter-2107 |
| RT-508 one turn per machine | iter-2125 | RT-517 child fleet | iter-2176 (re-landed) |
| RT-509 no drain | iter-2113 | RT-518 dashboard | iter-2143 |
| RT-510 in-flight turn | iter-2112 | RT-519 session API | iter-2134 |
| RT-511 interrupted marker | iter-2111 | RT-520 webhook bind | iter-2127 |
| RT-512 turn-lease leak | iter-2102 | RT-521 unbounded growth | iter-2144 |
| RT-513 shared session server | iter-2118 | RT-522 presence registry | iter-2149 |
| RT-514 ACP runtime | iter-2117 | RT-523 status file | iter-2131 |
| RT-515 MCP timeout desync | iter-2105 | RT-524 one transcript writer | iter-2151 |
| | | RT-525 unwired machinery | iter-2152 |
| | | RT-526 flock unlinked | iter-2142 |

The red-team pass over this campaign's own output added RT-533…RT-536
(iter-2139, iter-2137, iter-2136), the checkpoint-list row RT-541 (iter-2171),
the file-state row RT-538 (iter-2158), the reload-decision race (iter-2155),
and RT-527's follow-on — all closed. `docs/audit/2026-11-12-process-architecture-red-team-execution.md`
is the report.

## Execution order

Each iteration is one commit on `origin/main`, one push, one release build, one deploy, one gateway
restart (the repo's iteration loop). Every iteration ships a regression test that fails without the
fix.

| # | Iteration | Findings | What changes | Regression test |
|---|---|---|---|---|
|1|iter-2101 (planned iter-2097)|RT-507|`operant service install` delegates to the gateway installer; the broken `daemon` unit and its duplicate installer are deleted|a unit-content test: the generated unit's `ExecStart` names a subcommand the CLI actually has (`operant gateway run`)|
|2|iter-2102 (planned iter-2098)|RT-512|the turn-lease key set moves to a `std::sync::Mutex` so `Drop` removes without `try_lock`|a lease test that drops the guard while the map is contended, then asserts the key is re-acquirable|
|3|iter-2105 (planned iter-2099)|RT-515|the MCP stdio read becomes cancel-safe; a parse/desync failure is treated as transport-level and forces one reconnect; the client read budget sits under the registry timeout|a stdio-client test: a server that answers late leaves the stream usable for the next call (no permanent parse errors)|
|4|iter-2107 (planned iter-2100)|RT-516|`remove_server` unregisters the server's namespaced tools and resets `had_session`; a removed server cannot respawn|a manager test: remove a server, call the stale tool, assert no child was spawned and the tool is gone|
|5|iter-2112 (RT-510), iter-2111 (RT-511) — planned iter-2101|RT-510, RT-511|a pending tool row is written before execution; the interrupted marker is written at turn start and cleared on a clean exit|a DB test: a half-written turn rehydrates as an identified unfinished call, not as a deleted message; a marker test: the marker survives a simulated kill|
|6|iter-2113 (RT-509), iter-2125 (RT-508) — planned iter-2102|RT-509, RT-508|per-seat agent pool behind a semaphore; the global `turn_lock` becomes per-session; SIGTERM drains (interrupt flag → bounded wait → status file)|a restart test: a turn in flight, the daemon is killed, the next start reports it as interrupted and the transcript keeps the call/result pairing|
|7|iter-2118 (planned iter-2103)|RT-513|the daemon serves ACP over a unix socket; `operant chat`/`run` attach to it instead of building their own runtime|a two-process test: one daemon, two attached clients, one shared MCP child set|
|8|iter-2176 (planned iter-2104)|RT-517|one `AftBridgePool`, one `KernelRuntime`, one MCP watchdog per process|a test that two registry builds share one AFT pool and one kernel runtime|
|9|iter-2143 (RT-518), iter-2134 (RT-519), iter-2127 (RT-520) — planned iter-2105|RT-518, RT-519, RT-520|the dashboard reads the daemon's status file instead of its own process state; the API holds one `Database` and caps in SQL; the webhook bind failure is a hard start failure and status probes the listener|a dashboard test: with a live daemon, `gateway_running` is true; an API test: `?cap=N` returns the newest N|
|10|iter-2144 (planned iter-2106)|RT-521, RT-522|bounded ingress channel, bounded per-session queue, `ProcessRegistry::Drop`; presence is claimed on switch, refuses a live owner, hashes the filename, and keeps the count in memory|a load test: 1,000 queued messages stay bounded; a contention test: two claims for one session, the second is refused|
|11|iter-2131 (RT-523), iter-2151 (RT-524), iter-2142 (RT-526) — planned iter-2107|RT-523, RT-524, RT-526|the daemon writes `gateway.status.json`; the API binds before printing its token; every transcript path uses `save_message_full`; steers and nudges persist; the lock file is not unlinked|a persistence test: a rewind branch keeps its tool history; a steer survives a restart|
|12|iter-2152 (planned iter-2108)|RT-525|the unwired surfaces are wired or deleted, with before/after line counts|the audit's own counts, re-derived after the cut|

## Gates / success criteria

**Per iteration (the repo's loop):** `cargo fmt --all`; scoped `check` for the crate touched; the
new regression test green; `bash scripts/clippy-warning-gate.sh` with no NEW warnings;
`./scripts/genome lint` green for doc-touching iterations; commit with `<type>(iter-N)`; push to
`origin/main`; release build; deploy to `~/.local/bin/operant`; `systemctl --user restart
operant-gateway`; provenance check (artifact mtime newer than the commit, and the running process
executes the deployed binary).

**End of the run — the four operator requirements, each with a check:**

1. *One integrated gateway + server.* CHECK: `operant status` shows the daemon, the API and the
   dashboard as one runtime; `ps` shows one `operant` process for N CLI sessions.
2. *A restart drains safely.* CHECK: start a long turn, `systemctl --user restart operant-gateway`,
   and the session's next turn sees an **identified** interrupted call (not a silently dropped tool),
   and the daemon's status file records the drain outcome.
3. *100–1000 parallel agents.* CHECK: N concurrent gateway sessions run N turns concurrently (the
   queue-wait counter stays near zero), and the process count and child count stay flat.
4. *Shared MCP/CLI tools.* CHECK: N attached CLI sessions share one MCP child set, one AFT bridge
   and one registry (counted with `pgrep`).

**Honest limits.** The 100–1000 figure is a *target*, not a measured claim until the load test in
the final iteration runs on this box. The plan does not promise a number it has not measured; the
gate is that the counters exist and are flat.

## Cost

- **Lines:** the plan adds the drain, the per-seat pool, the daemon's ACP socket and the status
  file. It also removes the duplicate service installer and, in the final iteration, the unwired
  surfaces (RT-525: `session/autocompact.rs` 766 lines, `persistence_seam.rs` 217 lines, the two
  unwired lifecycle surfaces ~250 lines, the duplicate installer ~120 lines) — **-1,350 lines** of
  dead machinery against the new runtime work.
- **Dependencies:** none added. `axum`, `tokio`, `rusqlite` and `tokio-util` (already present for
  the websocket feature) cover the socket and the shutdown channels.
- **Time:** twelve iterations. The two heavy ones are #6 (per-seat pool + drain) and #7 (the ACP
  attach path); each needs a workspace-wide verification and a release build.
- **Risk:** #6 and #7 touch the agent's session-identity and interrupt paths, which the TUI also
  uses. Their regression tests must run against the TUI's resume path too, or the TUI's resume
  silently changes behaviour.
