---
ID: "AD-002"
Title: "The TUI org view reads the Holon read API in-process, not by shelling out"
Status: "Active"
Date: "2026-10-19"
Description: "The TUI org view reads the Holon read API in-process, not by shelling out"
---

# AD-002: The TUI org view reads the Holon read API in-process, not by shelling out

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The 2026-10-19 audit found that no TUI panel reads Holon. `/org` runs `operant org holon <action>` as a subprocess and suspends the alternate screen. `/agents` and `/tasks` read only the local agent definitions. The read API (`operant_os::api::HolonApi`) already serves the roster, board, approvals, messages and triggers in-process: it opens `work.db` with `mode=ro` and refuses an unknown `api_version`. `operant org holon` already opens it in-process (`crates/operant-cli/src/cmd_org.rs`, `holon_api`).

## Decision

The TUI's org views read `HolonApi` in-process and read-only. They do not shell out to `operant org holon`. This decision adds no TUI write path: `supervise`, `tick`, `archive` and `notify` stay outside the TUI org views.

## Consequences

- Org panels refresh from the read API without a subprocess and keep the alternate screen.
- A missing `work.db` or an unknown `api_version` shows the refusal state in the panel. Org mode stays off, as the doctor line already reports.
- A panel cannot write: the connection is `mode=ro` (invariant 1).
- `/org` keeps its shell-out behavior until the org tab lands. This record does not change `/org`.

## Orthogonality

BOUNDARY section 3.2 states the broker read path, but no record decides the TUI transport: shell-out to operant org holon, or in-process HolonApi. (recon R1019053854: session tags body run holon roster)

## Revisions

- 2026-10-19: created
