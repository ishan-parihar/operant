# Holon integration (proposed, 2026-10-14)

**Holon** is the Chair's agentic operating system. It is a stdlib-Python kernel, separate from
operant, at `github.com/ishan-parihar/holon`. It holds the organization:
- departments, persistent employees and their authority;
- one work graph (plan, approve, execute, verify), with rubrics, gold standards and versioned
  playbooks;
- objectives, memory promotion, and healing.

It supersedes the legacy Hermes organism that `crates/operant-core/src/org/` ports.

The audit and phased plan are in Holon's repository: `docs/design/2026-10-14-operant-integration.md`.
Read it before changing operant's org layer, its cron cast, or anything Holon would drive.

## The proposal in brief

- **Holon runs as a sidecar kernel; operant is its primary body.** Holon drives operant runs
  through an adapter (`operant -c <posture> run --no-stream -s <identity> -q <brief>`), with
  Holon's MCP server mounted (`holon mcp`, authenticated by `HOLON_RUN_TOKEN`).
- **Native seams in operant**, behind a `holon` feature, active only inside a Holon run
  (`HOLON_RUN_TOKEN` and `HOLON_RUN_DIR` set):
  - enforce the approved plan per tool call, through `org/seat_policy.rs::decide`;
  - turn skill and memory nudges and `skill_manage` writes into result annotations instead of
    live edits;
  - record the trajectory into the run directory;
  - report usage.
- **The Telegram gateway becomes the Chair's channel** for Holon: approvals, notes and deliveries.
- **One organization.** The cast seats map onto Holon departments and kernel mechanisms; the cast
  crons are replaced by Holon routines. The org stores (`operant_kanban.db`,
  `operant_decisions.db`) are kept as an archive and are never deleted.

Status: **awaiting the Chair's decision.** Nothing here changes operant's behavior until a phase is
approved. Each phase updates this file in the same iteration.
