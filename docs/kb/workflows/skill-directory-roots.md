# Workflow: skill-directory-roots

`genome wf add skill-directory-roots "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| skill-directory-roots-001 | done | Publish the measured plan: extra_roots + project_root, one SkillRoots resolver for every skill surface | claimed-by `agent-omarchy` since 2026-11-14T07:35 | [2026-11-14] docs/plans/2026-11-14-skill-directory-configuration-global-project-and-manual-root.md |
| skill-directory-roots-002 | done | Land the resolver + the recursive loader + the config keys (code iteration) | — | [2026-11-14] crates/operant-core/src/skills.rs |
| skill-directory-roots-003 | done | Thread SkillRoots through the popup, /skills, skills_list/skill_view, gateway picker, SkillManager | — | [2026-11-14] iter-2295 (5b2537ae): 13 surfaces read the resolver's list. CORRECTED by iter-2307: the original "CLI failure set identical to base" reading was taken against a tree that could NOT compile (it had reverted 3 peer lines from iter-2281), so it proved nothing. Pinned properly by iter-2312's two seam tests (extra_roots + project_root reach the manager), mutation-proven. |
| skill-directory-roots-004 | done | Add the doctor row: resolved roots, rejected roots with reasons, dead symlinks, advertised vs dropped | — | [2026-11-14] iter-2283 (1b13a696): `skills` category on the one engine list |
| skill-directory-roots-005 | done | Land the resolver + config keys + recursive walk + / grammar (iter-2260, pushed cdd16b38) | — | [2026-11-14] crates/operant-core/src/skills.rs |
| skill-directory-roots-006 | done | Move the seeder's `collect_skill_dirs` onto the shared bounded walk (iter-2272, 2b6a02e1) | — | [2026-11-14] crates/operant-cli/src/cmd_skills.rs |
| skill-directory-roots-007 | done | Validate every per-surface claim in the plan against the code with a mutation-proven test | — | [2026-11-14] crates/operant-cli/src/tui/prompt_input/typeahead.rs |
