# Per-Employee Permission Scoping — plan

> Program name from the request: the unified permission mechanism becomes the
> organism's **genome** — one inherited substrate, specialised per seat, that
> makes every employee agent legible to its seniors and governable by the
> operator.
>
> Status: **proposal, 2026-10-03. Not started.** Blocks on the alignment
> questions in §6. Reads after `docs/NEXT-IMPLEMENTATION-OUTLINE.md`; it extends
> Steps B′/D′, it does not replace them.

---

## 1. What we are building

A per-employee permission regime with a spectrum:

| Mode | Meaning |
|---|---|
| `yolo` | every tool open, no prompts |
| `standard` | today's behaviour — dangerous tools escalate |
| `scoped` | only listed tools/commands open; the rest escalate |
| `lockdown` | nothing runs without a standing grant |

…plus **negations** (deny lists that win over everything else), **escalation**
(an employee that lacks a permission requests it from its seniors rather than
failing or — today — being silently allowed), and an **audit surface** where the
operator and a seat's seniors can see and revoke what it held and used.

The goal behavioural property: *the organism operationalises like an
organisation*. An employee acts within a mandate, exceeds it by asking, and
someone above owns the answer.

## 2. What already exists (verified, do not rebuild)

| Piece | Location | State |
|---|---|---|
| Grant ledger with expiry/revocation/`reason` | `org/authority.rs:452` (`Grant`), `GrantDb` `:551` | built, **no runtime caller** |
| Six-rung authority ladder (`Own..Org`, `contains`) | `org/authority.rs:85` | built |
| Grant issuance, refuses inert/invalid seats and scope amplification | `tools.rs:1007` (`issue_grant`) | built |
| Grant consumption query | `tools.rs:861` (`tool_authority_check`) | built, **no runtime caller** |
| Per-tool authority bindings on the registry | `tools.rs:362`, accessors `:1059`/`:1069` | built, never set |
| Reporting-line queries | `hierarchy.rs:379` (`manager_of`) | built, **edges unpopulated** |
| Interactive escalation transport | `agent/mod.rs:546` (`ToolPermissionRequest`) | live for the gateway agent |
| AllowOnce/AllowSession/AllowAlways responses | `agent/stream.rs:788-811` | live |
| Cron org-identity gate | `org/identity_gate.rs:90` (`GateDecision`) | live on the cron run path |

The substrate is the genome. Everything below wires policy into it.

## 3. What is missing

1. **A seat policy type.** Employee → mode + allow list + deny list. Nothing in
   the repo models this. This is the piece the word "genome" names: one defined
   shape every employee agent inherits and every seat specialises.
2. **Enforcement on the run path.** `tool_authority_check` is never called
   during a run. The permission guard (`stream.rs:737`) and the smart-approval
   gate (`stream.rs:709`) know nothing about employee authority.
3. **Escalation routing.** On deny, there is no path from "employee lacks
   authority" to "senior is asked". The gateway's interactive channel is the
   obvious transport but its no-active-channel branch auto-allows
   (`gateway_runner.rs:2002`) — correct for one interactive user, wrong as
   an org primitive.
4. **Approval → durable authority.** Today an approval is transient
   (`AllowSession`/`AllowAlways`). For the org, a senior's approval of an
   employee's request is a *grant*: `issue_grant` with a TTL, so the same tool
   doesn't re-ask on every run (§2.5's grants-expire-by-default already exists
   as constructor discipline).
5. **Audit/management surface.** Grants are already append-only and attributable
   (grantor/grantee/reason/timestamps). Missing: the query view (`/permissions`),
   mutation commands (`/grant`, `/revoke`), and a record of every *request and
   response*, not just the grants that survived.
6. **Hierarchy edges populated** — escalation needs `manager_of` to return real
   data. This is the pending D′ owner decision (reports_to column vs. edges
   table), now load-bearing rather than cosmetic.

## 4. Precedence (proposed, needs sign-off — Q4)

A tool call is decided by the first matching rule:

1. **Hardline blocklist** (`approval.rs` dangerous-pattern `Blocked`) — always
   wins, over YOLO and over grants. Infrastructure, not policy.
2. **Seat deny list** — always wins over anything below.
3. **Seat allow list** or `mode = yolo` — runs.
4. **Standing grant** (unexpired, unrevoked, scope+dept match) — runs.
5. **Standard mode dangerous tool** — escalate.
6. **Everything else** — escalate (deny on failed escalation, never allow).

Rationale: amplification must be impossible both *through the ladder*
(`issue_grant` already refuses it) and *through negation* (a deny row a CEO
grant cannot override).

## 5. Phases

Each is its own iteration(s), each verified in a clean worktree with a negative
control. Numbering follows the convention in `docs/NEXT-IMPLEMENTATION-OUTLINE.md`
§10 — no absolute labels reserved here; take the next free label at commit time.

### P0 — Seat policy model

**Status: engine landed** — `org/seat_policy.rs` ships `SeatMode`,
`SeatPolicy`, and `decide()` with the full §4 precedence, namespace-glob
patterns via the LCM matcher (`lcm.rs:49`), and 10 adversarial tests
(fault-injection red→green proven, restore verified by sha256).
Storage wiring is the remaining piece; the engine is storage-agnostic by
construction, so Q1's answer changes one adapter, not the core.

New `seat_policies` persistence (§6 Q1 decides table vs. config), mode +
allow/deny lists, and the precedence engine from §4 as a pure function —
`decide(policy, actor, tool, grants, now) -> Run | Escalate | Deny` — with unit
tests for every rule ordering, including the two adversarial pairs in §4
(blocklist-beats-yolo, deny-beats-grant).

*Acceptance*: a pure-function test suite; no touching of the run path yet.

### P1 — Enforcement on the run path
Build an `AuthorityActor` for the running employee (cron: `derive_employee_id`;
gateway: the seat the chat is speaking for) and consult `decide` at
`stream.rs` before the existing guards. Unwired bindings stay global (Rule 1),
so this is byte-identical for anyone without a policy row — the migration is
opt-in per seat.

*Acceptance*: one process, two employee agents with different policies; the
same tool call succeeds under `yolo` and escalates under `lockdown`; a
no-policy agent's behaviour is unchanged from current mainline (regression
guard).

### P2 — Escalation routing
Deny-or-escalate paths emit a `ToolPermissionRequest` tagged with the
requester's employee id, routed by `manager_of` up the ladder (employee → HoD →
CEO → operator). An approval calls `issue_grant` with an explicit TTL (§6 Q3)
and is *persisted*; the re-run consults the grant ledger and doesn't re-ask.
Unattended escalations (cron) obey a deny-after-TTL default (§6 Q2) instead of
the 120s packet stall — the current timeout is shaped for a watched prompt, not
a scheduler.

*Acceptance*: end-to-end — a lockdown cron job is denied, its request surfaces
to the right senior, an approval mints a grant row (grantor/grantee/TTL/
reason), and the next run of the same job executes without re-escalating; a
denial stays denied and is auditable.

### P3 — Audit & management
Gateway commands: `/permissions [seat]` (effective policy + standing grants +
recent requests), `/grant`, `/revoke`. Seniors over the same commands, scoped to
their subtree (a HoD manages direct reports only — `DirectReports`, the rung
already exists).

*Acceptance*: operator revokes a grant via the UI; the next run of a job that
relied on it escalates again. Audit list shows grantor, grantee, capability,
TTL, reason, and the request that produced it.

### P4 — D-2 resolves itself
Cron's permission posture stops being a standalone question: a cron job runs as
its employee, whose seat policy IS the posture. Unattended jobs under
`lockdown`/`scoped` escalate per P2 with cron-appropriate TTLs; `yolo` seats
reproduce today's open behaviour deliberately, not by missing code.

---

## 6. Alignment questions — what I need from you

I have a recommendation for each; confirm or redirect and I execute.

1. **Policy source of truth** — pick one:
   (a) a `seat_policies` table, runtime-editable, auditable, survives restarts —
      *recommended*; grants already live in a DB, one surface;
   (b) config file, reviewed in git but frozen at runtime;
   (c) config as defaults, DB as overrides.
2. **Escalation TTL when unattended** — recommend deny with a configurable TTL
   (46 min HoD, configurable; operator = no timeout but requests queue for the
   next interactive session). Confirm the deny-default and the TTL shape.
3. **Senior approval → durable grant?** — recommend yes, with `expires_at`
   required (§2.5's standing discipline). Suggested default TTL: 7 days for
   HoD grants, none (standing) only at CEO+. Confirm TTL policy.
4. **Precedence §4** — confirm: blocklist and seat-deny always win; grants
   extend but never un-deny; YOLO does not bypass the hardline blocklist.
5. **Terminal authority** — is the human operator the final escalation for
   everything, and does HR sit on every chain or only when a seat names it?
6. **Hierarchy edges (the old D′ decision), now blocking** — escalation routing
   needs `manager_of` to be real: (a) `reports_to` column on employees —
      the design's stated intent, touches eight construction sites;
   (b) separate edges table — cheaper, matches how `HierarchyEntry` is stored.
      Recommend (b): ships faster, less blast radius.
7. **Scoped-mode semantics** — for a `scoped` seat facing a *dangerous but
   unlisted* tool: escalate (recommend — missing ≠ denied) or hard-deny?
8. **Granting topology** — HoD within own department only, CEO anywhere, HR
   read-only/audit? Or another split.

## 7. Known risks this inherits (from BUGS.md, not re-discovered)

- **120s stall per escaltion** (`stream.rs:788-791`): the interactive timeout
  cannot be the cron semantic (P2's deny-after-TTL addresses exactly this).
- **D-5 memory-write race** between the two agents over one MEMORY.md is live
  and independent of this plan; P4 does not wait on it but P1's "two agents"
  acceptance hits fewer orderings if it lands first.
- **D-6 process-global sub-agent limits**: employee agents share one delegation
  budget; per-employee governance does not isolate it. Escalation can request
  authority, not budget — out of scope for v1.
- **No active channel means auto-AllowSession** (`gateway_runner.rs:2002`)
  — branch deliberately bypassed by P2's router, but left untouched for the
  interactive path it was written for.
- **`derive_employee_id` collision ceiling** — 28 effective bits (7 of 8 hex
  chars): fine at ~100 jobs, ~18% at 10k. Pin injectivity in a test, fix only
  if the org grows.

## 8. Escalation delivery model — when does a senior see a request?

The two timelines are deliberately separate: **delivery** (push) and
**resolution** (the grant). Nothing about the tool call executes at request
time.

1. **At request time** — the request is persisted to a durable
   `pending_requests` queue *and* pushed to the approver's channel if they
   are connected at that moment. The employee's run either waits, bounded
   (interactive), or records the denial and proceeds (cron). A 3am cron
   escalation is never lost to a sleeping HoD: the queue survives.
2. **When the senior looks** — three surfaces, in latency order: the live
   push above; a `/pending` pull at any time; a session-start digest
   ("3 requests pending in your subtree") at next connect. So the answer to
   "when would the HoD check" is: *at request time if connected, otherwise
   at their next interaction — the request is durable either way.*
3. **At resolution** — approval mints a TTL'd grant via `issue_grant`; the
   waiting run (or the next cron run) consults the ledger and runs without
   re-asking. Denial denies. **TTL expiry denies that attempt only** — the
   request row stays in the queue for audit, and re-escalation references it
   ("still pending since…") rather than duplicating it.

So: requests are *delivered* the moment they are made, but the *tool* runs
only when a grant stands — never at request time.

## 9. Scalability & topology invariants (audited 2026-10-03, code-verified)

The power topology was audited against the substrate before extending it.
Every row cites code that was read this session:

| Invariant | Evidence | Why it scales |
|---|---|---|
| Escalation chains terminate | `hierarchy.rs:255` `MalformedKind::{Cycle, SelfManaged, DanglingManager}` — malformed graphs reported, never walked infinitely | any org shape, including bad imports, is safe to route over |
| Authority cannot amplify | `issue_grant` (`tools.rs:1007`) refuses grants exceeding the grantor's scope | delegation scales with no central validator; the check is local |
| Grant lookups indexed | `idx_authority_grants_grantee` (`authority.rs:575`) | O(log n) per consult at any org size |
| Manager hops O(1) | `manager_of` is a HashMap get (`hierarchy.rs:379`) | routing cost is linear in chain length, not org size |
| **Any number of systems** | policy patterns are LCM globs; `Grant.capability` is a free-form dotted namespace | a future subsystem is governed by one row (`jcode.*` bars or opens it wholesale) — proven by the `namespace_glob_governs_future_subsystems` test governing a tool that does not exist yet |
| New tools default-open until bound | `tool_authority_check` Rule 1 (`tools.rs:861`) | adding a subsystem imposes zero registration burden to stay ungoverned |
| Decision core is pure | `decide()` (`org/seat_policy.rs`) — no I/O, no clock | every future surface (gateway, cron, new subsystems, multi-tenant) composes one function; no forks |
| Unpolicied seats byte-identical | `no_policy_is_byte_identical_to_mainline` test | governance is opt-in per seat; adopting never redeploys behaviour on seats that did not ask |
| Denial absolute | `deny_list_beats_grant_allow_and_yolo` test | no grant — a CEO's included — can override a negation; no privilege path around policy |

Deliberately out of scope: the process-global sub-agent budget (D-6).
Escalation grants *authority*, never *capacity* — a budget genome is a
separate substrate.
