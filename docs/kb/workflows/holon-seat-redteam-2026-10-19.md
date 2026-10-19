# Workflow: holon-seat-redteam-2026-10-19

`genome wf add holon-seat-redteam-2026-10-19 "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| holon-seat-redteam-2026-10-19-001 | open | OS-1 (H): body reads supervisor environ via /proc/$PPID and chair keys via HOME — isolation decision (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-002 | open | OS-2 (H): act body reaches ~/.config/holon/keys/*.chair as operator uid — isolation decision (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-003 | open | OS-3 (H): enforcement no-op when policy.json missing, no log — decide fail-closed in org mode (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-004 | open | OS-4 (H): holon_skill_invoke allowed in observe posture and writes the ledger — add to OBSERVE_HOLON_DENY (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-005 | open | OS-5 (H): chair notify exits 0 when every send failed; kernel cursor advanced first (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-006 | open | OS-6 (H): Ctrl-C or SIGTERM on supervise leaves the body running — forward signals, kill on exit (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-007 | open | GW-1 (H): budget charged to write-time binding, never rolls — decide attribution rule (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-008 | open | GW-2 (H): cron turns never set memory scope — set it from the seat (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-009 | open | GW-3 (H): unbound turn after /new keeps previous employee's seat — clear it (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-010 | open | GW-4 (H): queued memory writes resolve bank at worker time — carry scope in the job (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-011 | open | OS-7 (M): any non-running status counts as a clean stop - non-zero exit, no self-submit (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-012 | open | OS-8 (M): timed-out native tool leaves its kernel child running - kill_on_drop, reap (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-013 | open | GW-5 (M): /new leaves the transcript and summary - clear them (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-014 | open | GW-6 (M): rebind keeps the chat transcript - owner decision in Review (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-015 | open | GW-7 (M): continuation summary keyed by chat, /resume can inject the wrong roll - key by session (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-016 | open | GW-8 (M): budget gate fails open on a usage-read error - owner decision in Review (BUGS.md) | — | — |
| holon-seat-redteam-2026-10-19-017 | open | GW-9 (M): /session reports premiere for an unbound chat - same defect as G-1 (BUGS.md) | — | — |
