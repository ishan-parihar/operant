---
ID: "AD-015"
Title: "The Ctrl+B fork tree is read off the event loop, and a database error is shown"
Status: "Active"
Date: "2026-10-19"
Description: "The branch browser's parent and fork rows are read on a blocking thread, and a failed read is reported in the transcript instead of opening an empty browser"
---

# AD-015: The Ctrl+B fork tree is read off the event loop, and a database error is shown

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`run_pending_agent_actions` read `session_parent` and `child_sessions` synchronously on the event loop. Each error was discarded by `if let Ok`, so a database error opened an empty branch browser that looked like a session with no forks (TUI-11, branch half).

## Decision

1. `history::fork_tree(session_id)` runs the two reads on a `spawn_blocking` thread with its own connection (`Database::init(default_db_path())`), the same pattern as the session list. The result returns on `branch_list_rx`.
2. `drain_branch_list` opens the browser when the result arrives. An error is pushed to the transcript as an error card and the browser stays closed.

## Consequences

- The fork tree reads from a second connection to the same file, so it sees committed rows only. A fork created in the same instant may not appear until Ctrl+B is pressed again.
- The git-diff load (`DiffState::reload_files`, `diff_viewer/parse.rs`) still runs on the event loop when the diff viewer opens. That half of TUI-11 is open and is not covered by this record.
- Tests: `a_failed_fork_tree_read_is_reported_not_shown_as_an_empty_tree` and `a_loaded_fork_tree_opens_the_branch_browser` (`crates/operant-cli/src/tui/app/tests.rs`). Mutation check: dropping the error text from the transcript fails the first test.

## Orthogonality

AD-012 covers the session list's save path. The session list already used a blocking thread, and this record follows it.

## Revisions

- 2026-10-19: created
