# Next implementation outline — after iter-541

**Date**: 2026-10-01
**Baseline**: `origin/main` @ `df2cf23d` (iter-541)
**Predecessor docs**: [`ORG-AUTHORITY-ARCHITECTURE.md`](ORG-AUTHORITY-ARCHITECTURE.md) §14,
[`ORG-AUTHORITY-DESIGN-REVIEW.md`](ORG-AUTHORITY-DESIGN-REVIEW.md) "Revised sequencing",
[`REMAINING-GAPS.md`](REMAINING-GAPS.md)

This is a plan, not an implementation. Every claim below was re-verified against
the code at `df2cf23d` rather than carried over from the docs, because two of the
three source documents predate iter-539/541 and describe a system that no longer
exists in that form.

---

## 0. Where the last two iterations actually left the system

Worth stating precisely, because the source docs are stale here and the next
agent should not trust them on this point.

iter-539 landed five stores (`department_db`, `authority`, `decisions_db`,
`dm_thread`, `resolver`) plus an idempotent `schema.rs`. iter-541 wired three of
them into `open_org_db` and gave the CLI four reachable subcommand groups
(`org department`, `org decision`, `org grant`, `org dm`).

**What that means for the plan**: the original §14 sequence listed "departments
table + CLI" at step 4 and "recipient resolver" at step 5 and "DM thread + 3-turn
budget" at step 6. Those three are **done**. The gaps ledger's `GAP-2.1` (loop
guard), `GAP-2.2` (resolver), and the department/DM rows are also closed. Do not
re-plan them.

The corrected §14 sequence in the design review is still the right spine, but its
first six steps are now partly consumed and the numbering has to be re-derived.

---

## 1. The corrected starting point

Steps 2, 4, 5, and 6 of the design review's revised sequence are landed. What
remains, in dependency order:

| Was | Step | Status @ `df2cf23d` |
|---|---|---|
| 1 | Per-employee session substrate | **not started** — the blocker |
| 2 | Schema versioning helpers | **done** (iter-539, `schema.rs`) |
| 3 | Department membership + NULL surfacing | **done** (iter-541, `unstaffed_employees`) |
| 4 | `departments` table + CLI | **done** (iter-539 + 541) |
| 5 | Recipient resolver | **done** (iter-539) |
| 6 | DM thread + 3-turn budget | **done** (iter-539 + 541) |
| 7 | Write barrier | **not started** |
| 8 | `Autocompactor` wiring | **not started** |
| 9 | `subjective_log` + `org_decisions` | **store done** (iter-539); **not fed** |
| 10 | Persona synthesis + frozen-prefix injection | **not started** |
| 11 | `reports_to` / peers / authority scopes | **store done**; **hierarchy absent** |
| 12 | Authority-filtered tool registry | **not started** |
| 13 | `assignments` board + grant check | **not started** |
| 14 | CEO loop + artifacts + fan-out | **not started** |
| 15 | `org check` exit contract | **partly done** (`org department findings` exits 1; the umbrella gate does not) |

---

## 2. Why step 1 is still the blocker

Re-verified at `df2cf23d`, not quoted from the review:

- `OperantAgent` holds **one** `conversation: Arc<RwLock<Vec<Message>>>`
  (`agent/mod.rs:346`). There is no session store and no per-employee key.
- `clear_history()` is still called before each cron job
  (`cronjobs/scheduler.rs:277`) and on session change in the gateway
  (`gateway_runner.rs:564`). Every scheduled run therefore starts blind.
- Cron executes `for job in due_jobs` (`cronjobs/scheduler.rs:95`) — strictly
  sequential. The CEO fan-out in §11 is genuinely new concurrency work.
- No `Autocompactor` symbol exists anywhere in `operant-core`. The
  context-overflow path (`agent/compress.rs`) is a *different* mechanism from the
  per-run compaction §4.5 asks for.
- `Employee.persona` is a nullable JSON column (`org/employee.rs:105`) that
  nothing reads. Persona injection is entirely unwired.
- No `assignments` table exists (`ls org/` confirms 13 modules, none is one).
- `tools/builtin.rs` contains **zero** references to `authority`, so nothing
  constrains tool availability by scope.

Everything from step 7 onward attaches to a per-employee session boundary. Doing
them before step 1 means building on a single global conversation slot that
cannot address an individual employee.

---

## 3. Proposed order for the next stretch

Six steps. Each is independently shippable and leaves the system working.

### Step A — Per-employee session substrate *(the big one)*

Replace the single conversation slot with a session-keyed store plus a
per-employee lock. `clear_history()` stops being the session boundary.

Design constraints that are already decided and must not be re-litigated:

- **Load-on-demand with a bounded warm cache**, not 102 resident processes. The
  owner's instruction and the review agree.
- The key is `(employee_id, session_key)`. A fresh employee run must see its own
  history and nobody else's.
- The store must be swappable per agent instance so the TUI and gateway paths
  keep working with a single implicit session.
- `clear_history()` is retained but redefined as *release this session's slot*,
  not *wipe the global history*.

Risk: this is the one step that touches the agent core. It is first precisely so
it can be de-risked while there is still room to back it out.

Verification that matters: two interleaved employee runs must not see each
other's turns, and a released session must not leak its history into the next
run under the same employee.

### Step B — Write barrier

Every employee run must pass a write barrier before completion:

- one objective **worklog** row
- **subjective** reasoning entries
- **decision** objects for organizational changes

The seam already exists and is nearly free: `OperantAgent.turn_end_bus`
(`agent/mod.rs:377`) is an `Option`, is `None` by default, and the emit sites
(`run.rs:1271`, `stream.rs:888/926/1012`) already check it with a documented
zero-subscriber cost. **Nothing in the CLI ever calls `with_turn_end_bus`** —
so the barrier has a mount point but no subscriber.

That makes Step B smaller than it looks: attach a subscriber that writes the
three artifact kinds, and make the run *fail* if the write fails. The design
calls this a precondition of completion, so a silent write failure is the
failure mode to guard.

### Step C — Autocompaction wiring

Per §4.5, sessions compact after scheduled execution, with a **compaction floor**
so short sessions are not reduced to a lossy summary-of-summary.

Blocked on Step A: the compressor needs a per-employee conversation to compact.
Note the existing `compress_context_overflow` is a different mechanism and should
not be conflated with this.

### Step D — Hierarchy fields + authority-filtered tools

`reports_to` / peers, then make authority actually constrain the **tool
registry**, not merely board mutations — this is an explicit owner requirement
and is the single most load-bearing remaining correctness property. §2.4: a
non-crossing scope is offered its own department's tool surface with no grant
recorded; a crossing scope requires an explicit, attributable, expiring grant.

`GrantDb` and the scope lattice already exist; what is missing is the hierarchy
they read from and the enforcement point in tool resolution.

### Step E — Assignment boards

First genuinely *governed* mutation: the cross-department grant check gates it.
This is also the first place the authority lattice is exercised by something
other than a test.

### Step F — Concurrent CEO meeting loop

Fan out to HODs, collect report artifacts, synthesize a trajectory, optionally
reconvene once for a bounded alignment round. Carries the D3 concurrency work.

Finish by widening the `org check` exit contract from `org department findings`
to the umbrella §11.4 gate.

---

## 4. Sequencing constraints worth respecting

1. **A before everything.** B through F all attach to a per-employee session
   boundary. Skipping A means building on an unaddressable global slot.
2. **B before D.** The barrier is what makes an unauthorized mutation
   attributable. Adding authority-filtered tools before every run is guaranteed
   to leave a worklog means the first governed action in the system is also the
   first unauditable one.
3. **D before E.** A governed mutation needs something to check against. E before
   D would be a board nobody can enforce.
4. **C is genuinely blocked by A** and by nothing else.

---

## 5. What is deliberately not in this outline

- **Importing organism cron jobs, prompts, ventures, or business data.** Port the
  general architecture only.
- **The identity gate activation.** The design keeps it dark until the
  department `rules` array exists — that array now exists, so this becomes
  *possible*, but it is an opt-in policy decision for the owner, not a
  sequenced step. Flagged, not scheduled.
- **`retention_gc` wiring** (`GAP-2.6`) and the **`CHANGELOG` entry** for the
  Wave 1 breaking CLI change (`GAP-X.1`). Both real, both small, both
  housekeeping. `GAP-X.1` in particular is a documentation debt that grows
  every iteration.
- **A second `Autocompactor` design pass.** One exists in the design; do not
  write a competing one.

---

## 6. Open questions the owner may want to answer before Step A

These are the ones that would change the design, not merely tune it. None blocks
starting Step A.

1. **Session key shape.** Is a run's identity `(employee_id, cron_job_id)`, or
   should one employee have a single continuous session across all its jobs?
   The answer determines whether a per-employee lock is even the right
   primitive.
2. **Warm cache bound.** What is the ceiling on resident warm sessions before an
   LRU eviction? Needs a number, since "bounded" is currently unquantified.
3. **Barrier failure semantics.** If the worklog write fails, does the run fail,
   or does it complete and raise a finding for `org check` to report? The design
   says "precondition of completion", which implies the former — but that is a
   availability-vs-auditability tradeoff the owner should own.
4. **Barricade against fabricated seats.** The owner rule is to never fabricate
   capabilities for vacant or invalid seats. Confirm whether a grant to a seat
   that is currently vacant should be *refused* or *accepted but inert*. These
   have very different audit implications.

---

## 7. Suggested first concrete task

**Step A, first slice: the session store in isolation.**

- New `operant-core/src/session/` module: `SessionKey`, `SessionStore`, a bounded
  LRU of warm sessions, and a `release` operation.
- No agent-core changes yet. `OperantAgent` keeps its single slot; the store is
  proven by its own tests (create, load, release, evict, per-key isolation).
- Land it as an independent iteration.

Why this slice: it is the riskiest step in the plan, it has no dependency on any
concurrent work, and shipping it separately means the agent-core wiring becomes a
reviewable second iteration rather than one large risky change.
