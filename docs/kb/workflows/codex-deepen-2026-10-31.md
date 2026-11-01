# Workflow: codex-deepen-2026-10-31

`genome wf add codex-deepen-2026-10-31 "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| codex-deepen-2026-10-31-001 | done | Deepen the codex contrast: eight unmeasured families documented + ranked ponytail plan | claimed-by `agent-omarchy` since 2026-10-31T17:47 | [2026-10-31] docs/audit/2026-10-31-codex-architecture-deepen/00-INDEX.md |
| codex-deepen-2026-10-31-002 | done | The execution outline: every twelve-rows plan row re-measured against the live tree, with edit sites, tests, mutation guards, and the READY/BLOCKED split | claimed-by `agent-omarchy` since 2026-10-31T21:16 | [2026-10-31] docs/plans/2026-10-31-codex-architecture-deepen-execution-outline.md |
| codex-deepen-2026-10-31-003 | done | Row 1 shipped: interpreter unwrap (iter-1595). Row 2 shipped: process hardening (iter-1597). | claimed-by `agent-omarchy` since 2026-11-01T05:27 | [2026-11-01] iter-1595 + iter-1597 |
| codex-deepen-2026-10-31-004 | done | Row 5 shipped (row 3 and row 4 were shipped earlier under the same workflow; the board rows lapsed, so this row records rows 3-5 together): the codex mermaid engine ported to tui/operant_mermaid/, wired through markdown_mermaid_fallback so a mermaid fence DRAWS instead of reporting "Mermaid rendering is disabled". 23 engine tests + 132 markdown tests green. Two mutation guards, each re-derived to fail by name before restore. | claimed-by `agent-omarchy` since 2026-11-01T01:50 | [2026-11-01] iter-1608 on origin/main; crates/operant-cli/src/tui/operant_mermaid/ |
