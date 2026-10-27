# Session management red-team audit v2: corrections + the server question answered

Dated-frozen: published `2026-10-28`. This audit SUPERSEDES
`docs/audit/2026-10-28-session-management-red-team-audit-all-surfaces-vs-jcode-oh-m.md`
(kept for the record). Companion plan (also v2):
`docs/plans/2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md`.

## Why v2

The v1 audit's five findings (RT-A fragmentation, RT-B redundant modal,
RT-C missing session id on App, RT-D no subagent surface, RT-E employee
identity gateway-only) all held up on re-verification. Three details in the
remediation *suggestions* were overstated, and v1's scope answer on the
"server" half of the operator's question was wrong. Corrections:

## Corrections to v1

1. **`seat_id()` already exists** — `agent/events.rs:684-689`
   (`#[expect]`-annotated read of the seat RwLock). v1's remediation text
   implied iter-A would add "one getter" to `agent/events.rs`. False — the
   write point only CALLS it. iter-A touches no agent file.

2. **The `sessions` table has its own additive-reconciliation mechanism** —
   `reconcile_columns()` (`database.rs:312-343`) diffs live columns against
   `DESIRED_SCHEMA_SQL` and ADDs missing ones idempotently (duplicate-column
   errors are the expected re-run path), on top of the append-only
   `DATABASE_MIGRATIONS` runner (`migrations.rs:58`). v1 said "copy the
   gateway's `ensure_employee_binding` ALTER" — wrong tool: the sessions DB
   already reconciles from a single declared schema. The `employee_id` column
   rides `DESIRED_SCHEMA_SQL` + reconcile. (The gateway's own table keeps its
   hand-rolled `ensure_*` — different DB file, different history, not ours to
   refactor in this plan.)

3. **`get_session_messages(session_id)` already exists** — `database.rs:872`
   (plus `get_session_messages_full` at :898). v1's finding #7 said the preview
   pane "needs `db.list_session_messages(id, cap)`" as if it were new. It is
   a capped SELECT over an existing query, not a new capability.

4. **One v1 claim needs precision, not retraction** — v1 §1.7 said F0066's
   flock gap was closed. True, and the cite (`gateway_runner.rs:1812-1880`)
   is right, but the closure is a PEER's iter (landed after F0066 was filed,
   carrying its own iter-1345 process-lifetime regression fix). The v1 text
   read as if this audit verified an operant-original design; it verified a
   peer's fix that already matches jcode's `single_instance_lock` shape
   (`jcode-harness-api-server/src/lib.rs:225-250`). Credit where due.

## The server question, answered (v1 dodged it)

The operator's ask: "I want to implement the unified server/session-management
implementation like that of jcode." v1 §4 excluded any server as speculative.
That was answering a different question ("does operant need one today?") than
the one asked ("give me jcode's session architecture"). Re-derived facts:

- jcode's actual server surface: `jcode-harness-api-server` — a
  single-instance **socket bridge** (flock beside the socket, 0600 perms,
  accept loop, wire-API translation over the daemon's legacy socket:
  `lib.rs:179-296`), plus the session-pid presence registry the picker reads.
- operant already owns every ingredient: the gateway daemon's
  `GatewayDaemonLock` (same flock discipline), axum + `TcpListener` +
  `axum::serve` in-tree (`dashboard_server.rs:124-133`), and the dashboard's
  token auth pattern (AD-040).
- Therefore the honest ponytail answer is: **one route group on the existing
  daemon, not a new server** — `GET /api/sessions`, `GET
  /api/sessions/:id/messages`, `GET /api/subagents`, read-only, token-gated,
  behind the daemon's existing flock. That is the F0066 gap-4 thin-client
  seam at bridge cost, and it is what the plan v2's iter-E builds. What stays
  out (and stays named): jcode's 66K-line harness surface, stdio bridging,
  translate layers, and moving session ownership server-side.

## Findings (unchanged from v1, cited there, not re-argued)

RT-A through RT-E stand as written in v1 §1 and §3, with the remediation
corrections above folded into plan v2. The ranked list is unchanged:

1. RT-B redundant 70x20 modal (delete, replace with full-frame picker)
2. RT-D no subagent surface (omp hub model)
3. RT-A record fragmentation (SessionEntryV2 + one loader)
4. RT-C no current-session id on App (one field, one sync)
5. RT-E employee tags (`sessions.employee_id` via reconcile)
6. ←← gesture (omp's 500ms window + armCloseTap handoff)
7. Preview pane (over `get_session_messages`, capped)

## Correction hygiene note (for the record)

v1 was pushed as iter-1428 and is append-only history — it is not edited.
The v2 audit + v2 plan are the correction mechanism, per the same law the
fleet used for F0095→F0096 (self-correction as a new append, never a rewrite).
The INDEX carries the supersede markers.
