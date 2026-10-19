---
ID: "AD-026"
Title: "The git diff loads on a worker thread, and the viewer shows a reading state"
Status: "Active"
Date: "2026-10-19"
Description: "The git diff for the diff viewer is read on a worker thread and taken on the event-loop tick, so opening /diff does not stall the loop"
---

# AD-026: The git diff loads on a worker thread, and the viewer shows a reading state

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

TUI-11, diff half. `open`, `toggle_diff_type` and the reload ran `git diff` on the event loop (`diff_viewer/state.rs`). AD-015 moved the branch half off the loop.

## Decision

1. `reload_files` starts a `std::thread` that runs `load_git_diff`, and sets `loading`. The result comes back on an `mpsc` channel.
2. `drain_git_diff` takes the result on each tick (`app/mod.rs`). Until it arrives the viewer shows " Reading the git diff…".
3. A new load replaces the receiver, so an older result is dropped. Switching to the turn diff clears the receiver, so a pending git load cannot overwrite the turn list.
4. The receiver sits in `Arc<Mutex<…>>`. The paint path clones the viewer every frame and only reads `loading`, so the copy shares the receiver and never drains it. `Receiver` alone is neither `Clone` nor `Sync`.

Options not taken: `spawn_blocking` (the viewer's methods run in tests with no Tokio runtime, and a thread keeps them testable); a timeout (it still stalls the loop for its length).

## Consequences

- The diff appears one tick after `/diff` opens, instead of in the same frame.
- One thread is started per open. That cost is negligible at a human's pace.
- Test `a_git_diff_is_read_off_the_loop_and_taken_once_it_arrives`; mutant (loading cleared at start) turned it red.

## Orthogonality

AD-015 is the branch half of TUI-11; this record is the diff half.

## Revisions

- 2026-10-19: created

