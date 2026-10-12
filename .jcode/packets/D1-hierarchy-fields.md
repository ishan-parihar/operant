# Packet D1 — Hierarchy fields + reporting-tree queries (Step D, part 1)

## Goal

Add **hierarchy** (`reports_to`, peers) to the employee registry, plus the query surface the
authority lattice will read from.

This packet is deliberately split from the authority-filtered-tools work. Packet D2 owns
`tools.rs` and `tests/org_authority_tools.rs`. **Do not touch either.** You do not need to wait
for D2 — your work is independent.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/org/hierarchy.rs` (you create this)
- `crates/operant-core/src/org/employee.rs` (you may ADD fields here — read carefully below)

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `session/mod.rs`, `turn_end.rs`, `tools.rs`,
`employee_db.rs`, `authority.rs`, `worklog_db.rs`, `decisions_db.rs`,
`tests/org_authority_tools.rs`, or any file under `crates/operant-cli/`. Other agents own those
right now and are editing them concurrently.

**If you believe you need to change a file you do not own, do not change it — report it in
your final message instead.**

## What already exists (read it before designing)

- `crates/operant-core/src/org/employee.rs`: `struct Employee` at ~line 91 with
  `employee_id: String`, `name: String`, `role: String`,
  `department: Option<String>`, `skills: Vec<String>`, `agent_type: Option<AgentType>`.
  **`reports_to` does not exist yet.** There is already a
  `missing_required_fields()` predicate — read it, because a new field must not silently
  become a new required field and break every backfilled employee.
- `crates/operant-core/src/org/employee_db.rs`: holds the DDL and the deterministic backfill
  from the live `CronJob`. **You do not own it.**
- `crates/operant-core/src/org/authority.rs`: `resolve_scope(is_dept_head, is_org_lead)`.
  Hierarchy is the PRIMARY authority grant **within** a department, so your reporting tree is
  what makes "is this employee a department head" answerable — but `resolve_scope` itself is
  D2's/authority's concern. Do not edit `authority.rs`.

## Owner requirements — explicit, do not reinterpret them

1. **Hierarchy is the PRIMARY authority grant within a department.** A department head
   outranks a peer by position, with no explicit grant needed. Your reporting tree must make
   head-vs-peer answerable from position alone.
2. **`None` is the honest value and must stay visible.** A missing `reports_to` (top of the
   department, or not yet assigned) is a real state. Do not default it to a guess, and do not
   invent a placeholder manager.
3. **Department membership is an operator-set employee tag. Missing departments must remain
   visible** — an employee with `department: None` must not be silently bucketed into a
   default department by your queries.
4. A **`reports_to` cycle must be detected and reported, never followed forever.** A malformed
   graph must produce a finding, not a hang or a stack overflow.

## Implementation guidance — the `Employee` field is the delicate part

`Employee` is constructed in several places across the workspace, including `employee_db.rs`
(which you do **not** own) and many tests. Adding a field breaks every struct literal that
does not use `..Default::default()`.

Your options, in order of preference:

1. If `Employee` derives or can derive `Default`, add the field and rely on existing literals.
2. If literals exist that you cannot reach, consider whether the field can be added to
   `Employee` at all, or whether it belongs on the hierarchy type instead (e.g.
   `HierarchyEntry { employee_id, reports_to }` in your own `hierarchy.rs`, leaving `Employee`
   untouched).

**Option 2 is acceptable and may be the better engineering choice.** Weigh it honestly rather
than forcing a field into `Employee` and leaving the workspace broken.

Whatever you choose: **report every `Employee` construction site you could not fix**, as
`file:line`, and say whether the crate still compiles. A change that leaves `operant-core`
not compiling is not a change, it is damage.

## Tests you must write

Inline `#[cfg(test)]` in `org/hierarchy.rs`, and/or a new `crates/operant-core/tests/org_hierarchy.rs`
(you own that new test file if you create it).

Required, each with a real assertion:
- a head outranks a peer **by position, with no grant recorded**
- `reports_to == None` is reported as a real state, not silently resolved
- an employee with `department: None` stays visible in findings rather than being dropped
- a **two-node cycle is detected** rather than followed
- a **longer cycle** (3+ nodes) is detected
- a reporting tree resolves correctly for a normal acyclic chain
- peers of an employee are exactly the set expected — not over- or under-inclusive

**Mutation-prove your tests.** After they pass, break your implementation on purpose (e.g.
remove the cycle guard, or treat `None` as top-of-org), confirm the tests FAIL, then restore.
Report the counts. A test that cannot fail proves nothing.

## Verification (run these yourself)

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_identity_gate
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_wave2_shared_file
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests**, nor
`org_identity_gate` (8) or `org_wave2_shared_file` (9). Run them and report the counts.

Other agents are editing sibling files concurrently. If you see a compile error in a file you
do **not** own (`write_barrier.rs`, `autocompact.rs`, `tools.rs`), **ignore it and report it** —
do not attempt to fix it, and do not treat it as a blocker for your own work.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` or `.expect()` in production code.** The clippy gate denies both.
- **Never claim `PRAGMA user_version`** — that counter belongs to the kanban family alone.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, whether you added `reports_to` to `Employee` or kept it on a
hierarchy type (and why), the public API the coordinator needs, every `Employee` construction
site you could not fix as `file:line`, whether `operant-core` compiles after your change, test
counts, mutation results (N of M failed when broken), the confirmation that the core lib and
both org suites still pass, any compile errors you saw in files you do not own, and the
verification output tail. If you could not complete something, say so plainly rather than
describing partial work as done.