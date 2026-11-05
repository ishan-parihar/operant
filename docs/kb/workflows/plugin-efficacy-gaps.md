# Workflow: plugin-efficacy-gaps

`genome wf add plugin-efficacy-gaps "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| plugin-efficacy-gaps-001 | done | G7 creator mode: plugins new + --validate | claimed-by `agent-omarchy` since 2026-11-05T03:45 | done 2026-11-05: iter-1731 fcb69168 — scaffolds pass the installer's own validator (6 tests); evidence docs/kb/workflows/plugin-efficacy-gaps.md |
| plugin-efficacy-gaps-002 | done | G8 in-tree WASM fixture + real-module tests | claimed-by `agent-omarchy` | done: iter-1732 bc99a7a1 — fixture.wasm committed, 4 real-module tests |
| plugin-efficacy-gaps-003 | done | G6 live watcher re-registration (tools+observers) | claimed-by `agent-omarchy` | done: iter-1733 09a2a101 — apply_plugin_changes, named interceptors, 5 live tests; skills stay restart-bound (named upgrade) |
| plugin-efficacy-gaps-004 | done | G9 plugins status | claimed-by `agent-omarchy` | done: iter-1734 10bd68fd — honest discovery view, smoke-tested |
| plugin-efficacy-gaps-005 | done | G2 post-execute Replace verdict | claimed-by `agent-omarchy` | done: iter-1739 d37dc682 — PipelineAction::Replace, 3-level real-WASM pins |
| plugin-efficacy-gaps-006 | done | G1 turn-level pre-step seam | claimed-by `agent-omarchy` | done: iter-1742 15d998db — turn_gate opt-in, veto/rewrite before context prologue, 4-level pins |
| plugin-efficacy-gaps-007 | done | G10 dashboard /api/plugins + G3/G4/G5/G11 decisions | claimed-by `agent-omarchy` | done: iter-1744 d1c7c094 — route landed (page untouched, RT-44 intact); decisions recorded in docs/library/plugins.md |
