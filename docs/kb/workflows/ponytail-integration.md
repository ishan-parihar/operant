# Workflow: ponytail-integration

`genome wf add ponytail-integration "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| ponytail-integration-001 | done | Phase 1: InstallKind::Content in binary_updater + ponytail spec module | claimed-by `jcode` since 2026-10-26T09:01 (re-stamped: original claim omitted --agent, F0035 class) | done 2026-10-26 — iter-1306 37902c75 pushed; 16 tests green; clippy gate clean; release deployed to ~/.local/bin + gateway restarted (md5 5a4fb7a0 verified at /proc/exe) |
| ponytail-integration-002 | done | Phase 2: config + startup kick + update/doctor rows | claimed-by `jcode` since 2026-10-26T11:46 | done 2026-10-26 — iter-1307 c71e57be pushed; config tests + scope tests green; clippy gate pass; deploy: FIRST rebuild FAILED (rust-lld -lonnxruntime missing in worktree — local/ is gitignored and absent; also `\| tail -3` swallowed cargo's rc=101 and the watchdog showed exit 0). Fixed by symlinking main-tree local/ into the worktree + logging full build output to a file. Deploy verified by md5 after fix. |
| ponytail-integration-003 | done | Phase 3: SkillManager extra roots + exposure wiring | claimed-by `jcode` since 2026-10-26T13:02 | done 2026-10-26 — iter-1311 fe825f21 pushed (label shared with peer's docs 25db1b78 per collision doctrine, noted in body); SkillManager.add_extra_root + SkillOrigin + first-wins shadowing + delete-refusal; resolve_skill_dir integration fallback; wired at agent-core/gateway-reload/TUI-reload/skills-view/skills-list ([integration] marker + JSON origin); 5 new tests + plan acceptance test green (37/37 family); no_registered_tui_command_is_unwired + 96/96 gateway + 5/5 skills_view green after peer leg landed; clippy gate pass. Deploy build running. |
| ponytail-integration-004 | open | Phase 4: frozen-prefix injection + /ponytail command | — | — |
| ponytail-integration-005 | open | Phase 5: docs (CHANGELOG, README, AGENTS.md) | — | — |
