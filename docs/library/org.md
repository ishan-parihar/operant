# The org layer — Holon runtime, seats, work graph

Owns: operant's organization surface — the Holon boundary and what
survived of the native org layer (seat policy, budgets, authority),
kanban, the curator, and cron.
Not for: the agentic core ([agent-loop.md](agent-loop.md)), context
injection ([context-injection.md](context-injection.md) §10 covers only
the injection seam), the kernel sidecar ([kernel.md](kernel.md)).

Source: `crates/operant-core/src/org/` (mod, seat_policy.rs,
seat_policy_db.rs, seat_budgets.rs, seat_authority.rs,
context_injection, worklog.rs), `crates/operant-core/src/kanban/`
(mod, db, dispatcher, diagnostics, notify, triage),
`crates/operant-core/src/cronjobs/`
(mod, db, lock, schedule, scheduler, suggestions),
`crates/operant-core/src/curator/` (mod, archiver, backup, review),
`crates/operant-os/src/` (the Holon runtime crate:
`api.rs` — the read-only Holon API client over the `api_*` views;
`supervisor.rs` — the per-org run supervisor that claims, spawns
and leases bodies; `policy.rs` — `policy.json` → the live seat policy
phase C; `enforcement.rs` — the seat authority mounted from that
policy; `chair.rs` — the Chair's channel identity resolution and
dispatch; `contract.rs` — the vendored `result.json` JSON Schemas;
`archive.rs` — phase G one-time copy of the org stores),
`crates/operant-cli/src/cmd_org.rs`, `cmd_kanban.rs`, `cmd_cron.rs`,
`cmd_budget.rs`. Read FIRST: `docs/agentic-os/README.md` and
`docs/agentic-os/BOUNDARY.md`.

## The boundary in one paragraph

Holon (a stdlib-Python kernel) holds the *organization* — departments,
employees, the work graph, approvals, verification, playbooks,
objectives, memory promotion. Operant is the *runtime* the organization
lives in: the bodies of its employees, the security boundary,
enforcement, metering, the clock, events, and the owner's channel. The
rule: a capability belongs in operant when it is resident, per-turn or
per-tool-call, acting on the OS for agents, or real-time with the
outside world; it belongs in Holon when it is an organization decision
made per stage/tick, run as a short-lived process. There is one
organization, one owner per capability, one writer per database.

## What survived the supersession (`org/mod.rs`)

The G+1 keep-shape after Holon superseded the native organism:

- **`seat_policy`** — the pure policy engine; the enforcement core for
  Holon's `policy.json` (phase C).
- **`seat_policy_db`** — personal-mode operator seats; org mode never
  consults it (the run's `policy.json` IS the policy).
- **`seat_budgets`** — budget enforcement, one row per seat:
  NULL-able fields pin just one knob and inherit the rest from
  `[bylaws].budget`; `cap = 0` means explicitly ungoverned (the
  documented release valve). Write surface: `operant budget` +
  `operant org budget set/list/clear`.
- **`seat_authority`** — the escalation core: `SeatAuthority::consult`
  returns the policy-driven verdict for one tool call.
- **`context_injection`** — DM/feed context quotas (see
  [context-injection.md](context-injection.md) §10).
- **`worklog`** — the org work log surface.

## `operant-os` — the Holon runtime crate

The Rust half of the boundary contract: the read API over Holon's
`work.db` (opened SQLite `mode=ro` — operant never writes a Holon
store; the invariant is STRUCTURAL), `api_version` negotiation
(refusing unknown versions), every query naming an `api_*` view, plus
`policy` (phase C), enforcement, events, supervisor, chair, and the
native Holon tools generated from ONE definition
(`vendor/holon-tools.json` — Holon's own `holon.mcp.tools()` walk; the
vendor file is the contract so the two sides cannot drift).

Two files under `operant-os/src/` are the residency contract the
boundary paragraph above names: `organs.rs` is the resident-organs loop
(the supervisor and the 2-minute `holon tick` clock, living in the
gateway process rather than a CLI one-shot), and `run_env.rs` is the
run-environment contract (the `holon_employee` / `holon_run_dir` /
`is_holon_run` predicates plus the department-resolved `holon_skills_dir`,
re-exported from `operant_core::platform` where the single definition
sits because the agent loop and the cron tool read it there).

All Holon-backed surfaces are OFF by default (`[holon] enabled =
false`): personal mode is byte-identical to mainline.

## Kanban, cron, curator, suggestions

- **Kanban** (`kanban/`) — the task board the agent and operator both
  work (`operant kanban list/show/create/complete/block/comment/link/
  stats`; the `kanban` tool for the agent). `kanban/db.rs` is the store
  (`KanbanDb`, `Task`, `Run`, `Comment`, `Event`); `dispatcher.rs` is the
  worker engine (`pending_tasks`, `claim_task`, `complete_run`, `fail_run`,
  `gc`, with the claim's audit insert deliberately best-effort outside the
  failure path); `diagnostics.rs` reports stale running tasks, zombie runs
  and orphaned links with the command that fixes each; `notify.rs` holds
  per-task platform subscriptions; `triage.rs` builds the LLM triage
  context and prompt for one task.
- **Cron** (`cronjobs/`) — the scheduler: cron expressions are calendar
  rules in UTC; `every N <unit>` phrases are fixed durations. The
  scheduler skips due jobs while ESTOP is set; `operant cron run <id>`
  is the accepted bookkeeping stub (RT-135 — do not "fix" unasked).
  `cronjobs/db.rs` is `CronDb` (jobs, runs, deliveries, occurrence claims,
  and the skill-reference rewrite the curator's archive guard reads);
  `cronjobs/schedule.rs` validates a schedule and computes the next run,
  canonicalizing 5-field cron input to the 6-field form it persists;
  `cronjobs/lock.rs` names the gateway daemon's `flock` in ONE place,
  which is why the lock's presence means "the cron scheduler is running
  here".
- **Curator** (`curator/`) — skill lifecycle: states, pinning, usage
  tracking, backup/rollback snapshots, LLM consolidation of overlapping
  skills. Fed by the background review. The archive/prune/restore
  mechanics are `curator/archiver.rs`; `curator/backup.rs` is the
  snapshot-and-rollback pair; `curator/review.rs` is the LLM review seam
  (`LlmReviewClient`, `ModelReviewClient`, and the consolidation
  verdicts).
- **Suggestions** (`cronjobs/suggestions.rs` + `cmd_suggestions.rs`) —
  ready-to-run cron specs surfaced to the user; accepting one creates
  the real job.

## Running the cron cast

`operant cron list` shows every job, `create <name> "<schedule>"
"<command>"` adds one, and `get` / `history <id>` / `update` /
`delete` / `pause` / `resume` manage it. A schedule is either a cron
expression read in UTC (`"0 9 * * *"` is 09:00 UTC) or a fixed
duration (`"every 6h"`). `operant cron run <id>` runs a job now,
`status` prints the total/active/paused counts, `tick` asks the
scheduler for due jobs, and `blueprint morning-brief|weekly-digest|
reflection` creates one from a template. The board beside it:
`operant kanban list/show/create/complete/block/comment/link/stats`
(`--board <slug>`, default `default`). `operant curator status` and
`operant curator run` drive the skill lifecycle, `operant org budget
set/list/clear` provisions a seat's cap, and `/cron`, `/kanban`,
`/curator` and `/org` shell out to those commands from the TUI.

## Intention

The org layer is operant's answer to "many agents, one mission": the
boundary exists so the organization's *decisions* stay rewritable
without a compile (Holon's Python) while everything resident,
enforcing, or real-time stays in one Rust binary. The supersession
(G+1, 2026-10-15) deleted the second organization the native layer had
grown — seats/grants/notices as a parallel world — and kept exactly the
enforcement shapes operant is uniquely positioned to own. The phase
state and open work packages live in `docs/agentic-os/` (AUDIT-2026-10-15,
capability-layer audit, remaining gaps).
