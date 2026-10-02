# Organism OS — next implementation outline

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `1cb88511` (iter-551)
**Predecessor**: [`NEXT-IMPLEMENTATION-OUTLINE.md`](NEXT-IMPLEMENTATION-OUTLINE.md)
(written 2026-10-01 at iter-541, now superseded — its "Step A is the blocker"
premise was resolved by iter-548)

Every claim below was re-verified against the code at `1cb88511` on 2026-10-03,
not carried over. Where the predecessor said a thing was unbuilt and it now
exists, that is recorded rather than silently renumbered.

---

## 0. What iter-550 and iter-551 actually changed

Both packets had been sitting as **untracked files** — never committed, and one
of them never even compiled.

`session/autocompact.rs` was an orphan: `lib.rs:110` declares `pub mod session`,
but `session/mod.rs` never declared `mod autocompact`, so the file was outside
the module tree. `cargo check --lib` passed *because it never saw it*.
`tests/session_autocompact.rs` failed `E0432`. iter-550 added the declaration;
the 766 lines type-checked for the first time and 15 tests now run.

`write_barrier.rs` shipped two failing tests, both test-side:

- `a_run_that_changed_the_organisation_writes_its_decision_objects` — the
  `authorless_decision` helper passed its meeting note into the 6th argument of
  `OrgDecision::new(id, subject, scope, decided_by, rationale, reason)`, i.e.
  the `reason` slot. It was therefore non-blank and the substitution at
  `write_barrier.rs:701` correctly declined to fire. Fixed the helper.
- `the_barrier_leaves_the_kanban_user_version_untouched` — dropped the fixture,
  and with it the `TempDir`, before reopening, so it asserted on a
  `SqliteFailure` rather than on `user_version`. Removed the drop; a second
  `Connection::open` on a live file is valid SQLite.

iter-551 then repaired forward a defect iter-550 introduced: it committed
`tests/org_authority_tools.rs` but not the `tools.rs` implementation the test
imports, publishing a test target that could not compile. Force-pushing `main`
is forbidden, so it was fixed in a following commit rather than amended.

**Current measured state:** 2235 lib tests pass, 2 pre-existing
`tools::kernel` failures that are present at iter-549 and unrelated (no
`kernel.rs` file exists; neither commit touches `tools::kernel`), plus
8 `agent_session_isolation`, 6 `org_authority_tools`, 15 `session_autocompact`.

---

## 1. The honest status table

"Built" here means *compiled and tested*. It does **not** mean *on a run path*.

| Step | Built | On a run path | Note |
|---|---|---|---|
| A per-employee session substrate | ✅ iter-548 | ✅ | 8 isolation tests |
| B write barrier | ✅ iter-550 | ❌ **no caller** | 38 inline tests |
| C autocompaction | ✅ iter-550 | ❌ **no caller** | 15 tests |
| D hierarchy + §2.4 tool authority | ✅ iter-550/551 | ❌ **no caller** | 6 + inline tests |
| E assignment boards | ❌ | ❌ | absent |
| F concurrent CEO loop | ❌ | ❌ | absent |

The single most important line in this document: **Steps B, C and D are three
finished subsystems that nothing calls.** The organ can currently record a
worklog, compact a session and filter tools by authority — provided something
invokes them. No run does.

Verified absent: `operant-cli/src` contains no reference to `write_barrier` or
`autocompact`; `with_turn_end_bus` (`agent/builders.rs:280`) has no caller.

This is the same class of defect `REMAINING-GAPS.md` calls `DARK` — built, not
reachable. The gap ledger predates all of this and its Wave 2-4 rows are stale;
this outline supersedes its status column, not its findings.

---

## 2. Step B′ — mount the write barrier *(the next iteration)*

The barrier has two entry points, deliberately distinct
(`write_barrier.rs:743`):

- `WriteBarrier::apply(&request)` — the **postcondition**. Returns `Err`, and
  the caller fails the run. This is the one §11 wants.
- `WriteBarrier::attach_subscriber(...)` — the **passive observer**. Cannot fail
  a run; records to a `BarrierFailureLog`.

A `TurnEndBus` subscriber cannot make a run fail — that is why these are two
functions. So the postcondition has to be awaited by the run path itself, not
merely observed.

**Mount point:** `run_agent_job` (`cronjobs/scheduler.rs:274`). It already has
the run's outcome in hand as `(success, output, final_response, error_msg)`.

The shape is roughly:

```
1. build WriteBarrier::for_app(&db_path)
2. await WriteBarrier::apply(&request)  after self.agent.run(...) returns
3. on Err: the run is a failure regardless of `success`
```

Constraint from the design that must be honoured: the barrier writes the
**objective** worklog row, **subjective** reasoning, and **decision** objects.
`run_agent_job` currently calls `clear_history()` at `:277` before every run.
Under the iter-548 session substrate that now means "release this session's
slot", so the turn data the barrier needs must be captured before or across
that call, not after.

**Why this before C or E:** the design's own sequencing says B before D,
otherwise the first governed action in the system is also the first
*unauditable* one. D's enforcement point is built and waiting; B is what makes
a governed action attributable.

**Verification that matters:** a scheduled run that completes must leave
exactly one worklog row; a run whose barrier write fails must report failure
and must not be recorded as successful.

---

## 3. Step C′ — autocompact after scheduled execution

`Autocompactor::compact(&key)` (`session/autocompact.rs:429`) and the
summarising variant `:448`. It takes an `Arc<SessionStore>` and an
`Arc<Database>` — both already exist after iter-548.

The design wants compaction **after scheduled execution**, with a
**compaction floor** so short sessions are not reduced to a
summary-of-summary. That floor is implemented: `COMPACTION_FLOOR_TOKENS = 4_000`,
`KEEP_HEAD_MESSAGES = 3`, `KEEP_TAIL_TOKENS = 8_000`.

This is the same `run_agent_job` seam as B′, and the order matters: **compact
after the barrier has read what the run produced**, or the barrier is
compacting against a transcript it is about to write a row about.

Note `compress_context_overflow` (`agent/compress.rs`) is a *different*
mechanism and the two must not be conflated.

---

## 4. Step D′ — hierarchy and §2.4 enforcement, on a real agent

`hierarchy.rs` models the reporting line as explicit edges over
`HierarchyEntry` rather than as a field on `Employee`. That was forced:
`Employee` is constructed from struct literals in eight files, none using
`..Default::default()`, so adding a field breaks the crate. When a later packet
adds the column, `HierarchyEntry` is what reads it.

Two things remain for D to be real:

1. **The edges have to come from somewhere.** `hierarchy.rs` is a pure query
   surface over a set of edges. Nothing populates that set from the employee
   table, because the column does not exist yet. Until it does, the tree is
   empty and every hierarchy answer is trivially "no relation".
2. **The registry has to be consulted at agent construction.** `ToolRegistry`
   now carries `tool_authority` and the filtering methods exist, but
   `OperantAgent::new` at `cmd_acp.rs:156` and `main.rs:1844` construct a
   registry with no actor attached, so every lookup is unrestricted.

Decide explicitly whether to (a) add `reports_to` to `Employee` and fix the
eight construction sites, or (b) populate edges from a separate table. (a) is
the design's stated intent; (b) is cheaper. **This is an owner decision, not a
sequencing detail.**

---

## 5. Step E — assignment boards

Not started. First genuinely *governed* mutation: the cross-department grant
check gates it, and it is the first place the authority lattice is exercised by
something other than a test.

Requires D′ to be live first — a governed mutation needs something to check
against. E before D would be a board nobody can enforce.

---

## 6. Step F — concurrent CEO loop

Not started. Fan out to HODs, collect report artifacts, synthesize a
trajectory, optionally reconvene once for a bounded alignment round.

Carries the D3 concurrency design and the largest open design risk in the
program. `scheduler.rs:95` is still `for job in due_jobs` — strictly
sequential — so nothing about fan-out exists yet. This also widens the `org
check` exit contract from `org department findings` to the umbrella §11.4 gate.

**Concurrency safety note:** iter-548 gave every session its own slot, which
removed the reason fan-out was unsafe. It did not make fan-out *correct* —
shared SQLite writes and the barrier's write ordering across N concurrent runs
are unexamined.

---

## 7. Sequencing constraints

1. **B′ before D′.** The barrier is what makes an unauthorized mutation
   attributable.
2. **B′ before C′**, same seam: the barrier reads the run's artifacts.
3. **D′ before E.** A governed mutation needs something to enforce against.
4. **E before F.** The CEO loop reads boards.
5. **The identity gate stays dark.** `with_org_gate` exists
   (`scheduler.rs:60`) but activation is an owner policy decision, not a step.
   The department `rules` array it waits on now exists, so it is *possible* —
   still opt-in.

---

## 8. What is deliberately out of scope

- Importing organism cron jobs, prompts, ventures, or business data. Port the
  architecture, not the business.
- `retention_gc` wiring (GAP-2.6) and the CHANGELOG entry for the Wave 1
  breaking CLI change (GAP-X.1). Both real, both housekeeping.
- `GAP-4.4` AD/RG lifecycle — a separate packet.

---

## 9. Suggested first concrete task

**iter-552: mount the write barrier in `run_agent_job`.**

Smallest change that converts a finished subsystem into a reachable one, on a
seam that already exists and already has both the run's outcome and the session
key in hand. `WriteBarrier::apply` is synchronous and already fully tested, so
the risk is in the plumbing — which is exactly what should come next after
three packets landed unwired.

Acceptance:

- a completed scheduled run leaves exactly one worklog row;
- a barrier write failure makes the run report failure, not success;
- `cargo test -p operant-core --lib` green apart from the 2 pre-existing
  `tools::kernel` failures;
- **verified in a clean worktree at the pushed commit, not the working tree** —
  the iter-550 defect shipped precisely because the working tree masked it.