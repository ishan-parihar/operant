# Holon integration (proposed, 2026-10-14)

**Holon** is the owner's agentic operating system: a stdlib-Python kernel at
`github.com/ishan-parihar/holon` that holds the organization. It supersedes the legacy Hermes
organism that `crates/operant-core/src/org/` ports. Operant becomes the organization's runtime:
the employees' bodies, enforcement, metering, the clock, events and the owner's channel.

The plan is in **`docs/agentic-os/`**:
- `README.md`: index and reading order;
- `BOUNDARY.md`: what operant owns natively, in Rust; Rust versus Python; the two modes;
- `ORG-LAYER-DISPOSITION.md`: every org module marked keep, rewire or retire;
- `IMPLEMENTATION-PLAN.md`: phases A–I across both repositories.

Holon's side is in its `docs/design/2026-10-14-holon-operant-boundary.md` (canonical rule,
interfaces) and `2026-10-14-operant-integration.md` (audit, decision).

Read them before changing:
- the org layer;
- the cron cast;
- the kanban;
- the curator and skill nudges;
- the kernel sidecar;
- anything Holon would drive.

Status: **awaiting the owner's decision.** Nothing changes operant's behavior until a phase is
approved.
