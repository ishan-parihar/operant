---
ID: "AD-014"
Title: "A shell-out restores the terminal before its result is reported, and releases mouse capture for the child"
Status: "Active"
Date: "2026-10-19"
Description: "A shell-out restores the terminal before it reports the child's exit, reports a failed or signalled exit as such, and frees the mouse for the child"
---

# AD-014: A shell-out restores the terminal before its result is reported, and releases mouse capture for the child

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The shell-out (`run_suspended_shell_command`) had three faults. A spawn error skipped the terminal restore (TUI-6). A non-zero exit or a signal printed "Returned to operant." (TUI-7). Mouse capture stayed on for the child, so the child's own mouse handling never saw a click (TUI-10).

## Decision

1. The child runs inside `run_child_then_restore`, which always calls the restore before the child's result is turned into an error. The restore is `restore_tui_terminal`: raw mode, the alternate screen, mouse capture if it was on, and a resize.
2. `shell_return_message` reports a clean exit, a non-zero exit code, or a signal, in that order.
3. Mouse capture is turned off before the child starts when `mouse_capture_active` is set, and turned back on by the restore.

## Consequences

- TUI-10 has no runtime test. The toggles are crossterm writes to a real terminal, which the test harness does not have. The change was checked by reading the call order, and by `cargo check`. A live run on a terminal is the only way to confirm it.
- A child that takes the mouse while the TUI is suspended is no longer blocked by operant's capture.

## Orthogonality

The restore order is the same as before for the success path. The mouse-capture state is the `/mouse` state changed in TUI-13 (`commands.rs`).

## Revisions

- 2026-10-19: created
