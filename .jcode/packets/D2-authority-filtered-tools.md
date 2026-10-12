# Packet D2 — Authority-filtered tool availability (Step D, part 2)

## Goal

Make authority constrain **tool availability**, not merely board writes. This is an explicit
owner requirement and the single most load-bearing remaining correctness property remaining
in the system.

§2.4: a non-crossing scope is offered **its own department's tool surface with no grant
recorded**; a crossing scope requires an **explicit, attributable, expiring grant**.

This packet is deliberately split from the hierarchy work (Packet D1 owns `employee.rs` and
`org/hierarchy.rs`). You do not need `reports_to` to do this job — the authority lattice already
exists. Do not wait for D1.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/tools.rs` (you may ADD here)
- `crates/operant-core/tests/org_authority_tools.rs` (you own this new test file)

**Do NOT touch:** `org/mod.rs`, `cmd_org.rs`, `session/mod.rs`, `turn_end.rs`,
`employee.rs`, `employee_db.rs`, `authority.rs`, `hierarchy.rs`, `worklog_db.rs`,
`decisions_db.rs`, or any file under `crates/operant-cli/`. Other agents own those right now
and are editing them concurrently.

**If you believe you need to change a file you do not own, do not change it — report it in
your final message instead.**

## What already exists (read it before designing)

- `crates/operant-core/src/org/authority.rs`:
  - `enum AuthorityScope` — the scope lattice
  - `fn resolve_scope(is_dept_head: bool, is_org_lead: bool) -> AuthorityScope`
  - `struct ScopeCheck` — the verdict type
  - `fn can_post_to(...)`, `fn can_accept_decision(actor, decision_scope) -> ScopeCheck`
  - `struct Grant`, `struct GrantDb` — cross-department grants (attributable, expiring)
  - `struct Designation` — the derived designation string
- `crates/operant-core/src/org/employee.rs`: `Employee` has `department: Option<String>` and
  `employee_id`. That is all you need.
- `crates/operant-core/src/tools.rs`: `struct ToolRegistry` at ~line 352, with `register`,
  `register_dyn`, `unregister_tool`, `unregister_tool_if`, `disable_tool`, `enable_tool`,
  `disable_toolset`, `enable_toolset`, `set_disabled_tools`, `set_disabled_toolsets`.
  **There is no authority filter.** Find where the tool list is **advertised to the model**
  (the schema/`list` path) as well as where tools are **executed** — you need both.

## Owner requirements — explicit, do not reinterpret them

1. **Hierarchy is the PRIMARY authority grant within a department.** In-department authority
   needs no explicit grant.
2. **Cross-department actions require explicit, attributable, expiring grants.** Never infer
   cross-department authority from a grant that does not exist.
3. **Authority must constrain TOOL AVAILABILITY, not only board writes.** A tool the actor may
   not use must not be **offered** to them.
4. **A filtered-out tool must be ABSENT from the advertised tool list**, not merely blocked at
   execution time. Blocking execution alone is insufficient: a tool the model can see but
   cannot call produces a turn of confusion and wasted retries. This is the requirement most
   likely to be implemented wrong — make sure your test proves absence from the advertised
   list, not just a failed call.
5. **A non-crossing scope records NO grant.** Do not write a grant row for in-department
   authority — that would misrepresent where the authority came from.
6. **Grants to vacant or invalid seats must be REFUSED, never stored as inert fabricated
   authority.**

## Implementation guidance

`ToolRegistry` is used by many tests across the workspace. **Prefer an additive API** — a new
filter/actor type plus an explicit `tools_for(actor)` / `advertise_for(actor)` query — over
changing the behaviour of existing methods, so the **2199 currently-passing** `operant-core`
lib tests keep passing unchanged.

**If you change the semantics of an existing method, say so loudly in your final message and
list every test whose meaning changed.** Silent semantic drift in a shared registry is the
worst outcome available here.

Handle the grant check with an explicit expiry comparison against the current time, and make
the "no grant needed" and "grant required" paths clearly distinct in the code — a reader must
be able to tell which rule fired.

## Tests you must write

`crates/operant-core/tests/org_authority_tools.rs` (you own this new test file).

Required, each with a real assertion:
- a non-crossing scope is offered its own department's tools, and **no grant row is written**
- a crossing scope is refused without an explicit grant
- an explicit grant admits the crossing tool
- an **expired** grant does **not** admit the tool
- a grant to a **vacant/invalid seat is refused and not persisted**
- a filtered-out tool is **absent from the advertised tool list**, not merely blocked at
  execution

Note: Packet D1 owns the `reports_to` cycle-detection tests in the same area but a **different
file** (`org/hierarchy.rs` inline tests). Do not duplicate cycle tests here — yours go in
`tests/org_authority_tools.rs` and focus on tool availability.

**Mutation-prove your tests.** After they pass, break your implementation on purpose (e.g.
return the unfiltered list, or skip the expiry check), confirm the tests FAIL, then restore.
Report the counts. A test that cannot fail proves nothing.

## Verification (run these yourself)

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test org_authority_tools
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests.** Run the full lib suite and
report the count. Other agents are editing sibling files concurrently, so if you see a compile
error in a file you do **not** own (`write_barrier.rs`, `autocompact.rs`, `hierarchy.rs`,
`employee.rs`), **ignore it and report it** — do not attempt to fix it, and do not treat it as
a blocker for your own work.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` or `.expect()` in production code.** The clippy gate denies both.
- **Never claim `PRAGMA user_version`** — that counter belongs to the kanban family alone.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, the public API the coordinator needs to wire authority-filtered
tool advertising, whether you changed the semantics of any existing method and which tests that
affected, test counts, mutation results (N of M failed when broken), the full lib suite count
confirming no regression, any compile errors you saw in files you do not own, and the
verification output tail. If you could not complete something, say so plainly rather than
describing partial work as done.