# Packet E — Governed assignment boards (Step E)

## Goal

Assignment boards are the **first genuinely governed mutation** in the system: a cross-department
assignment is gated by the authority check from Step D. This is also the first place the authority
lattice is exercised by something other than a test.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/org/assignment.rs` (you create this)

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `kanban/db.rs`, `kanban/mod.rs`,
`tools.rs`, `authority.rs`, `hierarchy.rs`, `employee.rs`, `employee_db.rs`, `session/mod.rs`,
`write_barrier.rs`, or any file under `crates/operant-cli/`. The coordinator registers your
module and wires the CLI.

**If you believe you need to change a file you do not own, do NOT make it — report it in your
final message instead.**

## Prerequisite

Packet D2 (authority-filtered tools) is landing concurrently and provides the authority layer
you gate on: `AuthorityScope`, `ScopeCheck`, `resolve_scope`, `Grant`/`GrantDb` in
`crates/operant-core/src/org/authority.rs`, plus the new authority-filtered tool surface in
`crates/operant-core/src/tools.rs`.

**Read `authority.rs` first and build on what is actually there.** If the authority API you need
is not yet present, build your gate against the existing `can_post_to` / `ScopeCheck` /
`GrantDb` surface rather than inventing a parallel one. Do not wait for D2, and do not
reimplement authority.

## What already exists

- `crates/operant-core/src/kanban/db.rs`: `TaskStatus`, `Task`, `Run`, `Comment`, `Event`,
  `CreateTaskParams`, `KanbanDb` — the existing board substrate. Read it. Reuse it; do not
  build a second task store.
- `crates/operant-core/src/kanban/mod.rs`: `BoardInfo`, `KanbanManager`.
- `crates/operant-core/src/org/department_db.rs`: `Department`, `DepartmentDb`,
  `StaffingFinding` — departments own mandates, protocols, rules, staffing requirements, and
  vacant seats.
- `crates/operant-core/src/org/employee_db.rs`: `EmployeeDb`, `Employee`, `employee_id`.

## Owner requirements — explicit, do not reinterpret them

1. **An assignment is gated by authority, not merely recorded.** A cross-department assignment
   without an explicit, attributable, expiring grant must be **refused**, and refused in a way
   that is visible (a `ScopeCheck`-shaped reason, not a silent no-op).
2. **Grants to vacant or invalid seats must be REFUSED, never stored as inert fabricated
   authority.** An assignment to a seat nobody fills is fabricated authority.
3. **Hierarchy is the primary authority grant within a department.** An in-department
   assignment needs no grant and must record none.
4. **Every mutation requires a non-blank reason.** This is an established invariant across the
   org stores (`department_db`, `decisions_db`, `dm_thread` all enforce it). Follow it exactly —
   read one of those files to see the established error shape before inventing a new one.
5. **CEO/head decisions default to binding while preserving visible dissent.** If an assignment
   is a decision-shaped change, the same rule applies as in `decisions_db`.
6. **Refusals must be reportable.** An operator needs to see *why* an assignment was refused.
   Return a structured refusal, not just `Err("denied")`.

## Semantics that are NOT negotiable

1. **An unauthorized assignment must not partially land.** Refuse before any write, or write
   nothing. A half-created assignment with a rejected grant is worse than no assignment.
2. **A refusal is not a silent failure.** It must be distinguishable from an I/O error.
3. **Never claim `PRAGMA user_version`** — that counter belongs to the kanban family alone. If
   you add tables, reconcile them idempotently the way `org/schema.rs` does.

## Tests you must write

`crates/operant-core/tests/org_assignment_boards.rs` (you own this new test file), plus inline
`#[cfg(test)]` in `assignment.rs` as you see fit.

Required, each with a real assertion:
- an in-department assignment succeeds with **no grant row written**
- a cross-department assignment **without** a grant is refused, with a structured reason
- a cross-department assignment **with** a valid grant succeeds
- an **expired** grant does not permit the assignment
- an assignment to a **vacant/invalid seat is refused and not persisted**
- a **blank reason is refused** (non-blank-reason invariant)
- a refusal leaves **no partial assignment** behind
- a refusal is distinguishable from an I/O error

**Mutation-prove your tests.** Break your implementation deliberately (e.g. drop the expiry
check, or skip the grant lookup), confirm the tests FAIL, restore, report the counts. A test
that cannot fail proves nothing.

## Verification

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_assignment_boards
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_wave2_shared_file
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests.** Run and report the count.

Other agents may be editing sibling files concurrently. If you see a compile error in a file
you do **not** own, **ignore it and report it** — do not attempt to fix it, and do not treat it
as a blocker.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` / `.expect()` in production code.** The clippy gate denies both.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, the public API the coordinator needs to wire boards into the
CLI, how you gated on the authority layer (which existing function you called), test counts,
mutation results (N of M failed when broken), the full lib suite count, any compile errors in
files you do not own, and the verification output tail. If something is incomplete, say so
plainly.