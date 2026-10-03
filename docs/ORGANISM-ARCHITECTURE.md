# Organism OS — Ratified Architecture (owner-validated, iter-609)

> **Status: RATIFIED.** This is the validated plan of record for the organism
> layer. Every session working on it reads this file first. Amendments go
> through the owner, land here, then release as their own iterations.

## Design lineage

Derived from the homed-hermes organism template (`~/.hermes/organism/`,
AD-057/AD-074): four strata (cortex/swarm/foundations/ventures), one
`_org.yaml` per pool, "teams declared in one manifest, roster derived —
never hand-edit". Operant keeps the **cast** and collapses the
**multiplicity**: hermes needed three swarm codebases because it had no org
layer; operant's primitives (employees, hierarchy edges, policy rows,
grants) are the org layer, so each system is a department + employees with
policy rows.

Standing invariants from the genome (iters 573–606) apply to everything
below: authority is data not code; one consult seam; fail closed on
authority, fail open on telemetry; ungoverned is byte-identical legacy;
authority never widened around a check; every decision leaves an audit row.

## 1. Cold-start cast

Seeded idempotently at first run (INSERT-OR-IGNORE seeder, manifest is a
single Rust constant table in `org/cast.rs` — the one-declared-manifest
doctrine, no new file format).

| Seat | Department | Role | Default cron job |
|---|---|---|---|
| `premiere` | — | org-lead; the owner's own working role; seeded with a **standing `Org`-scope grant** (unbounded — minted through `SeatApprover::mint_for`, consistent with the org-lead rule) | daily directive + aspirations review |
| `governor` | meta-governance | policy ratification review, `/audit` owner, doctor-finding triage | daily governance digest |
| `identity-warden` | identity | binds platform users → employees; owns the session↔employee registry (identity-core) | hourly identity audit |
| `compass` | strategy | objectives, priorities, steering (strategy-incubator / Compass) | weekly strategy review |
| `hrmaster` | crew | workforce lifecycle: onboarding provisioning, policy rows, budget defaults, performance/efficacy reviews (workforce-ops) | daily workforce digest |
| `crew-chief` | crew | agent-level governance of spawned crews: spawn depth, crew policy inheritance (agent-fabric folded in) | daily crew telemetry |
| `dispatcher` | crew | task assignment: cron registrations → employees, queue review (task-grid) | hourly queue review |
| `dp-the-program` | execution | autonomous execution engine; parent of all cron automata | continuous (existing) |

**Department ruling (owner):** the three hermes swarm pools (workforce-ops /
agent-fabric / task-grid) collapse to ONE department with three roles.
Owner-directed rename: **the department is `crew`** — organizational
vocabulary agents "feel at home" in (a crew hall for agents), replacing the
engineering term "swarm". `crew-chief` reports to `governor`; `hrmaster` and
`dispatcher` report to `crew-chief`; wardens (`identity-warden`, `compass`)
report to `governor`; `premiere` is above all.

**Self-operationalization (owner directive):** every cast role ships with a
default cron job (cadences above, all configurable/editable) so the organism
learns from its own telemetry and the owner's usage without operator
prompting: the digests are how governor/hrmaster/compass learn, and their
summaries roll into the compaction handoff (§3).

## 2. Employee-level session management

The durable unit of the organism is the **employee**: charter (system
prompt), policy row, budget, reporting edge. A **session** is one
conversation executing *as* an employee.

- `gateway_sessions` gains `employee_id` (backfill = `premiere`). New
  sessions bind by default to **`premiere`** (owner ruling — the owner
  talks to the premiere by default, not a separate assistant role).
- The operator talks to a specific employee via slash command
  (`/session new [employee]` / `/use <employee>` shape — naming finalised
  at implementation). CLI/TUI equivalents consume the same registry.
- **Multiple parallel sessions per employee are legal**; budget and
  authority account at the employee level across them (§5) — that is why
  budgets key on `employee_id`, never session.
- Each employee carries its own `system_prompt` charter (employees table
  gains the column; charters seeded from the cast).

### Seat derivation

`seat = employee_id`. The `gw_<hash(session_key)>` derivation stops being
produced. Existing rows keyed on `gw_<hash>` seats (policies, grants,
requests) are **preserved but inert** — nothing is deleted; the derivation
ceases, and since chats rarely survived long enough to be governed this
loses no ratified authority. Employee ratification is owner-consistent and
permanent: authority attaches to the employee row; a session borrows it.

## 3. Rolling compaction handoff (owner-specified)

- After each agentic loop completes, a session summary is written (reusing
  the `context_management` compressor + the iter-57 grace-call summarizer).
- Summary TTL is **configurable, 15–30 minutes** (default 30). The summary
  *contains* the prior summary's continuity plus what the current session
  did — so the chain compounds implicitly.
- If the user continues the session (restart / reconnect), the loop **begins
  from that summary** plus memory-wire facts. Always the latest one only —
  **rolling, never aggregating** old summaries. Old rows expire by TTL.
- Table: `employee_session_summaries(employee_id, session_key, summary,
  tokens, created_at, expires_at)`; lookup is latest-per-session.

## 4. Onboarding governance

- Cron registration provisions employee + seat + policy row in ONE
  transaction (`dispatcher` role surfaces it; `hrmaster` owns policy
  defaults). Safety config from flags → else `[genome]` default → else
  queued for `hrmaster` review.
- Doctor (unified per §6-F2) gains genome checks: ungoverned cron seats,
  unbound platform identities, budget rows without metering. Findings
  surface through the EXISTING doctor/status surfaces and as agent-prompted
  suggestions — **no new commands** (owner constraint).

## 5. Budgets as policy (owner-expanded)

- Config: per-employee `budget { basis: tokens|usd, window: daily|weekly|monthly, cap, mode: hard|soft }`;
  global default + per-employee override; no row = ungoverned (rule 2).
- Metering reuses the existing per-session accumulator
  (`gateway_session.rs` → `update_session_cost`) rolled up per
  `employee_id` per window — no second counter. `workforce-ops` precedent:
  telemetry already returns per-employee token usage.
- Injection: remaining budget is computed at run start and each iteration
  and placed in loop context ("employee X: 40k of 100k daily tokens
  remain"), so the agent self-economizes *before* a cap is hit.
- **Hard cap:** iteration-boundary check (same seam as the grace call) →
  mandatory summarize-and-exit; unattended runs exit with a budget reason
  and may escalate per genome. **Soft cap:** warn-and-continue, logged.
- Window boundary: UTC (default; open question if the owner wants
  operator-local).

## 6. Known integration gaps (rulings)

- **F1 — fold `org grant give` AND the `Revoke` arm (cmd_org.rs:1054-1063) onto the approver**; make
  `GrantDb::insert` approver-private. One authority-write path. (Give and revoke are the same breach direction: direct table writes that skip containment.)
- **F2 — one doctor engine on `AppConfig`**; port the runtime-doctor checks
  worth keeping; gateway API consumes the same engine.
- Sweep knob removal: DONE (iter-606, strict-schema break, documented).

## 7. Wave order (each wave = its own iteration(s))

1. **Cast seed** (`org/cast.rs` + seeder + edges + charters + default cron
   registrations). Data only; first-run effect observable via
   `/permissions`-style views and cron list. **Seeder discipline: every
   insert goes through the approving stores ONLY — `DepartmentDb::upsert`,
   `HierarchyEdges::upsert_edge`, `SeatApprover::mint_for` (incl. the
   premiere's standing grant). No direct `authority_grants` writes, even
   at seed time** — the containment is structural, not convention.
2. **Sessions→employees**: `gateway_sessions.employee_id`, default
   `premiere` binding, `/session new [employee]` select, seat derivation
   flip. This wave makes 3/4/5 keyable.
3. **Rolling compaction handoff** (table + injector + TTL config).
4. **Budgets** (rollup + injection + hard/soft cap at the iteration seam).
5. **Onboarding transaction + doctor genome checks** (§4 + F2).
6. **F1** (grant-give fold) + remaining hygiene
   (config.rs doc-comment staleness about knob count — doc-only).

## 8. Open items (not blocking; defaulted)

- Budget window boundary: UTC unless overruled.
- `/session new [employee]` vs `/use <employee>` command naming: resolved
  at the wave-2 implementation per the repo's command-shape conventions.
- Agent-fabric fold (crew-chief subsumes it) is final unless a measured
  reason to re-split appears.
