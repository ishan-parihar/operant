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
| tui-bugfix-campaign-021 | done | B25: the tips strip advertised three chords that no longer do what the tips say (Ctrl+G, Ctrl+T, the PageUp step) | — | [2026-11-05] origin/main iter-1762 |
| tui-bugfix-campaign-022 | done | B26: the scroll-bookmark notice told the operator to press Ctrl+G to return — Ctrl+G is the external editor | — | [2026-11-05] origin/main iter-1763 |
| tui-bugfix-campaign-023 | done | B27: four of the /help overlay's 111 command descriptions were stale (/branch, /logout, /save, /effort) — the table had no guard, unlike the chord catalogue | — | [2026-11-05] origin/main iter-1766 |
| tui-bugfix-campaign-024 | done | B28: two vendored messages pointed at commands operant does not have — /toolsets and /reset — and the class now has a scan guard | — | [2026-11-05] origin/main iter-1767 |
| tui-bugfix-campaign-025 | done | B29: the catalogue's Prompt-context rows were a stale second copy — /keys printed one chord as two features — and the guard's 'at least one row matches' hole is closed | — | [2026-11-05] origin/main iter-1768 |
| tui-bugfix-campaign-026 | done | B30: the vim modes' catalogue rows were in the wrong context, and Ctrl+C in a vim mode wiped the composer's text | — | [2026-11-06] origin/main iter-1770 |
| tui-bugfix-campaign-027 | done | B31: the visual corpus is green again — the context-window slice landed without re-proving the goldens, so the gate was red at HEAD | — | [2026-11-06] origin/main iter-1771 |
| tui-bugfix-campaign-028 | done | B32: a count argument was silently turned into a different count, and /copy 0 blamed an empty transcript | — | [2026-11-06] origin/main iter-1772 |
| tui-bugfix-campaign-029 | done | B33: /output-style verbose cycled the style instead of setting it | — | [2026-11-06] origin/main iter-1773 |
| tui-bugfix-campaign-030 | done | B34: the catalogue advertised Alt+V — Toggle Vim mode while the repo had pinned that Alt+V must never become one; the row is gone and the Prompt context's 21 rows are now pinned | — | [2026-11-06] origin/main iter-1775 |
| tui-bugfix-campaign-031 | done | B35: the simulator could not gate an expected error, and unit tests rewrote the operator's real settings.json | — | [2026-11-06] origin/main iter-1778 |
| tui-bugfix-campaign-032 | done | B36: the TUI's footer and the gateway's /context quoted two different context windows again — the third resolver is deleted | — | [2026-11-06] origin/main iter-1779 |
| tui-bugfix-campaign-033 | done | B37: the flat 128000 window stub — a third answer, still the warning path's fallback — is deleted | — | [2026-11-06] origin/main iter-1780 |
| tui-bugfix-campaign-034 | done | B38: `/memory` was a registered command that could only print a usage dump — `operant memory` needs a subcommand, and the `/memory on|off` form /help advertised does not exist | — | [2026-11-06] origin/main iter-1781 |
| tui-bugfix-campaign-035 | done | B39: the /help overlay's filter was DEAD in the Commands view, and `/help <command>` dropped its argument — the row and the footer both promised a feature that did nothing | — | [2026-11-06] origin/main iter-1785 |
| tui-bugfix-campaign-036 | done | B40: the /help overlay's command rows were a stale second copy of the registry and advertised argument forms no command reads; the rows now read from the table, the lies are gone, and a local /help no longer lists the remote-only commands | — | [2026-11-06] origin/main iter-1786 |
| tui-bugfix-campaign-018 | done | B23: the per-row diff expansion scenario had no committed golden, so the corpus verify failed on every run | claimed-by `agent-omarchy` since 2026-11-05T13:13 | [2026-11-05] origin/main iter-1758 |
