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
| tui-bugfix-campaign-009 | done | phase-2 fresh sweep: corpus verify + unit tests + ledger triage, fix all found | claimed-by `agent-omarchy` since 2026-11-05T06:05 | [2026-11-05] origin/main iters 1746-1748 (ledger RT-212/RT-213/RT-214) |
| tui-bugfix-campaign-010 | done | B10-B13: the TUI gate's own defects — ambient colour depth, an unlocked palette read, a no-op protocol override, a source guard with no `#[test]` | — | [2026-11-05] origin/main iter-1746 |
| tui-bugfix-campaign-011 | done | B14-B15: the TUI crate's lint gate — a duplicated `#[test]` that registered a test twice, and the crate's only production `unwrap` | — | [2026-11-05] origin/main iter-1747 |
| tui-bugfix-campaign-012 | done | B16: the TUI's command sweep called a simulator that could not start 98 broken commands (no hermetic HOME, no error class) | — | [2026-11-05] origin/main iter-1748 |
| tui-bugfix-campaign-013 | done | phase-3: the corpus's 8 UNGATED scenarios (the `<wait:ms>` unblock the scroll-pageup notes name), the live path, and the management panels' live data | claimed-by `agent-omarchy` since 2026-11-05T09:30 | [2026-11-05] origin/main iter-1751 (the wait token re-gated one scenario; the other 7 have wall-clock or machine-path causes, measured), iter-1753 (the live path verified: every live frame matches the corpus golden) |
| tui-bugfix-campaign-014 | done | B18: the scroll-back counters were painted OVER the transcript they were counting, so a reader in history saw corrupted words; the peer's `edit-row-diff-expands` scenario got its proven golden | claimed-by `agent-omarchy` since 2026-11-05T10:06 | [2026-11-05] origin/main iter-1754 |
| tui-bugfix-campaign-015 | done | B19-B20: the harness had no idle-frame token (so the scroll ladder's third rung could not be pinned) and the registry described a DELETED MCP-approval dialog as a live product gap | — | [2026-11-05] origin/main iter-1751 |
| tui-bugfix-campaign-016 | done | B21: AGENTS.md claimed a TUI workspace view and a Behavior panel that do not exist, and its module count was stale | — | [2026-11-05] origin/main iter-1752 |
| tui-bugfix-campaign-017 | done | B22: the live-verification tool lost its determinism controls on a failed mktemp, checked its wait where it could never succeed, and the doc named a help section that does not exist | — | [2026-11-05] origin/main iter-1753 |
| tui-bugfix-campaign-018 | done | B23: the keybindings catalogue advertised Ctrl+Q as "Quit application" and nothing dispatched it (the exit-confirmation state machine has two halves, and only one named the chord) | — | [2026-11-05] origin/main iter-1758 |
| tui-bugfix-campaign-019 | done | phase-5: the highlight test's cross-test frame race — the same palette resetter, reached through a second path | — | [2026-11-05] origin/main iter-1761 |
| tui-bugfix-campaign-020 | done | B24: the settings screen dropped the operator's edited "File injection max size" (and its own guard never saw the row), the keybindings guard could not see a MODIFIER, and the accent test's palette race was re-broken by an unlocked App::new | — | [2026-11-05] origin/main iter-1760 |
| tui-bugfix-campaign-018 | done | B23: the per-row diff expansion scenario had no committed golden, so the corpus verify failed on every run | claimed-by `agent-omarchy` since 2026-11-05T13:13 | [2026-11-05] origin/main iter-1758 |
