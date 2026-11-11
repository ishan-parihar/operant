---
ID: "AD-044"
Title: "one runtime daemon per machine owns the registry, the child processes and the session writes; every CLI session attaches to it"
Status: "Active"
Date: "2026-11-12"
Description: "one runtime daemon per machine owns the registry, the child processes and the session writes; every CLI session attaches to it"
---

# AD-044: one runtime daemon per machine owns the registry, the child processes and the session writes; every CLI session attaches to it

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The 2026-11-12 process-architecture audit
(`docs/audit/2026-11-12-process-architecture-audit-gateway-server-session-management.md`) measured
operant's process model against four operator requirements and found all four unmet. Operant ships
one 57 MB binary that every host starts as its own OS process:

- `operant chat` / `operant run` / `operant mcp serve` / every ACP client each build their own tool
  registry, open the app DB plus `operant_cron.db` plus `operant_kanban.db`, create their own AFT
  bridge pool and kernel sidecar, and connect their own MCP children — nothing is shared and the
  cost is linear in session count (RT-513).
- The gateway daemon is the only shared process, and it serialises every chat's turn behind one
  global `turn_lock` because one shared `Arc<OperantAgent>` carries one hot conversation slot
  (RT-508).
- On SIGTERM the daemon stops its adapters and returns; the runtime drops every in-flight turn and
  request. No server in the tree uses `with_graceful_shutdown`, nothing records that a turn was in
  flight, and the interrupted marker is written only by the TUI's exit teardown (RT-509, RT-510,
  RT-511).
- The dashboard is a separate process that reports the gateway as stopped because it reads its own
  process-global runner static (RT-518).

The 2026-10-28 unified-session-management v2 plan already scoped a "session server phase" as future
work and deliberately did not build it ("operant's sessions are same-process today; a session server
is a deployment feature with its own risk profile"). That phase is now the requirement.

## Decision

Operant runs **one long-lived runtime daemon per machine**. That daemon owns every expensive and
every crash-sensitive resource:

- the tool registry (built once), the app/cron/kanban SQLite handles (opened once), one
  `AftBridgePool`, one `KernelRuntime`, and one `McpManager` — one MCP child per configured server,
  one AFT bridge per project root, one kernel sidecar;
- the durable session store and the in-flight turn records, so it is the single writer for every
  session row;
- one agent per seat/session, pooled and bounded by a semaphore, sharing that registry and those
  children;
- the gateway platform adapters, the session API and the ACP socket that every CLI session attaches
  to. A CLI session becomes a thin client: no registry, no child process, no database handle — view
  state and an ACP connection only.

**Shutdown is a drain, not a drop.** On SIGTERM the daemon stops admitting new turns, marks every
in-flight turn `draining` durably, triggers each agent's interrupt flag so the turn commits its
partial tail through the existing graceful path, waits a bounded window for the turns to finish,
writes `gateway.status.json` (in-flight count, queue depth, delivery totals, drain outcome) and
then exits. A turn that misses the window is recorded as `interrupted` with its `tool_call_id`s, so
the next start resumes it as an identified unfinished call — never as a tool that never ran
(RT-510's pending tool row is the record that makes this possible).

The global `turn_lock` is replaced by the per-session turn lease plus the seat semaphore: one turn
per session, many sessions at once, shared children.

## Consequences

- The fleet's per-session cost stops being per-session: N CLI sessions share one registry, one
  child set and one DB connection set. That is the precondition for the 100–1000 concurrent-agent
  target, which is a *target until measured* — the plan's final iteration measures queue wait and
  child counts on this box and reports them.
- Crash honesty becomes real: a killed turn leaves an identified unfinished call and a status file,
  so the next start answers "what was interrupted?" instead of showing a transcript that denies the
  tool ran.
- The CLI becomes a client, so the TUI's and `run`'s resume paths must be re-tested against the
  daemon's session store — the plan names that risk and its regression tests.
- A new daemon surface is new attack surface: the ACP socket and the session API are loopback + a
  token, and the API gains WRITE routes (turn submission), so its auth and Host checks must reach
  the dashboard's discipline (RT-518, RT-519).
- This makes RT-513's fix a feature, not a cut-down: the audit's lean claim (-1,350 lines of unwired
  machinery) is independent and unaffected.

## Orthogonality

The 2026-10-28 v2 plan scoped a 'session server phase' as future work and deliberately did NOT build it; this decision is made now, on the 2026-11-12 process-architecture audit's evidence, and it adds three requirements the v2 phase never had: the drain-on-restart contract, the per-seat agent pool, and the 100-1000 concurrent-agent target. (recon R1112015541054: runtime daemon shared registry agent pool drain session server ACP attach)

The 2026-10-28 v2 plan scoped a 'session server phase' as future work and deliberately did NOT build it; this decision is made now, on the 2026-11-12 process-architecture audit's evidence, and it adds three requirements the v2 phase never had: the drain-on-restart contract, the per-seat agent pool, and the 100-1000 concurrent-agent target. (recon R1112015541054: runtime daemon shared registry agent pool drain session server ACP attach)

## Revisions

- 2026-11-12: created
