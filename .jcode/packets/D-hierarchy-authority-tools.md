# Packet D — Hierarchy fields + authority-filtered tools (Step D)

## Goal

Two parts, and the second is the load-bearing one:

1. Add **hierarchy** (`reports_to`, peers) to the employee registry.
2. Make authority actually constrain the **tool registry**, not merely board mutations.

Part 2 is an explicit owner requirement and the single most load-bearing remaining correctness
property in the system.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/org/hierarchy.rs` (you create this)
- `crates/operant-core/src/org/employee.rs` (you may ADD fields here — read carefully below)
- `crates/operant-core/src/tools.rs` (you may ADD here — read carefully below)

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `session/mod.rs`, `turn_end.rs`,
`employee_db.rs`, `authority.rs`, `worklog_db.rs`, `decisions_db.rs`, or any file under
`crates/operant-cli/`. Other agents own those right now and are editing them concurrently.
The coordinator registers your module and wires the CLI.

**If you believe you need to change a file you do not own, do not change it — report it in
your final message instead.**

## What already exists (read it before designing)

- `crates/operant-core/src/org/authority.rs`:
  - `enum AuthorityScope` — the scope lattice
  - `fn resolve_scope(is_dept_head: bool, is_org_lead: bool) -> AuthorityScope`
  - `struct ScopeCheck` — the verdict type
  - `fn can_post_to(...)`, `fn can_accept_decision(actor, decision_scope) -> ScopeCheck`
  - `struct Grant`, `struct GrantDb` — cross-department grants
  - `struct Designation` — the derived designation string
- `crates/operant-core/src/org/employee.rs`: `struct Employee` has
  `employee_id`, `name`, `role`, `department: Option<String>`, `skills: Vec<String>`,
  `agent_type: Option<AgentType>`. **`reports_to` does not exist yet.**
- `crates/operant-core/src/tools.rs`: `struct ToolRegistry` at line ~352, with
  `register`, `register_dyn`, `unregister_tool`, `unregister_tool_if`, `disable_tool`,
  `enable_tool`, `disable_toolset`, `enable_toolset`, `set_disabled_tools`,
  `set_disabled_toolsets`. **There is no authority filter.**

`GrantDb` and the scope lattice already exist. What is missing is (a) the hierarchy they read
from, and (b) the enforcement point in tool resolution.

## Owner requirements — these are explicit, do not reinterpret them

1. **Hierarchy is the PRIMARY authority grant within a department.** A department head
   outranks a peer by position, with no explicit grant needed.
2. **Cross-department actions require explicit, attributable, expiring grants.** Never infer
   cross-department authority from a grant that does not exist.
3. **Authority must constrain TOOL AVAILABILITY, not only board writes.** This is the point of
   the packet. §2.4: a non-crossing scope is offered **its own department's tool surface with
   no grant recorded**; a crossing scope requires an explicit, attributable, expiring grant.
4. **Grants to vacant or invalid seats must be REFUSED, never stored as inert fabricated
   authority.** If the grantee is not a valid, filled seat, reject the grant. Do not persist it.
5. A non-crossing scope must record **no grant** — do not write a grant row for in-department
   authority. That would misrepresent where the authority came from.

## Part 1 — Hierarchy fields

- Add `reports_to` to `Employee` (an `Option<String>` employee_id, `None` = top of the
  department). **`None` is the honest value and must stay visible** — do not default it to a
  guess.
- Be aware: `Employee` is constructed in several places, including tests and the backfill in
  `employee_db.rs`. **You do not own `employee_db.rs`.** Add the field so that existing struct
  literals still compile *if* the struct uses `..Default::default()`; if it does not, prefer
  implementing `Default` for `Employee` or otherwise minimising breakage, and **report any
  construction site you cannot fix** rather than editing a file you do not own.
- Provide hierarchy queries in your new `hierarchy.rs`: who reports to whom, a department's
  reporting tree, peers of an employee, and detection of cycles (a cycle in `reports_to` must
  be detected and reported, never followed forever).

## Part 2 — Authority-filtered tools (the load-bearing part)

Add an authority layer to tool resolution in `tools.rs`. Design it so that:

1. Given an actor's `AuthorityScope` and their department, the registry computes the set of
   tools that actor may use.
2. A tool restricted to a department is offered to an actor **in** that department with **no
   grant recorded**.
3. A tool restricted to another department is offered **only** with an explicit, attributable,
   expiring grant. Check expiry.
4. **Filtered-out tools must not be visible to the model.** Hiding them from the tool list is
   the requirement; blocking execution alone is not sufficient, because a tool the model can
   see but not call produces a turn of confusion and wasted retries.

Be careful about how you implement this: `ToolRegistry` is heavily used by tests across the
workspace. Prefer an **additive** API (a new filter type + an explicit
`tools_for(actor)` query) over changing the behaviour of existing methods, so the 2199
currently-passing tests keep passing. **If you change the semantics of an existing method,
say so loudly in your final message and list every test that changed meaning.**

## Tests you must write

- `crates/operant-core/tests/org_authority_tools.rs` (you own this new test file)

Required, each with a real assertion:
- a non-crossing scope is offered its own department's tools with **no grant row written**
- a crossing scope is refused without an explicit grant
- an explicit grant admits the crossing tool
- an **expired** grant does not admit the tool
- a grant to a **vacant/invalid seat is refused and not persisted**
- a filtered-out tool is **absent from the advertised tool list**, not merely blocked at
  execution
- a `reports_to` cycle is detected rather than followed

**Mutation-prove your tests.** After they pass, break your implementation on purpose, confirm
the tests FAIL, then restore. Report the counts. A test that cannot fail proves nothing.

## Verification (run these yourself)

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_authority_tools
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_wave2_shared_file
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_identity_gate
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests**, nor
`org_wave2_shared_file` (9) or `org_identity_gate` (8). Run them and report the counts.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` or `.expect()` in production code.** The clippy gate denies both.
- **Never claim `PRAGMA user_version`** — that counter belongs to the kanban family alone.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, the public API the coordinator needs to wire the authority
layer and hierarchy into the CLI, every `Employee` construction site you could not fix
(file:line and why), whether you changed the semantics of any existing method and which tests
that affected, test counts, mutation results (N of M failed when broken), the confirmation
that the core lib and both org suites still pass, and the verification output tail. If you
could not complete something, say so plainly rather than describing partial work as done.