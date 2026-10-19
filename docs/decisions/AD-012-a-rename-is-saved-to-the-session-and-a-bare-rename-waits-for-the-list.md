---
ID: "AD-012"
Title: "A rename is saved to the session, and a bare /rename waits for the list"
Status: "Active"
Date: "2026-10-19"
Description: "`/rename` persists the confirmed name, and a bare `/rename` opens the rename once the session list has loaded"
---

# AD-012: A rename is saved to the session, and a bare /rename waits for the list

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`/rename` opened the browser on an empty list, started the rename before any row existed, and never saved the name. The browser's Enter path changed only the in-memory title, so the name was lost on the next start. `/rename <title>` dropped its argument. The red-team row TUI-8 asked for both to persist.

## Decision

1. Confirming a rename in the browser saves the title for the row's session through `Database::update_session_title`, on a blocking thread with its own connection (`save_session_title` in `adapter_types/history.rs`). The same call is used for `/rename <title>`, which names the open session. The adapter saves that one in `run_pending_agent_actions`, because the App does not hold the open session's id.
2. A bare `/rename` sets `rename_on_list_load`. `drain_session_list` starts the rename on the first row when the list arrives, and clears the flag.

## Consequences

- A save that fails is logged with `tracing::warn!`, not shown in the transcript. The name still shows for the rest of the run. Surfacing it in the transcript is a possible follow-up, not a current gap.
- `session_title` is set for the rename on the spot, so the status bar changes before the save completes.
- Tests: `a_bare_rename_starts_on_the_first_row_once_the_list_arrives` (`crates/operant-cli/src/tui/app/tests.rs`). The persistence call is the existing `update_session_title`, which has its own test in `database.rs`. No unit test reaches the adapter's save, because it needs the operant home directory.

## Orthogonality

The `/resume` pick and the session list's row ids are unchanged. AD-005 covers plugins and is not affected.

## Revisions

- 2026-10-19: created
