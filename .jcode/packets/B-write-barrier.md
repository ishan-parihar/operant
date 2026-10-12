# Packet B — Write barrier (Step B)

## Goal

Every employee run must pass a **write barrier** before completion. The barrier writes three
artifact kinds:

1. one objective **worklog** row
2. **subjective** reasoning entries
3. **decision** objects for organizational changes

A silent write failure is the failure mode to guard.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/org/write_barrier.rs` (you create this)
- `crates/operant-core/src/turn_end.rs` (you may ADD to it, minimally — see "Mount point")

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `session/mod.rs`, `employee.rs`, `authority.rs`,
`tools.rs`, `employee_db.rs`, `worklog_db.rs`, `decisions_db.rs`, or any file under
`crates/operant-cli/`. Other agents own those in this wave and are editing them right now.
The coordinator registers your module and wires the CLI. **If you believe you need to change a
file you do not own, do not change it — report it in your final message instead.**

## Mount point (already exists, verified)

`OperantAgent` has an `Option`-typed turn-end bus with **zero subscribers**:

- field: `crates/operant-core/src/agent/mod.rs` (`turn_end_bus`)
- builder: `with_turn_end_bus` at `crates/operant-core/src/agent/builders.rs:280`
- emits: `crates/operant-core/src/agent/run.rs:1271`, `crates/operant-core/src/agent/stream.rs:888,926,1012`
- bus type: `crate::turn_end::TurnEndBus` in `crates/operant-core/src/turn_end.rs`

**Nothing in the CLI ever calls `with_turn_end_bus`.** That is the gap you are closing.
Read `turn_end.rs` first to understand `TurnEndBus`, its event payload, and how a subscriber
is registered.

## Already-existing stores to write through (do NOT create new tables)

- `org::worklog_db::WorklogDb` — the objective worklog row. Read `worklog_db.rs` for its
  append signature and the `WorklogRecord`/`WorklogEntry` shapes.
- `org::decisions_db::DecisionsDb` — subjective entries (`SubjectiveEntry`) and decision
  objects (`OrgDecision`). Read `decisions_db.rs` for both.
- `org::employee_db::EmployeeDb` — resolves `employee_id` → employee so the row is attributable.

These three stores live in different SQLite files. **Never claim `PRAGMA user_version`** —
that counter belongs to the kanban family alone (see `org/schema.rs` module docs). Do not
add a migration that writes it.

## Semantics that are NOT negotiable

1. **Objective vs subjective are different artifacts.** The worklog row is objective (what
   happened). Subjective entries are the employee's reasoning. Do not merge them into one row.
2. **Decision objects only for organizational changes.** A run that changed nothing
   organizational writes no decision object. Do not manufacture one per run.
3. **Barrier failure must be visible, not silent.** If a write fails, the barrier reports it.
   Do not swallow the error and report success.
4. **The barrier completes the run and raises a finding** if the failure happens *after*
   effects have landed. Do not pretend a completed run was prevented.

## Tests you must write

Put them in `crates/operant-core/src/org/write_barrier.rs` under `#[cfg(test)]`, or in a new
`crates/operant-core/tests/org_write_barrier.rs` (you own that new test file if you create it).

Required, each with a real assertion:
- a successful run produces exactly one worklog row
- subjective entries are recorded separately from the worklog row
- a write failure is surfaced, not swallowed
- no decision object is written when nothing organizational changed

**Mutation-prove your tests.** After they pass, break your implementation on purpose
(comment out the write, or ignore the error), confirm the tests FAIL, then restore. Report
the counts in your final message. A test that cannot fail proves nothing.

## Verification (run these yourself)

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --lib -- write_barrier
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.
`operant-core` lib must compile with **zero** warnings and zero clippy findings on your files.

## Hard constraints

- **No `.unwrap()` or `.expect()` in production code.** The clippy gate denies both. Use
  `match`/`if let`/`?`. In `#[cfg(test)]` code they are fine.
- If you genuinely need an `#[expect(clippy::unwrap_used, reason = "...")]`, that is allowed
  only for poison-recovery with a real reason.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files you created/changed, the public API you exposed (signatures the coordinator
needs to wire it), test counts, mutation results (N of M tests failed when broken), and the
exact verification command output tail. If you could not complete something, say so plainly
rather than describing partial work as done.