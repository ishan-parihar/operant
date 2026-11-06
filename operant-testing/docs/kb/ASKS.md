# Asks — what we asked the operator, and what they ruled

Status: LIVE · APPEND-ONLY (rows are amended in place by ruling, never deleted) ·
Vocabulary: `open` | `ruled` | `withdrawn`.

| id | asked | status | question | ruling | applied |
|---|---|---|---|---|---|
| Q1 | 2026-11-06 | open | G-013: `.serena/`, `.tmp-test/`, `.zcode/`, `.tmpwork/`, `_kernel/` are untracked in the tree. Do we gitignore each, or delete them? They are not part of any protocol and they make `git status` unreadable. | — | — |
| Q2 | 2026-11-06 | open | G-005: the drain-until-match MCP fix is ready (27/27 green) and uncommitted, because the shared tree carries peers' WIP in other files. Confirm I should commit only `crates/operant-core/src/mcp.rs` under the next free iteration label and push. | — | — |
