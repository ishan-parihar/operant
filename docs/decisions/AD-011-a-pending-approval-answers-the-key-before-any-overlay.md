---
ID: "AD-011"
Title: "A pending approval answers the key before any overlay"
Status: "Active"
Date: "2026-10-19"
Description: "An approval the agent is waiting on takes the key before any overlay, so a browser cannot swallow it"
---

# AD-011: A pending approval answers the key before any overlay

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The permission gate in `handle_key_event` ran after the session browser, the agents menu and the permissions picker. An open overlay therefore took the key for an approval the agent was waiting on, and the turn stalled while the prompt was still painted on top. The code comment that accompanied the gate called the order a deliberate debug assertion, not a router, and deferred the reorder. The red-team row TUI-1 asked for a decision.

## Decision

The approval gate runs directly after the bypass-permissions gate, which stays the highest priority, and before global search and every other overlay. An approval the agent is waiting on answers the next key.

## Consequences

- A key pressed while an approval is pending goes to the approval, even when the session browser is open. The test presses Esc: the approval is cleared and the browser stays open.
- The dialog-priority chain (`dialog_priority()`) still describes the other overlays and is unchanged. Only the approval moved.
- Test: `a_pending_approval_takes_the_key_before_an_open_browser` (`crates/operant-cli/src/tui/app/tests.rs`). Mutation check: placing the gate after the browser gate fails it.

## Orthogonality

AD-006 and AD-007 cover the Holon side and do not touch the TUI. The bypass-permissions dialog keeps its position.

## Revisions

- 2026-10-19: created
