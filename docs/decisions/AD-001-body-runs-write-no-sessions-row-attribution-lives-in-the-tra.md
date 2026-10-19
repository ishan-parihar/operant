---
ID: "AD-001"
Title: "Body runs write no sessions row; attribution lives in the trajectory"
Status: "Active"
Date: "2026-10-19"
Description: "Body runs write no sessions row; attribution lives in the trajectory"
---

# AD-001: Body runs write no sessions row; attribution lives in the trajectory

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

A Holon body run (an `operant` process the Holon kernel dispatches) produces a conversation. The open question was whether that run also writes a row to `sessions`, the table the TUI session browser and `operant sessions` read. The live body home (`~/holon/adapters/operant/home`) has no `database.db`: its history is the `trajectories/` files, and the kernel's run row carries the run id, the employee and the result. The 2026-10-19 audit found that the session browser shows no employee, and that the `session_tags` store has no live writer.

## Decision

A Holon body run writes no `sessions` row. The attribution that session history needs travels in the operant-side trajectory: a Holon run sets `holon_run_id` (from `HOLON_RUN_ID`) and `holon_employee` in `Trajectory.metadata`, which lands in the operant home's `trajectories/` file. The kernel's `trajectory.json` keeps the kernel's shape (`holon_trajectory_json` emits only `steps`), because the kernel already holds the run row.

## Consequences

- The session browser and `operant sessions` list interactive and gateway sessions only. Body history is reached through the org views (AD-002) and the trajectory metadata.
- Filtering body history by employee is a query on `holon_employee` in trajectory metadata. The session-tag filter work keys on that field, not on `session_tags` rows.
- Trajectories written before this change carry `metadata: {}` and stay unattributed. Nothing backfills them.
- A later need for body runs to appear in the session browser is a new record that supersedes this one.

## Orthogonality

No record decides whether a Holon body writes a sessions row. The agentic-os README states the boundary, not this storage choice. (recon R1019053854: session tags body run holon roster)

## Revisions

- 2026-10-19: created
