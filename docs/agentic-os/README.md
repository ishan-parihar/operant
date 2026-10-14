# Operant as an agentic OS, with Holon

Status: **proposed (2026-10-14)**, awaiting the owner. Until a phase is approved, operant behaves
exactly as it does today.

Operant and **Holon** will ship as one package that runs the owner's organization.
- **Holon** is a stdlib-Python kernel (`github.com/ishan-parihar/holon`) that holds the
  organization: departments, employees, the work graph, approvals, verification, playbooks,
  objectives and memory promotion.
- **Operant** is the runtime the organization lives in: the bodies of its employees, the security
  boundary, enforcement, metering, the clock, events and the owner's channel.

There is one organization, one owner per capability and one writer per database.

## Reading order

1. **`BOUNDARY.md`**: what operant owns natively, in Rust; the rules that decide Rust versus
   Python; operant's two modes (personal agent, organization runtime).
2. **`ORG-LAYER-DISPOSITION.md`**: every module of `operant-core/src/org/` and its satellites
   (kanban, cron, curator, kernel-sidecar, `operant-runtime`), each marked keep, rewire or
   retire, with what replaces it and the test that proves it.
3. **`IMPLEMENTATION-PLAN.md`**: phases A–I covering both repositories: Rust work packages,
   Holon's matching Python packages, config, acceptance tests, and the order.
4. In Holon's repository:
   - `docs/design/2026-10-14-holon-operant-boundary.md` is the canonical rule, the ownership
     table, the measurements and the interfaces: the read API, the run protocol, the contract
     additions;
   - `docs/design/2026-10-14-operant-integration.md` is the audit and the decision to run Holon
     beside operant rather than inside it.

## What this supersedes

The org layer was built to `docs/ORGANISM-ARCHITECTURE.md` and `docs/ORG-AUTHORITY-ARCHITECTURE.md`
as a native port of the legacy Hermes organism. Holon supersedes that organism.
- **Superseded once approved:** the parts of those documents that define a second organization
  (seats, grants, notices, worklog, decisions, synthesis, socialization).
- **Not superseded:** the enforcement parts, which operant keeps. `ORG-LAYER-DISPOSITION.md` says
  which is which, section by section.

## Keeping this current

Each phase updates, in the same iteration:
- these files;
- `operant.example.toml` (new `[holon]` keys);
- `AGENTS.md` (Operant Project Context);
- Holon's `docs/STATUS.md`.

A phase is not done until a fresh agent reading only these files could tell what runs where.
