# Packet F — Concurrent CEO meeting loop (Step F)

## Goal

Fan out to heads of department, collect their report artifacts, synthesize a trajectory, and
optionally reconvene **once** for a bounded alignment round.

Two parts, and the first is a correctness fix that must not be skipped:

**Part 0 (correction):** `run_agent_job` in the cron scheduler still calls the **global**
`clear_history()` before every job. Under the per-session substrate that landed in iter-548 that
is now wrong in a specific way — see "Part 0 is mandatory" below.

**Part 1:** the concurrent CEO meeting loop itself.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/org/ceo_loop.rs` (you create this)
- `crates/operant-core/src/cronjobs/scheduler.rs` (Part 0 only — see below)

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `assignment.rs`, `write_barrier.rs`,
`authority.rs`, `hierarchy.rs`, `tools.rs`, `employee.rs`, `session/mod.rs`,
`session/autocompact.rs`, or any file under `crates/operant-cli/`. The coordinator registers
your module and wires the CLI.

**If you believe you need to change a file you do not own, do NOT make it — report it in your
final message.**

## Part 0 is mandatory — read this carefully

`crates/operant-core/src/cronjobs/scheduler.rs` around line 277, in `run_agent_job`:

```rust
self.agent.clear_history().await;
match self.agent.run(job.prompt.clone()).await {
```

Since iter-548, `clear_history()` **discards the addressed session** (it drops it from the
`SessionStore`, not just the in-memory slot). The owner requirement is explicit:

> **Sessions persist across jobs.** Session identity is `employee_id`. Compaction happens *after*
> scheduled execution, within the persistent session.

So calling `clear_history()` before every job **destroys the persistence the substrate exists to
provide**. Your Part 0 fix: scope the scheduled run to a per-employee session
(`set_session_id(employee_id)`), and replace the global wipe with the correct lifecycle — do not
call `clear_history()` as a "start fresh" gesture.

Read `crates/operant-core/src/session/mod.rs` and `crates/operant-core/src/agent/events.rs`
(`set_session_id`, `clear_history`) before touching the scheduler. The distinction that matters:

- `release` / handing a session back = polite, keeps the transcript
- `discard` / `clear_history` = destructive, removes it

**If a job legitimately must start clean, that is a per-job session identity decision — make it
explicit and justified in a comment, not an incidental `clear_history()` call.**

## Part 1 — The CEO loop

**CEO is an ordinary employee participating through normal board interactions.** Do not build a
special privileged path for the CEO. The CEO acts through the same assignment-board and notice
surfaces as everyone else; its authority comes from its `AuthorityScope`, not from a code path
that bypasses checks.

Requirements:

1. **Fan out to heads of department** — read `org/hierarchy.rs` and `org/department_db.rs` for who
   the heads are, and `StaffingFinding` for departments with an unfilled head. A department with
   no head is a **vacancy to report**, not a slot to fill with a guess.
2. **Collect report artifacts** — heads report through the normal artifact path (worklog /
   subjective entries from the write barrier in `org/write_barrier.rs`).
3. **Synthesize a trajectory** — one coherent record of the meeting, not a concatenation.
4. **Optionally reconvene ONCE for a bounded alignment round.** "Optionally", "once", and
   "bounded" are all load-bearing. A reconvene loop that re-triggers itself is a fork bomb on
   your own org. Make the bound structural (a parameter with a hard default), not a comment.
5. **CEO/head decisions default to binding while preserving visible dissent.** See
   `org/decisions_db.rs` for the established shape. Dissent must stay visible — never suppress
   it in synthesis.
6. **Concurrency must be real but bounded.** The agent has an 8-worker concurrent tool pool; do
   not spawn unbounded fan-out. Pick a bound, document why, and make it a named constant.

## Semantics that are NOT negotiable

1. **A missing report is visible.** If a head does not report, the synthesis says so. Do not
   synthesize a report that was never given.
2. **The reconvene bound is structural**, and a test must prove it cannot exceed it.
3. **Never claim `PRAGMA user_version`** — that counter belongs to the kanban family alone.

## Tests you must write

`crates/operant-core/tests/org_ceo_loop.rs` (you own this new test file), plus inline tests in
`ceo_loop.rs` as you see fit.

Required, each with a real assertion:
- the CEO acts through **normal board interactions**, with no privileged bypass
- a head that did not report appears as **missing**, not silently synthesized
- **dissenting** heads' dissent **survives synthesis** and stays visible
- reconvene happens **at most once** — a test that proves the bound cannot be exceeded
- a department with **no head** is reported as a vacancy, not filled with a guess
- Part 0: a second scheduled job for the **same employee sees the first job's transcript**
  (this is the persistence requirement, and it is the test that proves Part 0 works)
- Part 0: two different employees' scheduled jobs do **not** see each other's turns

**Mutation-prove your tests.** Break your implementation deliberately (leave the
`clear_history()` in place, or remove the reconvene bound), confirm the tests FAIL, restore,
report the counts.

## Verification

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_ceo_loop
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test agent_session_isolation
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests**, nor the
`agent_session_isolation` suite (8). Run and report the counts.

Other agents may be editing sibling files concurrently. If you see a compile error in a file
you do **not** own, **ignore it and report it**.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` / `.expect()` in production code.** The clippy gate denies both.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, what you did for Part 0 (quote the before/after of the
`clear_history()` line), the public API the coordinator needs to wire the CEO loop, your
fan-out bound and reconvene bound with justification, test counts, mutation results, the full lib
suite and `agent_session_isolation` counts, any compile errors in files you do not own, and the
verification output tail. If something is incomplete, say so plainly.