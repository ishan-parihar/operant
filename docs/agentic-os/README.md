# Operant as an agentic OS, with Holon

Status: **executing** (owner order, 2026-10-14). Phases A–H are landed as code, but the
integration does not yet engage in a real run. **Read `AUDIT-2026-10-15.md` first:** it has what
works, what does not, and the next work packages (W1–W8). With `[holon] enabled = false` (every
box today), operant behaves exactly as before.

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
4. **`AUDIT-2026-10-15.md`**: the phases as built against this plan. It has findings O-01 …
   O-32, the invariants' real state, and work packages W1–W8: the next iterations.
5. **`AUDIT-2026-10-16-capability-layer.md`**: the open work list for how the organization's
   declared capabilities reach a body, and what operant injects into a Holon run on its own
   (R-02 … R-20). It gives each item's acceptance, its order relative to the kernel (the
   read-API v4 trap), and the verification method. It extends `REMAINING-GAPS.md`'s G-list.
6. In Holon's repository:
   - `docs/design/2026-10-16-capability-layer-audit.md` (public) is the kernel's half of that
     work list, with the same R-numbers;
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
