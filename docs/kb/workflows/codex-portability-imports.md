# Workflow: codex-portability-imports

`genome wf add codex-portability-imports "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| codex-portability-imports-001 | done | A: sandbox comment fix (blast_radius.rs, approval.rs) | — | [2026-10-31] crates/operant-core/tests/no_phantom_isolation_layer.rs |
| codex-portability-imports-002 | done | 3: tool dispatch trace via observer seam | claimed-by `agent-omarchy` since 2026-10-30T16:31 | [2026-10-31] crates/operant-core/tests/tool_call_duration_is_real.rs |
| codex-portability-imports-003 | in-progress | 4: relevance ranking (DONE iter-1510, BUGS RT-200) + B: MCP schema cap (OPEN) | claimed-by `agent-omarchy` since 2026-10-30T22:19 | [2026-10-30] ranking: crates/operant-core/src/tools/tool_search.rs; schema cap still open |
| codex-portability-imports-004 | done | plan 2026-10-31 row 1: ToolExposure as a type-level decision | claimed-by `agent-omarchy` since 2026-10-31T05:19 | [2026-10-31] crates/operant-core/src/tools.rs |
| codex-portability-imports-005 | done | plan 2026-10-31 row 2: context budget on the turn_context tail (~12 lines) | claimed-by `agent-omarchy` since 2026-10-31T06:51 | [2026-10-31] crates/operant-core/src/agent/run.rs |
| codex-portability-imports-006 | done | plan 2026-10-31 row 3: dispatch trace payload (input/output bytes) | implemented by `Sisyphus` in 79f2bd5c; `agent-omarchy`'s parallel attempt discarded as duplicate; repaired by `agent-omarchy` | [2026-10-31] implementation: agent/stream.rs park+read, agent/run.rs:2211-2212 emit, sized-tool test (Sisyphus, 79f2bd5c); repair: crates/operant-core/src/observer.rs:49-61 (fields now exist on the variant), :305-318 (tracing arm binds them) — 2cd1a0e2 |
| codex-portability-imports-007 | open | plan 2026-10-31 row 4: MCP schema byte cap (truncate + measure) | — | — |
| codex-portability-imports-008 | open | plan 2026-10-31 row 5: token-budget compaction mode, zero model calls | — | — |
