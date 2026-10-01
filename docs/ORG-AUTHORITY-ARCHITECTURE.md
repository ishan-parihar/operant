# Organism authority, memory, and the CEO loop — architecture design

> **Status:** design, not implementation. 2026-10-01.
> **Answers:** the six open questions in [`REMAINING-GAPS.md`](REMAINING-GAPS.md),
> and the owner's design session of 2026-10-01.
> **Supersedes nothing.** [`CHRONOGRAPH-DESIGN.md`](CHRONOGRAPH-DESIGN.md)
> remains the collective-synthesis rationale; this document is the authority,
> session, and memory architecture it depends on.

---

## 0. The shape of the thing

Ten layers. Each one is small on its own, and each is a prerequisite for the
next. The dependency that matters most:

```
  designation + hierarchy  ──▶  authority scope  ──▶  prompt injection
        │                                                   │
        │                                                   ▼
        └──────────────▶  department membership ──▶  the agent knows who it is
                                                        │
                          ┌─────────────────────────────┘
                          ▼
                  persistent session + autocompact
                          │
                          ▼
              write barrier (worklog + subjective + decisions)
                          │
                          ▼
              board: assignment / feed / DM (3 turns)
                          │
                          ▼
                    CEO loop (cron, fan-out, synth, re-convene)
```

**The single most consequential decision in this document** is that employees
stop being stateless cron invocations and become **resident actors with a
persistent session**. Everything else — the DM turn limit, the CEO loop, the
autocompact, the personality accumulation — depends on a session surviving
between runs. Section 4 covers this.

---

## 1. Designation — how other agents know who they are talking to

Every employee carries a **designation**: a short human-readable statement of
identity, position, and standing. It is visible to every other agent in every
DM header, board row, and artifact byline.

```
"Rowan Vale — Head of Department, Platform Infrastructure.
 Reports to: Chief Executive. Direct reports: 4.
 Authority: may assign work within Platform Infrastructure;
            may not alter staffing outside it."
```

**Storage.** `role` and `department` already exist on `employees`.
`reports_to` and `peers` are new columns (Wave 3). The designation *string* is
**derived, not stored** — it is composed at read time from
`(name, role, department, reports_to, direct_reports, authority_scope)`. Storing
it would let it drift from the fields it summarises.

Deriving it means every place that renders a person gets a consistent
designation for free: notice senders, DM headers, worklog `employee`, artifact
authors, and the CEO's meeting roster.

**Why derived matters for agents.** An agent reading a board must be able to
trust that a designation is current. A derived string cannot say "Head of
Platform" for someone who was moved yesterday.

**Unstaffed seats stay visible.** If a department declares it requires a
Platform Infrastructure head and the seat is empty, the designation for that
seat renders as `"VACANT — Platform Infrastructure (unstaffed)"` rather than
being omitted. This follows the standing rule: never fabricate a capability for
a seat nobody fills.

---

## 2. Hierarchy and scoped authority

### 2.1 The answer to Q1

The owner's direction was that hierarchy *scopes the range of authority*.
So: **hierarchy is the primary grant**, and it is sufficient for everything
inside a department. An **explicit capability grant** is required only to
cross a department boundary — because that is the only place where hierarchy
gives no answer.

This is the resolution of Q1 in [`REMAINING-GAPS.md`](REMAINING-GAPS.md), which
flagged it as an authorization boundary I should not guess at.

### 2.2 The scope lattice

`AuthorityScope` is an ordered lattice, not a flag set. An action is permitted
if the actor's scope **contains** the target's scope.

| Scope | Means | Grants |
|---|---|---|
| `Self` | own record only | read/write own profile, own worklog, own subjective log |
| `Peers` | same department | read peers' logs; **no** writes |
| `Department` | own department | post to `dept:<me>`; read assignment board; acknowledge |
| `DirectReports` | own reports | assign work; edit their persona drafts; approve their DMs |
| `Descendants` | whole subtree | department-level structural changes inside subtree |
| `Org` | whole organisation | cross-department grants, board creation, CEO loop |

Containment is **tree containment**: `Descendants` contains `DirectReports`
contains `Department`. `Org` contains everything. `Self` contains nothing but
itself.

An ordinary employee is `Peers` + `Self`. A department head adds `Department` +
`DirectReports` + `Descendants`. The CEO is `Org`.

### 2.3 What authority actually gates

Authority is checked in exactly four places. If it is not enforced in one of
these four, it is decoration.

1. **Board writes** — posting a notice addressed outside your scope needs a
   grant; posting to `dept:<own>` does not.
2. **Assignment board mutations** — reassigning a role to an employee outside
   your subtree needs a grant (§5).
3. **Decision acceptance** — only an actor whose scope contains the decision's
   target scope may accept it (§8).
4. **Tool availability** — see §2.4. This is the one most likely to be
   forgotten, and it is the one that matters most for containment.

### 2.4 Authority scopes tool access, not just board access

An HOD exploring its own system during a CEO meeting is running tool calls. If
those tool calls are not scoped, the authority model is decorative: the agent
can bypass every board check by reading files directly.

So the tool registry is filtered by authority at agent construction. A
`Department`-scoped agent is offered the department's tool surface. Reaching a
sibling department's tooling requires the capability grant, which is
individually recorded and individually revocable.

This means `OperantTool` registration becomes authority-filtered at build time.
It is the most invasive change in this document and is sequenced in Wave 3,
after the scopes exist to filter by.

### 2.5 Cross-department grants

New table `authority_grants`:

| Column | Notes |
|---|---|
| `grant_id` | `'ag_' || uuid` |
| `grantor` | employee id; must hold `Org` scope to grant `Org`-touching scope |
| `grantee` | employee id |
| `capability` | e.g. `platform_infra.tooling`, `content.read` |
| `scope` | the scope being extended, e.g. `Department` |
| `target_dept` | the specific department the grant is good for; NULL = any |
| `reason` | required, non-blank — same rule as every other mutation in Wave 1 |
| `granted_at`, `expires_at`, `revoked_at` | expiry and revocation are first-class |
| `revocation_reason` | required if `revoked_at` is set |

Every grant is a mutation, so it carries a reason and lands in the objective
log. Grants expire by default: a cross-department capability that nobody
renewed should not silently persist for a year.

---

## 3. Department membership, mandates, and feed segregation

### 3.1 Membership is a tag on the agent

The owner's instruction was explicit: membership is **a tag within agent
configuration**, which decides boundaries and job roles. That maps onto the
existing `employees.department` column — the tag is already there and is
already settable via `--department` on the CLI.

The Wave 1 audit found `employees.department` is `NULL` for all 102 backfilled
rows (`employee_db.rs:250` hardcodes `department: None`), which was Q4. The
answer is now settled: **it becomes an operator-set field, and NULL is
surfaced, not silently tolerated.** Concretely:

- `operant org employee set-department <id> <dept> --reason "..."` becomes the
  way in. Backfill does **not** guess from cron job names.
- An employee with `department IS NULL` renders in every feed view with an
  explicit `(undept)` marker.
- `org check` counts them and reports them (see §11.2).

An unstaffed organisation is legible; a silently mislabelled one is not.

### 3.2 Feed segregation follows from the tag

Three tiers, resolved per reader:

| Tier | Selector | Visible to |
|---|---|---|
| **Direct** | `agent:<me>` | the named employee |
| **Department** | `dept:<my dept>` | members of the reader's department |
| **Global** | `broadcast`, `role:<role>` | everyone |

This is the segregation the owner asked for, and it needs no new feed model —
`Recipient::{Agent,Dept,Role,Broadcast}` already encode all three. What was
missing was the resolver (Wave 2) and the department data (this section).

**Escalation is explicit.** A department feed entry is never automatically
promoted to global. If a department decides something is org-wide, the head
*posts a new notice* to `broadcast` citing the department notice. That makes
every escalation an attributable act rather than an emergent one.

### 3.3 The department contract

New table `departments`. This is the "staffing mandate, working protocols, and
rules" the owner specified.

| Column | Purpose |
|---|---|
| `dept_key` | canonical slug, PK. Renames go through `DEPT_ALIASES`, never in place |
| `display_name` | human label |
| `mandate` | prose: what this department exists to do, and how success is judged |
| `protocols` | JSON array of working protocols — the department's SOPs |
| `rules` | JSON array of hard constraints. Violating one is a gate failure, not a warning |
| `required_capabilities` | JSON array of capabilities the seat set must cover |
| `head_employee_id` | NULL when the head seat is vacant |
| `target_headcount` | planned size |
| `created_at`, `updated_at`, `reason` | standard Wave 1 mutation fields |

**The `rules` array is enforced, not advisory.** An entry is a fail-closed
precondition, checked by the same `IdentityGate` that already exists for
identity (§11.1). "This department must not write outside its own tree" is a
rule; a rule that only prints is not a rule.

**Staffing is computed, not stored.** Actual fill = live employees in the
department. Unfilled required capabilities produce a visible finding, per the
standing no-fabrication rule.

### 3.4 Mandate and protocols ride the prompt

`mandate` + `protocols` + `rules` are injected into every member's system
prompt (§6). This is what makes a department a department rather than a filing
label: an agent reads its constraints at the top of every run.

---

## 4. Persistent sessions — the load-bearing change

### 4.1 The change

Today a cron job spawns an agent, it runs, it exits. Everything it learned is
gone except what was written to a database.

**Under this design each employee has at most one live `AgentSession`**, keyed
by employee id, persisted across runs. Every invocation — cron, DM, CEO
meeting — loads that session, injects a fresh volatile suffix, runs, and saves
back.

```
  cron fires  ─┐
  user DMs    ─┼─▶  load session(employee)  ──▶  inject volatile suffix
  CEO meeting ─┘         │                              │
                         │                        run(tool loop)
                         │                              │
                         │                    ┌─────────┴─────────┐
                         │                    │  write barrier    │
                         │                    │  (§7) mandatory   │
                         │                    └─────────┬─────────┘
                         │                              │
                         └──── autocompact ◀────────────┘
                                    │
                              save session back
```

Consequences, all of them intended:

- **A user can DM any employee at any time** and get a response grounded in
  everything that employee has done. This is the owner's requirement and it is
  impossible without the session.
- Personality accumulates in *context*, not only in logs.
- The CEO meeting can be a real conversation, not five cold prompts.

### 4.2 A new conflict the owner has not raised yet

A persistent session per employee means **each live employee holds a Tokio task
and a context window**. With 102 employees all resident, that is 102 resident
sessions.

I do not think this should be 102 OS processes, and I do not think it should be
102 always-on tasks either. The design I propose:

- **Sessions are persisted and load-on-demand, not resident.** The session row
  exists; the agent is constructed only when something addresses the employee.
- A `SessionRegistry` holds a bounded LRU of *warm* sessions (default 4), so
  concurrent CEO meetings over 6 HODs evict cleanly rather than all at once.
- The DM path and the cron path share the registry, so a user DM during a cron
  run **joins** the warm session instead of forking a second copy of it. The
  per-employee lock is the serialisation point.

This keeps memory flat while preserving the conversation semantics. If the
owner would rather have true always-resident employees, that is a different
deployment shape and I would want to build it deliberately rather than
accidentally.

### 4.3 Autocompact after every cron run

The owner's requirement: each persistent session **must autocompact after its
cron job runs**.

I am implementing this as **mandatory-with-a-floor**, and I want to be explicit
about the deviation rather than bury it:

- After every scheduled run, the session is compacted. Always.
- **But** the summarisation step is skipped when the session is below
  `compact_floor_tokens` (default 4 000, well under any model's window).

Rationale: unconditional compaction of a short session replaces verbatim history
with a lossy summary for no benefit, and then compounds — the next compaction
summarises a summary. The *invocation* is mandatory and observable, so "the
session was compacted after the cron run" remains a true statement about the
pipeline; only the lossy step is skipped when there is nothing to lose.

If the owner wants literally unconditional summarisation, that is a one-line
change to the floor check and I will make it.

Compaction preserves the objective anchors (`llm_compressor.rs` already keeps
some), and the persona/personality block is re-injected from the persona
record rather than being carried in the summary — so personality survives
compaction by construction rather than by luck.

---

## 5. Assignment boards and feed boards

Two distinct board types, both over the same substrate.

### 5.1 Assignment board (per department)

New table `assignments`:

| Column | Notes |
|---|---|
| `assignment_id` | `'as_' || uuid` |
| `dept_key` | owning department |
| `role_label` | the seat or duty being staffed |
| `assignee` | employee id, or NULL for an **open seat** |
| `priority`, `due_at` | scheduling |
| `source_notice_id` | the notice that created the work |
| `status` | `open` \| `in_progress` \| `blocked` \| `done` |
| `revision` | optimistic concurrency, so two heads cannot silently clobber |
| `reason`, `created_at`, `updated_at` | standard mutation fields |

**Mutation authority (§2.3.2):** a head may create and reassign within their
own department freely. Moving a `role_label` to an assignee in *another*
department, or creating an assignment that pulls in out-of-department labour,
requires an `authority_grants` row. That is the explicit cross-department
answer to Q1, applied at the one place where it is needed.

**Open seats are rows, not absences.** An unstaffed role renders as an open
assignment with `assignee = NULL`. Fabricating a holder for it is forbidden.

### 5.2 Feed board

Not a new table. The existing `notices` table *is* the feed board, read through
the three tiers of §3.2 once the resolver (Wave 2) lands. The work is the
resolver and the department data, both already sequenced.

The one genuinely new read shape is a **thread view**: notices sharing a
`correlation_id`, ordered, rendered as a conversation. `CHRONOGRAPH-DESIGN.md`
already specifies this and it needs no schema work.

---

## 6. Prompt injection — hierarchy position changes the system prompt

The owner wants authority *conjoined with* prompt injection: an agent's position
in the hierarchy should organically change its system prompt.

### 6.1 The split

Operant already has the right seam — a **frozen prefix** for cache-stable
content and a **volatile suffix** for per-run content (`events.rs:374`,
`build_frozen_prefix`; iter-39's cache-stability work). Hierarchy content maps
cleanly onto it:

| Content | Placement | Why |
|---|---|---|
| Identity + designation | frozen | changes only on transfer |
| Authority scope + explicit limits | frozen | changes only on a grant change |
| Department mandate + protocols + rules | frozen | changes only on a dept edit |
| Reporting lines + direct reports | frozen | changes only on a transfer |
| **Persona block** (§9) | frozen | changes on personality evolution |
| Recent objective worklog summary | volatile | every run |
| **Subjective log digest** (§8) | volatile | every run |
| Inbox (DMs + board items addressed to me) | volatile | every run |
| **DM turn budget remaining** (§10) | volatile | every turn within a DM |

**Cache consequence, stated plainly:** the frozen prefix is per-employee, so
each employee gets its own cache entry and the byte-stability property is
preserved. But a **department transfer invalidates that employee's cache**,
because their mandate block changes. That is correct behaviour, not a bug, and
it is worth knowing it happens.

### 6.2 The persona block is the personality carrier

The frozen prefix gains one new section, composed from the accumulated persona
record (§9):

```
<organizational_identity>
  You are Rowan Vale, Head of Department of Platform Infrastructure.
  You report to the Chief Executive. You have 4 direct reports.
</organizational_identity>

<authority>
  You may: assign work within Platform Infrastructure; read and edit your
           direct reports' worklogs and persona drafts; post to
           dept:platform_infra and to broadcast.
  You may not: alter staffing outside Platform Infrastructure; accept
           decisions whose target scope exceeds your own.
  Cross-department actions require an explicit recorded grant.
</authority>

<department>
  Mandate: {mandate}
  Protocols: {protocols}
  Rules (hard constraints — violating any of these is a gate failure):
    {rules}
</department>

<persona>
  {evolved traits, accumulated across runs}
</persona>
```

The `<authority>` block is what makes the designation *actionable* for other
agents. An agent DMing this person reads their limits from the message header
before deciding how to ask.

---

## 7. The write barrier — logs are a precondition of completion

The owner's rule: **before any employee completes its job, it must append its
worklog, subjective memories, or decision logs.**

This is a **fail-closed write barrier**, and it is the natural counterpart to
the identity gate. Where the gate says "do not start unless identity is valid",
the barrier says "do not finish unless the record exists".

### 7.1 The rule

At the end of every run, in this order:

1. Append the **objective** row (worklog). One per run minimum.
2. Append the **subjective** rows (decisions, §8). Zero or more; at least one
   is required when the run made a choice worth recording.
3. Append **decision objects** for anything that changes organisational state.
4. Only then may the run report success.

A run that fails to write is a **failed** run, with the write failure as the
error — not a successful run with a missing log.

### 7.2 Why this resolves the identity-gate problem

Wave 1's gate is dark (GAP-2.3) because enforcing it would block 4 of 102 jobs
on upgrade. The write barrier has the same shape, but the failure mode is
different and much cheaper: a missing log is a **recoverable, visible defect on
one run**, not a global outage. So the barrier can be enforced immediately.

### 7.3 Retention of the barrier

The barrier is a **postcondition on `run()`**, not a per-caller convention.
This is what stops it decaying: it cannot be bypassed by a new call site,
because there is no path through `run()` that skips it.

---

## 8. The subjective log and the decision object

### 8.1 Two things, deliberately separate

The owner's framing was "a subjective log which you are terming as decision
object". These are related but not identical, and conflating them would lose
information:

- **Subjective log** — the agent's own account of its internal working:
  what it considered, what it rejected, how confident it is, what it would do
  differently. Free-form, per-employee, cheap, frequent. **This is the
  personality substrate** (§9).
- **Decision object** — a structured, attributable, *accepted* organisational
  decision: scope, decider, rationale, dissent, bindingness, expiry. Rare,
  authoritative, correlated across employees.

Keeping them separate means the subjective log can stay unstructured (the
memory engine's strength) while decisions stay queryable (the database's
strength).

### 8.2 Separate database

The owner asked for decisions to be **correlated into a separate database**.
Agreed, and it is also the only clean way to avoid the `PRAGMA user_version`
collision documented in `REMAINING-GAPS.md` §GAP-4.5 (the org tables share a
file already claimed by the kanban family at v1).

New file `operant_decisions.db`, with `org_decisions` and `subjective_log`:

**`subjective_log`**

| Column | Notes |
|---|---|
| `entry_id` | `'sl_' || uuid` |
| `employee_id` | author |
| `ts`, `ts_iso` | |
| `session_id` | the run that produced it |
| `run_kind` | `cron` \| `dm` \| `ceo_meeting` \| `manual` |
| `reasoning_digest` | what it weighed and rejected |
| `confidence` | self-reported |
| `would_do_differently` | free text — the personality growth vector |
| `traits_delta` | JSON: traits this run reinforced or shifted |
| `correlation_id` | joins to the board thread |
| `decision_ids` | decisions this entry produced |

**`org_decisions`**

| Column | Notes |
|---|---|
| `decision_id` | `'d_' || uuid` |
| `subject` | one line |
| `scope` | `Self` \| `Department` \| `Descendants` \| `Org` |
| `target_dept` | NULL for org-wide |
| `decided_by` | employee id |
| `rationale` | required, non-blank |
| `dissent` | JSON array of `{employee_id, position, reason}` |
| `binding` | bool — see Q3, below |
| `status` | `proposed` \| `accepted` \| `rejected` \| `expired` \| `superseded` |
| `expires_at`, `review_due` | decisions must expire or be reviewed |
| `correlation_id`, `artifact_ids` | joins to the meeting that produced it |
| `reason`, `created_at`, `updated_at` | standard mutation fields |

### 8.3 Q2 and Q3 are now answered

**Q2 — the CEO is an ordinary staff row.** Confirmed by the owner. So there is
no CEO runtime role, no special agent type, and no privilege the hierarchy
cannot express. The CEO is an `Employee` with `authority_scope = Org` and a
department of `executive`. The entire alignment meeting is machinery that
works for **any** two employees — which is the property that makes the DM
protocol (§10) reusable rather than a special case.

**Q3 — dissent is first-class.** Confirmed. `dissent` is a column, not a
comment. A decision that suppresses dissent is not a decision; it is a
suppression, and it is visible.

**Bindingness.** I am defaulting `binding = true` for decisions at `Descendants`
and `Org` scope, with dissent recorded and non-blocking, because an
organisation that cannot bind a decision has no alignment loop — only a
suggestion loop. Dissent remains visible and is the input to `review_due`. If
the owner wants alignment to be advisory only, that is one field default.

---

## 9. Personality accumulation — the worklog, the subjective log, and who the agent is becoming

The owner described the intent precisely: the logs *"progressively develop the
personality of each employee, as context related to these would be injected
into each next run, letting the agent know what kind of person it is."*

### 9.1 The loop

```
  run
   │
   ├─▶ objective log   (worklog: what was done, outcome, artifacts)
   ├─▶ subjective log  (what was weighed, rejected, confidence)
   ├─▶ decision objects (only when organisational state changed)
   │
   ▼
  persona synthesis  ──▶  <persona> block  ──▶  next run's frozen prefix
```

### 9.2 Persona synthesis

On session save, `PersonaSynthesizer` folds the run's subjective entries into
the employee row's existing `persona` JSON — the column that exists and is
always NULL today (`employee_db.rs:179, 262-263`).

Persona shape:

```json
{
  "traits": { "cautious": 0.7, "thorough": 0.9, "fast_to_escalate": 0.2 },
  "domain_confidence": { "platform_infra": 0.85, "content": 0.1 },
  "recurring_blockers": ["needs staging access before cutover"],
  "self_narrative": "Prefers to verify staging before touching production.",
  "evidence_runs": 47,
  "updated_at": "..."
}
```

Traits are **emergent from the logs, never hand-authored**. They are a
compressed summary of observed behaviour, which is what makes them honest. The
`employees.persona` column is also writable by a head under
`AuthorityScope::DirectReports` — a human or a head can set `self_narrative`
directly, but trait *scores* stay derived.

### 9.3 Sizing

Q5 (worklog granularity) is answered by the architecture: **one objective row
per run, guaranteed by the write barrier**. That is the natural granularity and
it removes the ambiguity in the original question — there is no "notable" to
define, because the barrier guarantees a row.

The volume concern is real but lands in the wrong place: the objective log is
high volume, so it is **never** injected verbatim. The volatile suffix carries
only a bounded digest (most recent N runs plus run outcomes since last
compaction). The full log stays queryable and becomes input to the CEO
meeting (§11), not to the prompt.

---

## 10. DM protocol — three turns, both parties aware

### 10.1 The rule

The owner's specification: two employees DM for **a fixed number of turns —
three**. Each message spawns the other agent. Each party may use tool calls
freely within its turn (transfer files, exchange information). **Both parties
must be aware of the limit.**

### 10.2 Turn accounting

- A **turn** = one message + the recipient's full tool loop to produce its reply.
- `turn_budget = 3`, initiator spends turn 1.
- Thread is **closed** at 0 remaining, for both parties.
- `dm_threads` carries `participants`, `turn_budget`, `turns_used`,
  `state`, `correlation_id`, `opened_at`, `closed_at`.

### 10.3 This resolves Q6

`REMAINING-GAPS.md` Q6 asked whether a ceiling should record-and-mark-terminal
or drop silently. **Neither.** The ceiling is a thread with a known length, and
the answer is a normal conversation that ends:

- The remaining-turn count is injected into **both** parties' volatile suffixes
  (§6.1), so awareness is structural, not a prompt request.
- At 0, the thread closes and further notices on that `correlation_id` are
  **rejected with a visible recorded reason** — never silently dropped.
- The full transcript is persisted, so a closed thread is still readable. An
  employee can be DM'd again in a *new* thread with a fresh budget.

So the loop guard the audit called GAP-2.1 is **this**: a bounded, symmetric,
visible turn budget rather than a global interaction cap. I would implement it
as the loop guard, and drop the "per-employee per-tick budget" idea from the
audit — it was the wrong mechanism for the same hazard.

### 10.4 What the guard actually prevents

A→B→A is bounded at 3 turns because the budget is shared and decremented, not
per-pair. That is why the budget belongs on the *thread* and not on the
*notice*: a per-notice cap is defeated by opening a new notice, which is
exactly the regression the audit called out.

---

## 11. The CEO loop — cron, fan-out, synthesis, re-convening

### 11.1 The shape the owner described

> The CEO agent is spawned through cron jobs. It invokes the HODs. Each
> invocation is a separate to-and-fro interaction. Each HOD may use tool calls
> to explore its own system and produce a report. With six systems, the CEO has
> six parallel meetings. Each produces an artifact. After all six, the CEO reads
> the artifacts, plans and articulates the systemic trajectory, and respawns the
> HODs for alignment if required.

That maps cleanly onto existing machinery:

| Step | Mechanism |
|---|---|
| CEO spawns on schedule | existing `CronScheduler::tick`, an ordinary cron job |
| Invokes each HOD | one DM thread per HOD, each a fresh spawn of that employee |
| HOD explores its own system | tool calls, **filtered by authority** (§2.4) |
| HOD produces a report | artifact, attached to the thread and to the worklog |
| CEO reads all artifacts | post-thread, injected into the CEO's next run |
| CEO articulates trajectory | decision objects + a narrative artifact |
| Respawn HODs for alignment | second round of threads, **bounded** |

### 11.2 The bound

A second round that can itself trigger a third round is a fork bomb with extra
steps. So: **`max_alignment_rounds = 2`** (initial + one alignment round), and
the CEO's decision to stop is itself a decision object. If alignment is
incomplete after round 2, the outcome is a recorded `unresolved` decision, not
a third round.

### 11.3 The meeting is not special

Because the CEO is an ordinary row (Q2), the CEO meeting **is** the DM protocol
with `turn_budget = 3` and a different message. The one addition is the
**artifact exchange**: each HOD's report is an artifact, and the CEO's synthesis
reads all artifacts from all threads in the round.

`artifacts` table: `artifact_id`, `author`, `kind` (`report` | `trajectory` |
`plan`), `content`, `thread_id`, `correlation_id`, `round`, `created_at`.

### 11.4 `org check` becomes the gate's instrument

`REMAINING-GAPS.md` GAP-2.4 found `cmd_check` returns `Ok` unconditionally. In
this architecture it becomes the **precondition check for every gate**: identity
validity, department rules, unfilled required capabilities, NULL departments,
and cycle-free `reports_to`. Exit contract:

- `0` — clean
- `1` — findings that block (gate would refuse)
- `2` — findings that warn (visibly unstaffed, but not blocking)

---

## 12. Schema versioning — resolved, because this design needs it

`REMAINING-GAPS.md` GAP-4.5 flagged that the org tables have no version marker,
and that Wave 3's first `ALTER TABLE` would be applied twice or skipped on an
older file. This design adds **two columns to `employees` and four new tables**,
so it must be resolved first.

Resolution, consistent with the one-family-per-DB-file invariant already
documented in `migrations.rs`:

- **New tables** — safe to add with `CREATE TABLE IF NOT EXISTS`. Idempotent.
- **New columns** (`reports_to`, `peers`) — need a migration. Additive
  `ALTER TABLE ... ADD COLUMN` in a single guarded pass, keyed on
  `PRAGMA table_info`, which is idempotent by construction and needs no
  file-wide version marker.
- **New databases** (`operant_decisions.db`) — fresh files, no migration.

So: no `PRAGMA user_version` use at all. The column-add helper checks
`table_info` and is safe to run on every open. This keeps the kanban family at
v1 untouched.

---

## 13. Data model summary

```
operant_kanban.db ─┬─ employees        (+ reports_to, peers)
                   ├─ notices          (unchanged — feeds, DMs, assignments)
                   ├─ kanban_*         (unchanged, stays at user_version 1)
                   ├─ departments      (NEW — mandate, protocols, rules, staffing)
                   ├─ assignments      (NEW — per-dept board, open seats)
                   └─ authority_grants (NEW — cross-department capability)
                   └─ dm_threads       (NEW — 3-turn budget, state)
                   └─ artifacts        (NEW — meeting reports, trajectories)
operant_worklog.db ─── worklog         (objective log — schema unchanged)
operant_decisions.db ─┬─ subjective_log (NEW — per-employee reasoning)
                      └─ org_decisions  (NEW — accepted, attributable decisions)
operant_cron.db ────── cron_jobs       (unchanged — spawns every employee)
```

---

## 14. Sequencing

Each step is independently shippable and leaves the system in a working state.

| # | Step | Why here |
|---|---|---|
| 1 | Schema versioning helpers (§12) | unblocks every later column add |
| 2 | Department membership command + NULL surfacing (§3.1) | answers Q4; feeds resolve |
| 3 | `departments` table + CLI (§3.3) | mandate/protocols/rules exist |
| 4 | Recipient resolver (§3.2) | makes the board readable — **keystone** |
| 5 | DM thread + 3-turn budget + turn awareness (§10) | this is the loop guard |
| 6 | Write barrier (§7) | one row per run, guaranteed |
| 7 | Persistent sessions + autocompact (§4) | unlocks DMs and personality |
| 8 | `subjective_log` + `org_decisions` + synthesis (§8, §9) | builds on the barrier |
| 9 | Persona injection into frozen prefix (§6) | personality becomes visible |
| 10 | `reports_to` / `peers` / authority scopes (§1, §2) | hierarchy exists |
| 11 | Authority-filtered tool registry (§2.4) | containment becomes real |
| 12 | `assignments` board + cross-dept grant check (§5) | first governed mutation |
| 13 | Authority injection into prompts (§6) | position changes the prompt |
| 14 | CEO loop + artifacts (§11) | alignment, once 1–13 hold |
| 15 | `org check` exit contract (§11.4) | observation for all of the above |

Steps 1–4 are Wave 2. Steps 10–12 are Wave 3. Steps 13–15 are Wave 4.

**The gate (GAP-2.3) is deliberately not in this list.** It stays dark until the
department `rules` array exists (step 3), because a gate that enforces rules
which do not exist yet would block on nothing and then be turned off.

---

## 15. Assumptions I am proceeding on

Flagged so they can be overridden cheaply:

1. `binding = true` for `Descendants`/`Org` decisions (§8.3). One field default.
2. Session registry is **load-on-demand with a 4-entry warm LRU**, not 102
   always-resident processes (§4.2). Different shape if the owner wants
   always-resident.
3. Autocompact is mandatory-with-floor at 4 000 tokens (§4.3). Deviation from a
   literal reading of "must autocompact", stated above.
4. Cross-department grants require the **grantor** to hold `Org` scope. A
   department head can grant within their own subtree but not across.
5. DM `turn_budget = 3` is the owner's example and is the default; it is a
   column, not a constant.
6. A department's `rules` are enforced by the identity gate, meaning rule
   violations are preconditions on starting work, not post-hoc findings.

## 16. Still open

Smaller than before, and none of them block step 1:

- Does the 3-turn budget apply to the CEO meeting per HOD, or per round? I
  default per-thread, which means per HOD.
- Should `depth` of the hierarchy be capped? An org chart with unbounded depth
  makes `Descendants` scope expensive. I default to soft-warn at depth 5.
- Are promotions (an employee gaining a scope) themselves decisions requiring
  CEO authority, or ordinary mutations with a reason? I default to decisions,
  because authority changes are exactly what the decision log is for.
