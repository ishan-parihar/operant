# The org layer — Holon runtime, seats, work graph

Owns: operant's organization surface — the Holon boundary and what
survived of the native org layer (seat policy, budgets, authority),
kanban, the curator, and cron.
Not for: the agentic core ([agent-loop.md](agent-loop.md)), context
injection ([context-injection.md](context-injection.md) §10 covers only
the injection seam), the kernel sidecar ([kernel.md](kernel.md)).

Source: `crates/operant-core/src/org/` (mod, seat_policy,
seat_policy_db, seat_budgets, seat_authority, context_injection,
worklog), `crates/operant-os/src/` (the Holon runtime crate),
`crates/operant-core/src/kanban/`, `crates/operant-core/src/cronjobs/`,
`crates/operant-core/src/curator/`,
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

All Holon-backed surfaces are OFF by default (`[holon] enabled =
false`): personal mode is byte-identical to mainline.

## Kanban, cron, curator, suggestions

- **Kanban** (`kanban/`) — the task board the agent and operator both
  work (`operant kanban list/show/create/complete/block/comment/link/
  stats`; the `kanban` tool for the agent).
- **Cron** (`cronjobs/`) — the scheduler: cron expressions are calendar
  rules in UTC; `every N <unit>` phrases are fixed durations. The
  scheduler skips due jobs while ESTOP is set; `operant cron run <id>`
  is the accepted bookkeeping stub (RT-135 — do not "fix" unasked).
- **Curator** (`curator/`) — skill lifecycle: states, pinning, usage
  tracking, backup/rollback snapshots, LLM consolidation of overlapping
  skills. Fed by the background review.
- **Suggestions** (`cronjobs/suggestions.rs` + `cmd_suggestions.rs`) —
  ready-to-run cron specs surfaced to the user; accepting one creates
  the real job.

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
