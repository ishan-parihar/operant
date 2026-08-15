# Operant Skills

Skills are markdown instruction packs (`SKILL.md`) that inject guidance into the agent on demand.

## Layout

- User skills dir: `~/.operant/skills/<skill>/SKILL.md` (flat; `HERMES_SKILLS_DIR` overrides)
- Repo pool: categorized (`skills/<category>/<skill>/SKILL.md`); shipped and seeded at install
- Frontmatter: `name`, `description`, `version`, `author`, `license`, `platforms`, `metadata.operant.{tags,related_skills}`

## Management

- `/skill <name>` in the TUI invokes a skill; `/bundle <name>` expands a bundle
- `operant skills list` / `search` / `inspect` / `install ./dir-or-url` / `uninstall` / `audit` / `seed [--source DIR] [--force]` / `market` / `tap` / `toggle` (note: `bundle` is a TUI slash command, not a CLI subcommand)
- `operant skills market search/install <name>` — marketplace
- `operant curator` — agent-curated lifecycle (archive/backup/restore)
- First-run auto-seeds the bundled pool when the dir is empty; `install-skills.sh` pre-seeds at install time
- `skill_manage` tool: create/edit/patch skills in-session

## Authoring

- One skill = one directory with SKILL.md + optional references/scripts/templates
- Name operant tools in backticks: `terminal`, `file_read`, `file_write`, `patch`, `file_search`, `web_search`, `web_fetch`, `browser` (navigate/snapshot/click/type/scroll/accessibility_tree), `computer_use`, `vision_analyze`, `delegate_task`, `skill_manage`
- Skills_guard scans on install for unsafe patterns (unpinned pip, rm -rf, etc.)
