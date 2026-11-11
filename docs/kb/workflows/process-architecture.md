# Workflow: process-architecture

`genome wf add process-architecture "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| process-architecture-001 | done | RT-507 (M): operant.service installer writes a unit that can never start (ExecStart=... daemon, no such subcommand) + infinite restart loop | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | [2026-11-12] docs/audit/2026-11-12-process-architecture-audit-gateway-server-session-management.md |
| process-architecture-002 | in-progress | RT-508 (M): gateway runs one agent turn at a time machine-wide (global turn_lock) | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-003 | in-progress | RT-509 (M): no server drains — a restart aborts in-flight turns; no with_graceful_shutdown anywhere | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-004 | in-progress | RT-510 (M): no durable record of an in-flight turn; a mid-flight kill leaves tool calls the transcript denies | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-005 | in-progress | RT-511 (M): the interrupted marker is written only by the TUI teardown — SIGKILL/SIGTERM/panic/gateway exit write nothing | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-006 | done | RT-512 (M): a dropped turn lease (try_lock on Drop) wedges a chat forever | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | [2026-11-12] docs/audit/2026-11-12-process-architecture-audit-gateway-server-session-management.md |
| process-architecture-007 | in-progress | RT-513 (M): every CLI session is a whole operant process; no server to attach to | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-008 | in-progress | RT-514 (M): ACP rebuilds an agent + tokio runtime + child fleet per prompt | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-009 | in-progress | RT-515 (M): the 30s registry timeout cancels a non-cancel-safe MCP read_line — permanent desync, no recovery | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-010 | in-progress | RT-516 (M): removing an MCP server leaves stale tools that respawn its child outside the manager | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-011 | in-progress | RT-517 (S): every registry build duplicates its child fleet (AFT bridge+detached warmup, kernel sidecar) and runs a 2nd MCP watchdog | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-012 | in-progress | RT-518 (S): the dashboard is a disconnected process — wrong gateway state, blocking I/O, swallowed board errors, cookie the API rejects | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-013 | in-progress | RT-519 (S): the gateway session API re-runs migrations per request; ?cap neither bounds the read nor returns the newest N | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-014 | in-progress | RT-520 (S): a webhook bind failure is invisible; status prints an address nothing listens on | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-015 | in-progress | RT-521 (S): unbounded growth — inbound channel, per-session queue, ProcessRegistry running children; the cap counts turns not sessions | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-016 | in-progress | RT-522 (S): the presence registry is never claimed, and its races are unhandled (last-writer-wins, pid reuse, name collisions, per-message dir scan) | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-017 | in-progress | RT-523 (S): the daemon keeps no state file (delivery ledger + queue-wait stats are RAM-only); the API prints its token before it binds | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-018 | in-progress | RT-524 (S): one persistence rule applied four ways — rewind fork and interrupted tail use the short writer; steers/nudges/goal never persisted; autocompact resident-only with no caller | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-019 | in-progress | RT-525 (N): ~1,350 lines of implemented, tested, unwired machinery (autocompact, persistence_seam, stall tracker, turn-exit classifier) | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
| process-architecture-020 | in-progress | RT-526 (N): the daemon lock file is unlinked on drop (two-daemon race); docs describe a gateway that does not exist | claimed-by `agent-omarchy-wt2096` since 2026-11-12T01:33 | — |
