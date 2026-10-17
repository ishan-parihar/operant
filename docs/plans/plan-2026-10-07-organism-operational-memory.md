# Organism operational-memory plan — 2026-10-07

Ground plan for the questions the owner raised after the seed-cron
infrastructure landed (iter-657/658, deployed and live-verified: eight
seats self-run at manifest cadence; dispatcher healed to top-of-hour and
resumed; identity-warden ran its first audit at 05:00 UTC).

Everything in §1–§4 is **observed fact** from the live dbs, the journal,
and the code as of iter-659. §5–§6 are the plan.

---

## §1 How employees operationalize today (the audit answer)

**A seat runs its charter — nothing else.** When a seeded job fires, the
agent receives the seat's charter as the prompt, runs once, and the
write barrier attributes one worklog row to the seat's session. That is
the entire operational surface:

| Aspect | State | Evidence |
|---|---|---|
| Self-run at manifest cadence | ✅ live | 8 `cron_cast_*` jobs active; 2 seats already executed |
| Worklog attribution | ✅ live | `worklog` rows written per completed run via the barrier |
| Notice-board reading | ❌ absent | `notices: 0`; no prompt or tool consumes the board |
| Cross-department worklog reads | ❌ absent | no read surface; nothing recalls another seat's work |
| Feed / DM checking | ❌ absent | adapters are outbound push only; no read API exists |
| Identity binding on the run | ❌ partial | prompt = charter (content-level); `IdentityGate` has zero production callers — no policy/gate/grant consult on the run path |
| Budget enforcement | ❌ unreachable | `seat_budgets` has 0 rows; no provisioning CLI |

So: **they work only their roles today.** The social-surface behaviors —
checking feeds, DMs, department worklogs, other departments' outputs —
are not implemented anywhere yet. That is §3's work, not a configuration
change.

**Live constraint observed:** both first runs (identity-warden 23:04,
dispatcher 23:26 UTC) failed provider-side — a stream death and a
`chat_admission_busy` 503. The organism is alive; the model provider's
capacity is its current growth limit. At manifest cadence (1×/hr worst
case per seat, 8 seats, ~40 runs/day) this is a scheduling and
retry problem, not an emergency.

## §2 Swarm intelligence — verdict: substrate exists, engine missing

The cross-seat machinery is built but **nothing circulates through it**:

- **Write side**: every completed run leaves a worklog row (live: 2).
- **Board**: the notice board is a functioning cross-department bus —
  with zero rows, because no seat's charter-prompt tells it to post,
  and no run reads it.
- **Decisions**: the decisions db and its accept flow exist — but
  `can_accept_decision` is never called by the write path (iter-642
  audit finding), so ratification is structurally open.
- **Loop**: emergent synthesis needs seats→write→synthesizer→circulate
  →next-runs. Only the first arrow exists.

Emergent synthesis is therefore **not yet working** — it is latent
infrastructure waiting for §3–§5.

## §3 Operational-model expansion (Slice 8): what a seat reads each cycle

Make the seat's prompt operate on real org state, in this order:

1. **Department-local memory** — the seat reads its own department's
   recent worklog rows before acting (feeds the "what happened while I
   was away" context).
2. **The notice board** — cross-department bus; seats read notices
   addressed to them or their department; governor/chief-of-staff post
   digests there.
3. **DMs/feeds**: **deferred by design.** The platform adapters are
   outbound push today; adding inbound read surfaces (Telegram/Slack
   fetch) is a separate adapter-capability wave. Recommend org-internal
   surfaces first (worklog/board), platform DM polling as its own slice
   with its own rate-limit budget. Owner call recorded as open.

Implementation shape: a `digest` read surface (one method over the
existing stores — no new schema), wired into the seeded prompts'
cadence lines, not a new tool class.

## §4 Memory architecture — two-tier, recommended

**The finding that makes this cheap:** `MemoryWireProvider` already
carries a `bank` field (`memory_wire.rs:115–123`, `from_service(service,
bank)`) — hardcoded to one `DEFAULT_BANK` today. memory-wire recall is
bank-isolated natively. Two-tier routing needs **no new dependency** —
only bank selection by active employee context, which
`gateway_sessions.employee_id` already carries.

| Tier | Bank id | Transferable | Contents | Writes | Reads |
|---|---|---|---|---|---|
| Org | `org` | yes | directives, ratified decisions, policies, synthesis digests | governance acts (decision accept, grant mint, directive) | every seat's prefetch |
| Seat | `emp-<seat_id>` | no | role working memory, charter-internal learnings | that seat's `sync_turn` | that seat's prefetch; manager audit (read-only) |

- **Why not employee-only banks:** directives and ratified decisions
  would strand per-seat; new seats start blank; the org never
  accumulates shared knowledge. The "collective identity" the owner
  wants cannot form — there is no commons.
- **Why not global-only (status quo):** premiere's directive memory and
  dispatcher's queue state bleed into one stream; seats cannot develop
  distinct operational identities; cross-seat contamination is the
  failure the owner is worried about, and today it is the default.
- **Transferable/non-transferable rule:** anything that changes org
  state or policy → org bank. Role-internal reasoning → seat bank.
  Classification is by *writer*, not by tags — the governance acts are
  the only org-bank writers, so the boundary is structural.

## §5 Identitarian evolution — how it happens on the go

Two loops, both mediated by existing machinery:

1. **Memory-level (automatic, from Slice 7 on):** each seat's `emp-*`
   bank diverges naturally — different charters, different work, no
   cross-contamination. Identity drift is the *default*; the org bank
   converges the shared layer via synthesis (Slice 9). The two-tier
   design makes evolution a property of the system, not a feature to
   build.
2. **Charter-level (ratified, Slice 10):** identity-warden's audit
   cycle and governor's governance digest propose charter/skill
   amendments as **decisions**; premiere accepts (requires the Slice-4
   `can_accept_decision` wiring — currently blocked on a peer's
   `cmd_org.rs` WIP); the registry updates through the existing
   amendment path. Evidence → proposal → ratification → mutation, once
   per cycle. Charter changes are deliberate acts with an audit trail —
   never silent drift.

## §6 Slice sequencing (for approval)

| # | Slice | Depends on | Risk |
|---|---|---|---|
| 7 | Two-tier memory routing (`org` + `emp-*` banks, merged prefetch, governance-write rule) | none — bank field already exists | low: behavior-additive, ungoverned seats byte-identical until routed |
| 8 | Operational visibility (digest surface + department worklog/board reads in seat cycles) | 7 (digests recall) | low: read-only surfaces |
| 9 | Synthesis mechanism (chief-of-staff daily synthesis job → org bank + notice post) | 4, 8 | medium: first writer that generalizes across seats |
| 10 | Identitarian evolution wiring (governor/identity-warden proposals → decisions → premiere accept → registry amendment) | 4, 9 | medium: requires the accept path live |
| 4 | Authority predicates (`can_post_to` @ notice post, `can_accept_decision` @ accept) | **peer lands `cmd_org.rs` WIP** | low: consult-only, fail-closed |
| 5 | Read-only surfaces (`org cast`, `org audit <seat>`, spend report) | **peer lands `cmd_org.rs` WIP** | low: read-only |
| 3b | Unattended-seat governance (`with_org_gate` + `SEAT_BUDGET_ENVELOPE` on cron path + `org budget` CLI) | owner's explicit yes | **high: can start blocking live jobs; gated behind policy row if approved** |

Recommended order: **7 → 8 → 4 (when unblocked) → 9 → 10**; 3b whenever
the owner calls it. Slice 6 housekeeping (test-cron cleanup — destructive
— and BUGS.md claim fix) slots anywhere.

### Open owner decisions (carried from the 2026-10-07 outline)
1. Slice 3b governance: yes/no (starts enforcing policies on cron runs).
2. Slice 6 destructive cleanup of the 6 test cron jobs + `wave5-test`: yes/no.
3. Platform DM/feed monitoring (Slice 8 scope): include now or defer.
4. The 2 cron-derived orphaned seats in `database.db`: `operant org sync`
   or accept 9.
