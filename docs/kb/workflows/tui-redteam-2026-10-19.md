# Workflow: tui-redteam-2026-10-19

`genome wf add tui-redteam-2026-10-19 "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| tui-redteam-2026-10-19-001 | open | TUI-2 (H): /resume loads a transcript but the agent keeps the old session id - retarget on load (BUGS.md) | — | — |
| tui-redteam-2026-10-19-002 | open | TUI-3 (H): creating an agent overwrites an existing file with the same slug - refuse on collision (BUGS.md) | — | — |
| tui-redteam-2026-10-19-003 | open | TUI-4 (H): multi-line agent description truncated on save - round-trip it (BUGS.md) | — | — |
| tui-redteam-2026-10-19-004 | open | TUI-6 (H): shell-out spawn error skips the terminal restore - restore on every exit path (BUGS.md) | — | — |
| tui-redteam-2026-10-19-005 | open | TUI-1 (M): approval prompt painted under later overlays, keys go to the open overlay - owner decision in Review (BUGS.md) | — | — |
| tui-redteam-2026-10-19-006 | open | TUI-5 (M): /rewind y/n confirmation stays armed after other keys - cancel on any other key (BUGS.md) | — | — |
| tui-redteam-2026-10-19-007 | open | TUI-7 (M): shell-out says Returned to operant on non-zero exit - show the status (BUGS.md) | — | — |
| tui-redteam-2026-10-19-008 | open | TUI-8 (M): /rename ignores its argument and is not saved - write the title (BUGS.md) | — | — |
| tui-redteam-2026-10-19-009 | open | TUI-9 (M): delegations stay running after /new or /rewind - clear the status entries (BUGS.md) | — | — |
| tui-redteam-2026-10-19-010 | open | TUI-10 (M): shell-out keeps mouse capture on for the child - toggle it around the child (BUGS.md) | — | — |
| tui-redteam-2026-10-19-011 | open | TUI-11 (M): blocking SQLite and git diff on the event loop - owner decision in Review (BUGS.md) | — | — |
| tui-redteam-2026-10-19-012 | open | TUI-12 (M): five command descriptions do not match their handlers - fix the descriptions (BUGS.md) | — | — |
