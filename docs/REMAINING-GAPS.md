# Remaining implementation gaps — Waves 2 through 4

> **Status:** verified inventory, 2026-10-01, against `origin/main` = `c5c9271a`
> (iter-528). Every claim below was checked by grep or by building a worktree
> from `origin/main` alone, not inferred from a design document.
>
> **Scope note.** This covers the owner's organism-OS program only. A separate
> TUI effort is in flight on this repo (iter-519..528 plus uncommitted work);
> its files are not mine and are excluded here.

## How to read this

- **Status values:** `DARK` (built, not reachable), `DEAD` (built, no caller),
  `MISSING` (never built), `PROMISED` (documented as shipped, not built).
- **Wave 2 is a safety+keystone wave.** GAP-2.1 (loop guard) must precede any
  multi-agent interaction, because GAP-2.2 makes notices reachable by *groups*
  for the first time. Enabling reachability before the guard converts a design
  gap into a fork bomb.
- **Nothing in Wave 3 or 4 is honestly buildable before GAP-2.2.** Every layer
  needs to answer "who is in this department" and "who reports to whom".

Current state, verified:

| Check | Result |
|---|---|
| `cargo check -p operant-cli --bin operant` on `origin/main` | **0 errors, 0 warnings** |
| `cargo test -p operant-core --lib` | 2047 passed (iter-518) |
| org suites | 83 passed across 4 files (iter-518) |
| CLI `org_reason` e2e | 10 passed (iter-518) |

The two `Color`-not-found errors that appear when building the **working tree**
are in `tui/agents_view.rs` and `tui/mcp_view.rs`, which are dirty with another
agent's WIP. `origin/main` is clean; I am not touching those files.

---

## Wave 2 — safety and the keystone

### GAP-2.1 — No loop guard. The owner's "infinite loops" is a live hazard.

**Status: MISSING.** `grep -rniE 'max_depth|loop_guard|max_hops|interaction_budget' crates/operant-core/src/org/` → no hits.

The owner said interaction "can create into infinite loops". Once GAP-2.2 lands,
A responding to B creates a fresh notice; B can respond to that; nothing bounds
the chain. The organism has no depth counter or budget either, so a port
inherits the bug rather than fixing it.

Acceptance:
- a hop counter on the correlation chain, threaded through the typed API;
- a per-thread generation ceiling (default 4) after which a notice is recorded
  but marked terminal;
- a per-employee per-tick interaction budget, mirroring the existing
  `DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK` pattern at `notice_db.rs`;
- budget-skipped work must not consume budget — the convention already
  established at `notice_db.rs:844`;
- a test proving A→B→A terminates, and one proving the ceiling is not
  circumvented by a fresh `correlation_id`.

### GAP-2.2 — No recipient resolver. The board is write-only for groups.

**Status: DEAD.** `NoticeIdentity` and `NoticeInboxMatcher` exist
(`notice.rs:354-408`) and correctly derive a reader's four selectors — but
`grep -rn 'query_inbox\|\.inbox(' crates/ | grep -v tests/` returns only
definitions and internal calls. **Nothing in production ever builds a
`NoticeIdentity` from an `Employee` row, and nothing calls the inbox.**

So today `dept:engineering` reaches nobody, `team:` reaches nobody (its doc
comment at `notice.rs:81-83` says so honestly), and the only working feed
shapes are broadcast and direct `agent:<id>`.

Compounding this: **`employees.department` is NULL for every backfilled row**
(`employee_db.rs:250` sets `department: None`). The CLI accepts `--department`
on create/update, but the backfill that produces all 102 real employees never
sets it. A resolver alone would therefore resolve every employee to "no
department".

Acceptance:
- derive a `NoticeIdentity` from an `Employee` row in one place, so the mapping
  cannot be reinvented per caller;
- backfill must populate `department` from a declared source, **or** the absence
  must be surfaced as an unstaffed-configuration finding rather than silently
  leaving every employee departmentless;
- a test proving a `dept:` notice reaches that department's members' inboxes and
  *not* an outsider's.

### GAP-2.3 — The identity gate is dark.

**Status: DARK.** `with_org_gate` is defined at `cronjobs/scheduler.rs:60` and
called from tests only. No config key, no boot path.

This is deliberate and correct (a gate blocking all 102 jobs on upgrade is an
outage) but it means the fail-closed guarantee is **not in force**. Measured
block set is **4 of 102**: two disabled legacy jobs, two enabled empty-prompt
placeholders.

Acceptance:
- an opt-in config key, off by default;
- `doctor --gate` and/or `org check` reporting the same 4 rows;
- entry points refusing to spawn on a gate failure;
- the default path provably unchanged (the current dark behaviour must remain
  reachable, or this becomes a breaking change with no migration).

### GAP-2.4 — `org check` cannot fail.

**Status: PROMISED but inert.** `cmd_check` (`cmd_org.rs:878`) prints counts
and returns `Ok(())` unconditionally. The decisions doc lists it as the
gate's input, but there is no exit code and no threshold.

Acceptance: a documented exit contract (0 clean, non-zero when the gate would
block), so it can be used in CI and by GAP-2.3.

### GAP-2.5 — Documented CLI subcommands that do not exist.

**Status: PROMISED.** `WAVE1-DECISIONS.md` §3.5 lists `org route`, `org show`,
and `org notice inbox` as read-only subcommands. `grep -c` for each in
`cmd_org.rs` returns **0**. The real surface is 7 groups: `employee`, `notice`,
`worklog`, `sync`, `import`, `check`, `list`.

Acceptance: either implement them or remove them from the doc. `org notice
inbox` is the one that matters — it is the only read path onto GAP-2.2.

### GAP-2.6 — `retention_gc` has no caller.

**Status: DEAD.** `NoticeBoard::retention_gc` (`notice_db.rs:569`) is fully
built with batching limits, and `grep -rn retention_gc crates/ | grep -v tests/`
finds only its own definition. Notice TTL is therefore never enforced; rows
accumulate forever.

### GAP-2.7 — The worklog is not fed by anything.

**Status: DEAD.** `grep -rn worklog crates/operant-core/src/agent/
crates/operant-cli/src/tui/` → no hits. `WorklogDb::append` is reachable only
from `org worklog append` (a human at a terminal). The framework-side write
path that the schema was designed around (`TurnObservation::from_turn_end`) is
not wired to session end, so **no agent run has ever written a worklog row**.

This is the single largest gap between "the worklog exists" and "the chronograph
has anything to synthesise from". It was blocked earlier because the agent tree
was owned by another agent; that ownership has since changed.

---

## Wave 3 — structure

### GAP-3.1 — No org chart.

**Status: MISSING.** `grep -rniE 'reports_to|peers' crates/operant-core/src/org/`
→ no hits. The organism's `AgentSpec` has both (`agent_fabric.py:134-135`).

Without a reporting line there is no department head, no set of direct reports
for a CEO to convene, and no authority model for cross-department change.

Acceptance: `reports_to` and `peers` on `Employee`, populated by backfill where
declarable and left NULL (visibly) where not; a cycle check, because
`reports_to` in a free-text column will eventually contain a loop.

### GAP-3.2 — No department assignment board.

**Status: MISSING.** No type, no table, no CLI. Depends on GAP-2.2 (membership)
and GAP-3.1 (ownership).

### GAP-3.3 — No team membership.

**Status: MISSING.** `NoticeIdentity::on_teams` exists and is unreachable;
`team:` selectors are documented as reserved for exactly this.

---

## Wave 4 — alignment

### GAP-4.1 — No CEO identity or convening mechanism.
### GAP-4.2 — No decision object, so alignment has nowhere to land.
### GAP-4.3 — No authorized structural-change path.
### GAP-4.4 — No AD/RG lifecycle.

**Status: MISSING.** `grep -rniE 'ArchitecturalDecision|arch_decision|research_gap' crates/operant-core/src/org/` → no hits. The organism-aligned decisions doc promised an AD/RG lifecycle with a staleness-checked derived registry.

Also absent, promised by `WAVE1-DECISIONS.md`: the **persona contract** (the
`persona` column exists and is always NULL, `employee_db.rs:263`), the
**maintenance contract** (`grep -niE maintenance crates/operant-core/src/org/` →
no hits), and the SELF/PEER/DEPT/ORG self-evolution loops.

### GAP-4.5 — Schema versioning for the org tables.

**Status: RISK, not yet a bug.** The org tables use bare
`CREATE TABLE IF NOT EXISTS` and deliberately do **not** use
`crate::migrations::migrate`, because `PRAGMA user_version` is file-wide and
`operant_kanban.db` is already claimed by the kanban family at v1
(`employee_db.rs:157-170`). That was correct when the schema was frozen.

It stops being correct the moment Wave 3 adds `reports_to`: a new column needs
`ALTER TABLE`, and there is no version marker to know whether it has been
applied. **This must be solved before GAP-3.1**, or the first column addition
will be applied twice or skipped on an older file.

---

## Cross-cutting

### GAP-X.1 — Wave 1 shipped a breaking CLI change.
`kanban block --reason` went from optional (defaulting to the literal
`"Blocked via CLI"`) to required. It is in the commit body and in
`WAVE1-DECISIONS.md` §3.5, but **not yet in `CHANGELOG.md`** — which the
project's own rules require for a user-facing behaviour change.

### GAP-X.2 — `org import` remains deliberately unimplemented.
Correct per the owner's instruction not to import organism data. It validates
its reason and then fails loudly. No action needed; listed so it is not
mistaken for an oversight.

---

## Recommended order

```
GAP-2.1 loop guard          (safety — before reachability exists)
GAP-2.4 org check exit code (needed to observe 2.1 and 2.3)
GAP-2.3 gate activation     (opt-in; makes the guarantee real)
GAP-2.7 worklog feed        (gives the chronograph any content at all)
GAP-2.2 recipient resolver  (keystone)
GAP-2.5 notice inbox CLI    (the only read path onto 2.2)
GAP-2.6 retention GC wiring
GAP-X.1 CHANGELOG entry
  ── schema versioning decision (GAP-4.5) BEFORE any column addition ──
GAP-3.1 org chart
GAP-3.2 assignment boards
GAP-3.3 teams
GAP-4.1..4.4 alignment + AD/RG + contracts
```

I would put **GAP-2.7 ahead of GAP-2.2** deliberately. The resolver makes the
board *reachable*; the worklog feed makes it *worth* reading. Doing the resolver
first produces a working group feed that is empty forever, which will read as
"the resolver is broken".

---

## Questions only the owner can answer

> **ANSWERED 2026-10-01.** The owner answered Q1–Q6 in a design session. The
> full architecture is [`ORG-AUTHORITY-ARCHITECTURE.md`](ORG-AUTHORITY-ARCHITECTURE.md);
> the answers are §2.1 (Q1), §8.3 (Q2, Q3), §3.1 (Q4), §9.3 (Q5), §10.3 (Q6).
> The original question text is kept below for auditability.
>
> The answers changed three things in this document:
> - **GAP-2.1** is now implemented as a shared 3-turn DM thread budget (§10),
>   not a per-employee per-tick interaction cap. The audit's mechanism was the
>   wrong one; the thread budget is both simpler and defeatable-proof.
> - **GAP-2.2** now also carries a data requirement: `department` must be
>   operator-set, and NULL is surfaced rather than tolerated.
> - **GAP-4.5** is resolved by never using `PRAGMA user_version` for the org
>   tables — guarded `ALTER TABLE` keyed on `table_info` instead (§12).

**Q1 (blocking, Wave 3 security boundary).** When a department head refactors
their assignment board: is `reports_to` sufficient authority, or does
reassigning a role *across* departments need an explicit capability grant? I do
not want to infer this — it is an authorization boundary, and the wrong default
is either a privilege hole or a board nobody can edit.

**Q2 (blocking, Wave 4).** Is the CEO an ordinary `Employee` row that
participates through board interactions, or a distinct runtime role? If it is a
row, the alignment meeting is just a correlated notice thread and needs no new
machinery — that is much cheaper and I would default to it.

**Q3 (blocking, Wave 4).** Does a CEO alignment decision *bind* a dissenting
head, and is dissent recorded explicitly as a first-class row? If yes, the board
needs a decision object; if dissent is just noise, it does not.

**Q4 (Wave 2, affects GAP-2.2's honesty).** `employees.department` is NULL for
all 102 backfilled employees and the cron jobs carry no department field. Where
should department membership come from — an explicit operator-set field, a new
cron field, a mapping file, or should it stay NULL and be surfaced as an
unstaffed configuration? I will not invent a derivation from job names.

**Q5 (Wave 2, affects GAP-2.7).** What granularity should worklog rows be
written at: every turn, every session, or only on notable outcomes? "Every turn"
is high volume and the board would drown; "only notable" needs a definition of
notable, and I would rather you pick than have me guess a heuristic.

**Q6 (Wave 2, affects GAP-2.1).** What should happen when the interaction
ceiling is hit: record the notice as terminal and stop, or drop it silently? I
default to record-and-mark-terminal, since silent loss is worse than a visible
dead end. Confirm if you disagree.