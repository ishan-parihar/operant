# Workflow: tui-redteam-2026-10-19

`genome wf add tui-redteam-2026-10-19 "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| tui-redteam-2026-10-19-001 | done | TUI-2 (H): /resume loads a transcript but the agent keeps the old session id - retarget on load (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: BUGS.md records TUI-2 fixed iter-966 |
| tui-redteam-2026-10-19-002 | done | TUI-3 (H): creating an agent overwrites an existing file with the same slug - refuse on collision (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-003 | done | TUI-4 (H): multi-line agent description truncated on save - round-trip it (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-004 | done | TUI-6 (H): shell-out spawn error skips the terminal restore - restore on every exit path (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-005 | done | TUI-1 (M): approval prompt painted under later overlays, keys go to the open overlay - owner decision in Review (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-006 | done | TUI-5 (M): /rewind y/n confirmation stays armed after other keys - cancel on any other key (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-007 | done | TUI-7 (M): shell-out says Returned to operant on non-zero exit - show the status (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-008 | done | TUI-8 (M): /rename ignores its argument and is not saved - write the title (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-009 | done | TUI-9 (M): delegations stay running after /new or /rewind - clear the status entries (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-010 | done | TUI-10 (M): shell-out keeps mouse capture on for the child - toggle it around the child (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-011 | done | TUI-11 (M): blocking SQLite and git diff on the event loop - owner decision in Review (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
| tui-redteam-2026-10-19-012 | done | TUI-12 (M): five command descriptions do not match their handlers - fix the descriptions (docs/audit/2026-06-19-BUGS.md) | — | [2026-10-20] docs/audit/2026-06-19-BUGS.md — reconciled iter-1053: the BUGS.md row records its fix |
