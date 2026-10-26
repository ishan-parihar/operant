# Workflow: ponytail-integration

`genome wf add ponytail-integration "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| ponytail-integration-001 | done | Phase 1: InstallKind::Content in binary_updater + ponytail spec module | claimed-by `jcode` since 2026-10-26T09:01 (re-stamped: original claim omitted --agent, F0035 class) | done 2026-10-26 — iter-1306 37902c75 pushed; 16 tests green; clippy gate clean; release deployed to ~/.local/bin + gateway restarted (md5 5a4fb7a0 verified at /proc/exe) |
| ponytail-integration-002 | in-progress | Phase 2: config + startup kick + update/doctor rows | claimed-by `jcode` since 2026-10-26T11:46 | — |
| ponytail-integration-003 | open | Phase 3: SkillManager extra roots + exposure wiring | — | — |
| ponytail-integration-004 | open | Phase 4: frozen-prefix injection + /ponytail command | — | — |
| ponytail-integration-005 | open | Phase 5: docs (CHANGELOG, README, AGENTS.md) | — | — |
