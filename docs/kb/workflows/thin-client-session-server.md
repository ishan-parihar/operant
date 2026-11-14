# Workflow: thin-client-session-server

`genome wf add thin-client-session-server "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

**What this campaign is.** The audit
(`docs/audit/2026-11-13-server-process-architecture-scale-audit.md`) measured
operant against jcode and codex and found one gap: the channel side is a
server, and the interactive side is a full stack per session. Thirteen full
`operant` processes, 792 MiB RSS, thirteen separate AFT bridges. The plan
(`docs/plans/2026-11-13-thin-client-session-server-plan.md`) is ten
iterations that give the terminal a server to talk to.

**The order, and why.** The protocol comes first, so the client has something
to speak (I1-I2). Then the server grows a socket (I3) and a session a client
can drive (I4). Then the client attaches (I5) and the hard parts cross the
socket (I6-I7). Last come the merges and the number (I8-I10).

**The rule each iteration obeys.** One iteration = one commit + one push + one
named regression test, verified with `cargo fmt --all`, a scoped `cargo check`,
the scoped tests, `cargo clippy`, `python3 scripts/audit-ledger-index.py` and
`./scripts/genome lint`. The iteration label comes from `origin/main` at commit
time, never from local history.

**The cost rule.** An attaching client must build NO runtime: no registry, no
database handles, no MCP children, no AFT bridge. If an attach path builds one,
the scale story is false again, and the regression test for that is in the
iteration's list.

**The ledger.** RT-542 (the interactive surfaces build the whole stack), RT-543
(two daemons), RT-544 (the plan and a comment promise what does not exist),
RT-545 (the protocol cannot carry an interactive session), RT-546 (the event
type cannot cross a socket), RT-547 (the thin client has no caller), RT-548
(32 sessions over 4 slots), RT-549 (three runtime builders), RT-550 (cancel
lies), RT-551 (no published session cost), RT-552 (the read-only session API).

| id | status | item | claim | verdict |
|---|---|---|---|---|
| thin-client-session-server-001 | done | I1: the agent's event type becomes a wire type (AgentEvent serialisable) | — | [2026-11-14] docs/audit/2026-11-13-server-process-architecture-scale-audit.md |
| thin-client-session-server-002 | done | I2: the session protocol as data (client command, server event, versioned envelope) | — | [2026-11-14] docs/plans/2026-11-13-thin-client-session-server-plan.md |
| thin-client-session-server-003 | done | I3: the gateway serves a client socket (two clients, one process, one child set) | — | [2026-11-14] docs/plans/2026-11-13-thin-client-session-server-plan.md |
| thin-client-session-server-004 | done | I4: a session on the server that a client can drive (two clients share one live session) | — | [2026-11-14] docs/plans/2026-11-13-thin-client-session-server-plan.md |
| thin-client-session-server-005 | done | I5: the TUI can attach (--attach; an attached client builds no registry) | — | [2026-11-14] docs/plans/2026-11-13-thin-client-session-server-plan.md |
| thin-client-session-server-006 | done | I6: approve, deny and cancel cross the socket (cancel really cancels) | — | [2026-11-14] docs/CHANGELOG.md |
| thin-client-session-server-007 | done | I7: the client dies, the session lives (grace window, a new client joins it) | — | [2026-11-14] docs/CHANGELOG.md |
| thin-client-session-server-008 | open | I8: one server, one child fleet (merge the three runtime builders; the ACP daemon folds in) | — | — |
| thin-client-session-server-009 | done | I9: the slot count and the session cap agree (one number, stated once) | — | [2026-11-14] docs/plans/2026-11-13-thin-client-session-server-plan.md |
| thin-client-session-server-010 | done | I10: the number is published (a session-memory bench script and its table) | — | [2026-11-14] docs/CHANGELOG.md |
