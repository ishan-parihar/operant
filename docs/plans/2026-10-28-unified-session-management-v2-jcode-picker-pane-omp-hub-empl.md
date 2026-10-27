# Unified session management v2: jcode picker pane + omp hub + employee tags + the session server phase

Dated-frozen: published `2026-10-28`. This plan is registered in genome.toml
`core.living_plans` — its five iterations are open, so corrections land as
typed amendments below (each dated and attributed) rather than as same-day
v3 files; the dated-frozen body above them is the frozen design.
Supersedes `docs/plans/2026-10-28-unified-session-management-jcode-style-session-picker-pane-o.md`
(archived with tombstone). Companion audit:
`docs/audit/2026-10-28-session-management-red-team-audit-v2-corrections-the-server-.md`
(supersedes the v1 audit — same corrections led to this plan's v2).

## Why v2 (what v1 got wrong)

v1 excluded the jcode-style unified server/session management as YAGNI. The
operator's ask named it explicitly ("I want to implement the unified
server/session-management implementation like that of jcode"), so the server
phase is IN scope — scoped ponytail-minimal (below), but in. v1 also carried
three overstated details, corrected here:

1. **`seat_id()` already exists** (`agent/events.rs:684-689`) — iter-A needs
   NO change in `agent/events.rs`; the write point reads an existing getter.
2. **The `sessions` table already has additive reconciliation**:
   `reconcile_columns()` diffs live columns against `DESIRED_SCHEMA_SQL`
   (`database.rs:312-343`) and ADDs what's missing, plus the append-only
   `DATABASE_MIGRATIONS` runner (`migrations.rs:58`). The `employee_id` column
   rides THAT mechanism (add to `DESIRED_SCHEMA_SQL`; the reconcile pass adds
   it to existing DBs) — NOT a hand-copied gateway `ensure_column` (that was
   v1's wording; the gateway's `ensure_employee_binding` was the pattern
   reference only).
3. **`get_session_messages(session_id)` already exists** (`database.rs:872`)
   — the preview pane needs only a capped variant (SELECT the last N by id),
   not a new query family.

## Mechanism

One full-frame page replaces the 70x20 session modal, unifying sessions,
branches, and live subagents into one list — jcode's picker geometry and
mechanics, omp's unified record and ←← gesture, operant's existing switch
plumbing (`set_session_id` retarget — untouched) and seat/employee model.
Then a thin server phase gives that record ONE live authority the way jcode's
harness-api-server does.

### Part 1 — the page (unchanged from v1, corrections folded in)

**1. `SessionEntryV2`** (new `crates/operant-core/src/session/listing.rs`):

```rust
pub struct SessionEntryV2 {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub employee_id: Option<String>, // NEW column; None = the operator
    pub source: String,
    pub origin: Option<String>,
    pub message_count: usize,
    pub cost_usd: f64,
    pub tokens: u64,
    pub cwd: Option<String>,
    pub status: RowStatus,           // Current | Live | Idle | Archived
    pub kind: RowKind,               // Session | Branch | Subagent{goal, model, status}
}
```

- Loader `Database::list_sessions_rich(limit)` — one SELECT (every column
  exists except `employee_id`, added via the reconcile mechanism above).
- Subagent rows join a snapshot of the live delegation registry
  (`async_delegation.rs` — `list_records()` EXISTS at :171; a
  `records_snapshot()` accessor for the TUI is the only addition).
- Preview data: `get_session_messages` (exists) + a capped last-N wrapper.

**2. Employee tag** — `employee_id TEXT` added to `DESIRED_SCHEMA_SQL`
(`database.rs:29-66`); reconcile adds it to existing DBs; write point at
session save reads `agent.seat_id()` (exists, events.rs:684). No second
employee directory (org/seat db stays the source of truth).

**3. The page** — `tui/session_hub/` (copy jcode `session_picker*`, cut the
~60% operant lacks: external-CLI loaders, server grouping, onboarding band,
crash banner, live-pid presence — operant is same-process; `RowStatus::Live`
comes from `is_streaming` + the delegation registry). Full-frame overlay
(journey_view pattern), 40/60 list+preview, filter cycle reduced to
`All | CurrentDir | ByEmployee | Active | Archived`, tokenized search,
cached-then-refresh over the existing `session_list_rx` mpsc (app/mod.rs:580).
omp roster/focus split: Enter on a Subagent row = focus view (goal/status/
result tail); Enter on a Session/Branch row = existing
`pending_switch_session` flow (tui_app.rs:1125 — untouched). `/session`
`/resume` `/sessions` route here; `session_browser.rs` DELETED.

**4. The ←← gesture** — omp's shape, burst-proof: key_handling.rs Left arm
(:1617) plain-Left + `prompt_input` empty + cursor 0 → tap counter on App;
the SECOND tap fires only inside omp's window `[40ms, 500ms)`
(`LEFT_DOUBLE_TAP_MIN_GAP_MS`/`MAX_GAP_MS`, input-controller.ts:265-266,
`#detectLeftDoubleTap` :902-923) — the 40ms floor rejects
terminal-synthesized arrow bursts (a stray click emitting a ←-run never
fires; a third-and-later tap before a quiet gap resets). `armCloseTap`
handoff seeds the hub's close detector so ONE further ← closes it
(agent-hub.ts:626-636, :2467); focused-subagent ←← returns to the list.
jcode's config-gated single-Left variant NOT ported.

### Part 2 — the session server phase (jcode parity, ponytail-scoped)

What jcode actually has (source: `jcode-harness-api-server/src/lib.rs:179-296`):
a single-instance **socket bridge** — flock beside the socket
(`single_instance_lock`, kernel-released on death), 0600 socket perms, an
accept loop translating a wire API over the daemon. Plus a session-pid
presence registry the picker reads (`session_picker_presence.rs`).

What operant already has for this (verified):

- a long-lived daemon with the SAME flock discipline:
  `GatewayDaemonLock`/`acquire_gateway_daemon_lock()`
  (gateway_runner.rs:1812-1880) — kernel-released, process-lifetime guards;
- axum + `TcpListener::bind` + `axum::serve` in-tree (dashboard_server.rs:124-133);
- the dashboard's token auth pattern (startup token, `Authorization: Bearer`,
  HttpOnly cookie exchange — AD-040).

So the ponytail server shape is: **no new daemon**. `operant gateway run`'s
existing process gains one axum route group — `GET /api/sessions`
(`list_sessions_rich`), `GET /api/sessions/:id/messages?cap=N` (capped
`get_session_messages`), `GET /api/subagents` (registry snapshot) — bound to
`127.0.0.1` with the dashboard's token discipline, behind the SAME
`GatewayDaemonLock` the daemon already holds. The TUI keeps its direct-DB
fast path (spawn_blocking + mpsc) and does NOT grow a client for its own
process's data — the API exists for the OTHER surfaces (dashboard page, a
future second TUI attaching to live sessions, scripts), which is the F0066
gap-4 thin-client seam, built at bridge-not-server cost.

Explicitly out (still): jcode's 66K-line harness surface (swarm reload,
stdio bridge, translate layer), a second listener process, session
ownership moving server-side. Sessions stay where they are; the server
EXPOSES the unified record, it does not own it.

## Gates / success criteria

(fixed before the run)

- G1: `/resume` opens the full-frame page at 110x45; 40/60 list+preview;
  `session_browser.rs` deleted; overlay guard table updated; no dead flag.
- G2: rows group by employee tag; `ByEmployee` filter works; a session
  created under a seat shows that id; personal sessions show none; the
  column appears on an EXISTING db without manual migration (reconcile).
- G3: subagent rows appear while a background delegation is live; Enter
  focuses; ←←/Esc returns; rows prune with the registry.
- G4: Enter on a session row retargets via `pending_switch_session`;
  transcript rehydrates; existing regression tests stay green.
- G5: double-tap ← on empty input opens the page, honoring omp's full
  window `[40ms, 500ms)` (two taps < 40ms apart = burst, never fires);
  ONE further ← inside the page closes it (handoff); ←← with text in the
  composer changes nothing.
- G6: scoped tests green; clippy gate passes; `command_sweep.py` 98/98 holds
  (command names unchanged).
- G7: 50-row cap gone (default 500 + search).
- G8 (server): with the gateway daemon running,
  `GET /api/sessions` returns the same rows the picker shows (minus
  subagents when none), auth-gated like the dashboard; the flock means no
  second listener can bind; with the daemon DOWN, the TUI is unchanged
  (direct-DB path — no regression when the server is absent).
- G9: `operant.example.toml` gains the documented `[gateway] api` block
  (bind address + enable flag), defaulting to on for `gateway run` — no new
  top-level config surface.

## Cost

Five iterations, ordered against the peer's dirty set (F0094:
key_handling, app/mod, app/commands, app/init, tui_app, terminal_setup):

1. **iter-A (core, zero TUI overlap)**: `SessionEntryV2` +
  `list_sessions_rich` + `employee_id` in `DESIRED_SCHEMA_SQL` (reconcile
  adds it) + seat write point + registry snapshot accessor + capped
  messages wrapper + unit tests. Files: `database.rs`,
  `session/listing.rs` (new), `async_delegation.rs`. NO agent/events.rs
  change (getter exists).
2. **iter-B (the pane)**: `tui/session_hub/` copy+refactor; `/resume`
  routes here; delete `session_browser.rs`. Files: new module,
  `render/operant_overlays.rs`, `dialog_routing.rs`, `app/mod.rs`,
  `app/commands.rs` (re-grep peer state first).
3. **iter-C (subagent rows + focus view)**: registry join, `RowKind::Subagent`,
  focus view, tests.
4. **iter-D (the gesture)**: Left arm + tap state (min-gap 40ms burst
  floor + 500ms ceiling) + handoff + G5 tests.
5. **iter-E (the server routes)**: axum route group on the gateway daemon,
  dashboard-token auth, `[gateway] api` config block, `curl`-able smoke
  test + a router unit test per route. Files: `gateway_runner.rs` (router
  assembly), `dashboard_server.rs` (token helper reuse), `operant.example.toml`.

### CLI surface ownership (amended 2026-10-28 — a gap in the v2 body)

`cmd_sessions.rs` (12 subcommands: List/Show/Delete/Stats/Export/Prune/
Rename/Browse/Search/Recap/Resume) consumes `db.list_sessions()` today and
moves to `SessionEntryV2` **in iter-A** — one loader, every surface. iter-A
adds the `list_sessions_rich` call sites for List/Stats/Show (the same
`database.rs` edit, no extra iteration); Browse/Resume keep their TUI/agent
flows untouched. WITHOUT this amendment the plan's own "only the TUI/CLI
`sessions` surface moves to the v2 record" line had no owner — the audit's
RT-A fragmentation would survive the plan by hiding in the CLI.

## Risks

- Peer WIP on the TUI files — iteration order above; re-grep before iters B/D.
- jcode picker assumes multi-session servers; grouping reduced to
  employee/source. If full server-side sessions ever land, `ServerGroup`
  returns at the grouping seam — Part 2's API is the door, not the room.
- The gateway daemon is the operator's live channel surface (7 platforms):
  iter-E touches only router assembly + a new route group, never the platform
  adapters or the turn pipeline; the routes are read-only.
- Auth: the dashboard token pattern is reused verbatim; the API never accepts
  a request without it (127.0.0.1 binding is defense-in-depth, not the gate).
- The `sessions.employee_id` column is NULL for all existing rows by design
  (no backfill lie: we do not know who ran them; the gateway keeps its own
  table with its own binding history — G+1 backfilled `premiere` there, but
  THIS table's rows predate seats and stay honestly NULL).

## Resume here

All five iterations open. Start iter-A (zero peer overlap; note the CLI
amendment above — `cmd_sessions.rs` call sites ride the same `database.rs`
change). iter-E can run independently any time after iter-A (it only needs
the loader + snapshot).

## Amendments (living-plan log)

- **2026-10-28, agent-jcode (self-review round 2):** (1) gesture window
  corrected to omp's full `[40ms, 500ms)` with the 40ms burst floor
  (input-controller.ts:265-266, :902-923) — v2 body said only "500ms",
  which would have re-introduced the terminal-burst bug omp's floor
  exists to reject; (2) close semantics corrected: `armCloseTap` makes
  ONE further ← close (agent-hub.ts:626-636, :2467), not "the same ←←"
  (the body contradicted its own G5); (3) `cmd_sessions.rs` ownership
  added (the 12-subcommand CLI surface had no iteration owner — RT-A
  would have survived in the CLI).
