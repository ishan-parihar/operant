# Workflow: tui-bugfix-campaign

`genome wf add tui-bugfix-campaign "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| tui-bugfix-campaign-001 | done | corpus repair: terminal-scroll-cls scenario + mass golden re-record + missing click baselines | claimed-by `agent-omarchy` since 2026-11-05T02:01 | [2026-11-05] docs/kb/WORKLOG.md |
| tui-bugfix-campaign-002 | done | B1: Error arm cancels pending tool blocks | claimed-by `agent-omarchy` since 2026-11-05T02:01 | [2026-11-05] origin/main iter-1728 |
| tui-bugfix-campaign-003 | done | B2+B3: @ grammar terminator + parse-before-list | claimed-by `agent-omarchy` since 2026-11-05T02:01 | [2026-11-05] origin/main iter-1729 |
| tui-bugfix-campaign-004 | done | B4+B5: native replay at rewind + failed-prompt restore | claimed-by `agent-omarchy` since 2026-11-05T02:01 | [2026-11-05] origin/main iter-1730 (landed via c3cbb381, noted in iter-1733) |
| tui-bugfix-campaign-005 | done | B6: hub grouping arithmetic | — | [2026-11-05] origin/main iter-1733 |
| tui-bugfix-campaign-006 | done | B7: /split fallthrough | — | [2026-11-05] origin/main iter-1734 |
| tui-bugfix-campaign-007 | done | B8: Done.reason consumer | — | [2026-11-05] origin/main iter-1735 |
| tui-bugfix-campaign-008 | done | B9: Closed arm flush | — | [2026-11-05] origin/main iter-1736 |
| tui-bugfix-campaign-009 | in-progress | phase-2 fresh sweep: corpus verify + unit tests + ledger triage, fix all found | claimed-by `agent-omarchy` since 2026-11-05T06:05 | — |
| tui-bugfix-campaign-010 | done | B10-B13: the TUI gate's own defects — ambient colour depth, an unlocked palette read, a no-op protocol override, a source guard with no `#[test]` | — | [2026-11-05] origin/main iter-1746 |
