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
| tui-bugfix-campaign-039 | done | B43: the command palette printed its category header again and again — its rows were in registration order, so the categories interleaved | — | [2026-11-06] origin/main iter-1791 |
| tui-bugfix-campaign-040 | done | B44: the MCP panel's tool filter could not hold an 'r' or an 'e' — typing "read" silently restarted the MCP runtime | — | [2026-11-06] origin/main iter-1793 |
| tui-bugfix-campaign-041 | done | B45: the /help overlay's lower half was unreachable (the arrow scroll stopped at line 49 of ~200) and its footer advertised three scroll keys that did not exist; the env-store's temp paths collided with a leftover directory | — | [2026-11-06] origin/main iter-1795 |
| tui-bugfix-campaign-042 | done | B46: the composer's streaming hint promised "Ctrl/Cmd+Enter to send now" — a key action operant does not have — and the accessor it branched on was plan mode wearing a queue mode's name and doc | — | [2026-11-06] origin/main iter-1796 |
| tui-bugfix-campaign-043 | done | B47: the mouse wheel scrolled the transcript behind every open panel but the ten the mouse's own gate happened to list — the gate now reads the overlay catalogue | — | [2026-11-06] origin/main iter-1802 |
| tui-bugfix-campaign-044 | done | B48: the workspace's clippy gate was RED — the floating stable toolchain added two lints the code cannot satisfy as written — and iter-1802's own commit had misplaced an `#[expect]` attribute | — | [2026-11-06] origin/main iter-1804 |
| tui-bugfix-campaign-045 | done | B49: the composer charged the send-mode glyph its UTF-8 byte length, so a plan-mode draft wrapped two columns early | — | [2026-11-06] origin/main iter-1807 |
| tui-bugfix-campaign-046 | done | B50: the Overview widget admitted itself on a section it never painted — so a todo list lost its own panel to a one-line summary | — | [2026-11-06] origin/main iter-1810 |
| tui-bugfix-campaign-047 | done | B51: the key-routing chain's guard did not exist, and the guard that did exist checked nothing | — | [2026-11-06] origin/main iter-1812 |
| tui-bugfix-campaign-048 | done | B52: a click with a panel open selected (and copied) the transcript text behind the panel | — | [2026-11-06] origin/main iter-1815 |
| tui-bugfix-campaign-049 | done | B53: the wheel was inert in the diff viewer and the skills view — the panels that CAN scroll, but were not wired to it | — | [2026-11-06] origin/main iter-1818 |
| tui-bugfix-campaign-050 | done | B54: the campaign's own ledger gate could not see a duplicate id — and the ledger carries eight of them | — | [2026-11-07] origin/main iter-1820 |
| tui-bugfix-campaign-051 | done | B55: the wheel did nothing in the ten panels whose list carries a cursor | — | [2026-11-07] origin/main iter-1828 |
| tui-bugfix-campaign-052 | done | B56: a release inside an open panel copied a selection made before the panel opened | — | [2026-11-07] origin/main iter-1830 |
| tui-bugfix-campaign-053 | done | B57: the unified picker's wheel did nothing — a long model list could only be walked with the arrow keys | — | [2026-11-07] origin/main iter-1835 |
| tui-bugfix-campaign-054 | done | B58: /model <name> and /effort <level> dropped their argument — the picker opened unfiltered | — | [2026-11-07] origin/main iter-1838 |
| tui-bugfix-campaign-055 | done | B59: the diff viewer's footer advertised a key that does nothing | — | [2026-11-07] origin/main iter-1840 |
| tui-bugfix-campaign-056 | done | B60: the suite's own gate flaked — another test's delegation landed in the frame | — | [2026-11-07] origin/main iter-1851 |
| tui-bugfix-campaign-057 | done | B61: the fleet lesson from iter-1840's revert — build the staged ledger from origin/main, and commit with --only | — | [2026-11-07] origin/main iter-1853 |
| tui-bugfix-campaign-058 | done | B62: the /help overlay had no page scroll — hundreds of lines, one at a time | — | [2026-11-07] origin/main iter-1854 |
| tui-bugfix-campaign-059 | done | B63: the settings screen's wheel did nothing — the mouse's dialog gate swallowed it | — | [2026-11-07] origin/main iter-1856 |
| tui-bugfix-campaign-060 | done | B64: the command palette's category headers said nothing for 58 of its 111 rows — the catch-all held the majority of the catalogue | — | [2026-11-07] origin/main iter-1877 |
| tui-bugfix-campaign-061 | done | B65: the session hub's subagent detail promised "Esc returns to the list" — Esc closes the whole hub | — | [2026-11-07] origin/main iter-1882 |
| tui-bugfix-campaign-062 | done | B66: the skills view's detail footer said "Esc close" — Esc went back to the list instead | — | [2026-11-07] origin/main iter-1885 |
| tui-bugfix-campaign-063 | done | B67: the keybinding catalogue's Completion context said Tab "Next completion" — it accepts | — | [2026-11-07] origin/main iter-1887 |
| tui-bugfix-campaign-064 | done | B68: the generic Dialog context named Tab "Next field" — most dialogs have no Tab | — | [2026-11-07] origin/main iter-1890 |
| tui-bugfix-campaign-065 | done | B69: the TUI's golden corpus was RED — 11 drifted baselines and one dead assertion from the notice TTL | — | [2026-11-08] origin/main iter-1893 |
| tui-bugfix-campaign-066 | done | B70: /settings offered two rows whose values nothing read, and hid the @-autocomplete's own settings behind the dead one | — | [2026-11-08] origin/main iter-1896 |
| tui-bugfix-campaign-067 | done | B71: /settings offered "Show current directory" and "Show git branch" — the header painted both unconditionally | — | [2026-11-08] origin/main iter-1898 |
| tui-bugfix-campaign-068 | done | B72: /settings offered four more rows whose values nothing read, and /statusbar existed only to toggle one of them | — | [2026-11-08] origin/main iter-1899 |
| tui-bugfix-campaign-069 | done | B73: /settings offered five more rows whose values nothing read (the last of the family), and two of them were not simple ported rows | — | [2026-11-08] origin/main iter-1904 |
| tui-bugfix-campaign-070 | done | B74: the typeahead golden's command count follows the /statusbar retirement | — | [2026-11-08] origin/main iter-1906 |
| tui-bugfix-campaign-071 | done | B75: /settings offered "Reduce motion" and the value reached a global nothing read — the one surviving dead row a grep could not see | — | [2026-11-08] origin/main iter-1914 |
| tui-bugfix-campaign-072 | done | B76: on a short terminal /settings let the operator walk the selection off the painted list — two windows, two numbers | — | [2026-11-08] origin/main iter-1917 |
| tui-bugfix-campaign-073 | done | B77: the catalogue claimed Ctrl+U pages up and Ctrl+D pages down — neither does, and nothing in this TUI pages | — | [2026-11-08] origin/main iter-1919 |
| tui-bugfix-campaign-074 | done | B78: the onboarding cards promised a number-selection picker that does not exist — a latent false claim behind the empty suggestion seam | — | [2026-11-08] origin/main iter-1929 |
| tui-bugfix-campaign-075 | done | B79: the /review footer promised space-collapse in the detail pane, where space is gated out and inert | — | [2026-11-08] origin/main iter-1930 |
| tui-bugfix-campaign-076 | done | B80: the memory file selector advertised a selection that nothing consumes — Enter is byte-for-byte Esc | — | [2026-11-08] origin/main iter-1931 |
| tui-bugfix-campaign-077 | done | B81: the Dialog bucket claimed Enter for every dialog, and two dispatcher section headers named chords their sections no longer dispatch | — | [2026-11-08] origin/main iter-1932 |
| tui-bugfix-campaign-078 | done | B82: scripts/build.sh — the only documented entry point — failed on a healthy tree: the cargo it resolved guarded on a STALE user-bus socket | — | [2026-11-08] origin/main iter-1933 |
| tui-bugfix-campaign-079 | done | B83: the rewind selector's header promised a rewind while the filter was open, where Enter only closes the filter | — | [2026-11-08] origin/main iter-1934 |
| tui-bugfix-campaign-080 | done | B84: the ask-user dialog's hint overflowed its panel and the border clipped the last claim — found by the session's class pin | — | [2026-11-08] origin/main iter-1935 |
| tui-bugfix-campaign-081 | done | B85: the ledger repair — six of my own audit rows had vanished since iter-1933 while the gate stayed green, because each dropped row removed both the index row and its checkbox | — | [2026-11-08] origin/main iter-1936 |
| tui-bugfix-campaign-082 | done | B86: the six user-facing TUI fixes were missing from docs/CHANGELOG.md — now in [Unreleased] → ### Fixed, each derived from its ledger row | — | [2026-11-08] origin/main iter-1937 |
| tui-bugfix-campaign-083 | done | B87: the @-autocomplete settings' consumer half was unpinned — every popup test passed a fixed 15 against the repo's own cwd | — | [2026-11-09] origin/main iter-1939 |
| tui-bugfix-campaign-084 | done | B88: the ledger gate was RED across five commits — iter-1968's repair note collided with the row it annotates (RT-384 vs RT-384-REPAIR) | — | [2026-11-09] origin/main iter-1941 |
| tui-bugfix-campaign-085 | done | B89: verification round — the release gate through the repaired entry point, the full corpus at 0 drift, and the audit's completion record | — | [2026-11-09] origin/main iter-1943 |
| tui-bugfix-campaign-086 | done | B90: the message context menu offered a fork the TUI cannot perform — /branch is the hub's branch filter and drops the argument | — | [2026-11-09] origin/main iter-1944 |
| tui-bugfix-campaign-087 | done | B90: the message context menu offered a fork the TUI cannot perform — /branch is the hub's branch filter and drops the argument | — | [2026-11-09] origin/main iter-1944 |
| tui-bugfix-campaign-088 | done | B91: the claim-issuer class swept — copy mode, MCP panel, export dialog, /hotkeys, agents menu, all honest | — | [2026-11-09] origin/main iter-1945 |
| tui-bugfix-campaign-089 | done | B92: the indirect-dispatch inventory completed — six issuers enumerated with per-surface existence proofs, only iter-1944's fork dishonest | — | [2026-11-09] origin/main iter-1946 |
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
| tui-bugfix-campaign-037 | done | B41: the corpus's own notes claimed the typeahead "paints zero rows" for two-or-more-character prefixes — measured false, corrected, and pinned | — | [2026-11-06] origin/main iter-1788 |
| tui-bugfix-campaign-038 | done | B42: the per-row diff expansion scenario had no committed golden, so the corpus verify failed on every run (id renumbered from 018 and B-number from B23 at iter-1788: the concurrent slice published a different 018/B23 at the same time, so the board's ids were ambiguous) | claimed-by `agent-omarchy` since 2026-11-05T13:13 | [2026-11-05] origin/main iter-1758 |
