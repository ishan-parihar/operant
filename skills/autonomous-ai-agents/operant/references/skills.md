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
- Name operant tools in backticks — the full tool surface (verify live with `operant tools list`):
  - **Core file/exec**: `terminal`, `file_read`, `file_write`, `patch`, `file_search`, `file_list`, `file_state`, `code_execution`, `process`
  - **Web**: `web_search`, `web_fetch`, `web_extract`, `web_scrape`, `web_crawl`, `http_request`, `openrouter_query`, `xai_http_request`
  - **Browser**: `browser` (navigate/snapshot/click/type/scroll/accessibility_tree/cookies_*), `browser_cdp`, `browser_dialog`, `browser_camofox_state`
  - **Memory**: `memory_save`, `memory_search`, `memory_store`, `memory_recall`, `memory_smart_search`
  - **Skills/MCP**: `skills_list`, `skill_view`, `skill_manage`, `mcp_management`
  - **Context/LCM**: `lcm_recall`, `lcm_recent`, `lcm_stats`, `lcm_assert`, `lcm_vector_recall`, `lcm_doctor`, `lcm_load_session`, `lcm_recall_round`
  - **Agentic**: `delegate_task`, `clarify`, `todo`, `kanban`, `cron`, `checkpoint`, `approval_request`, `notify`, `send_message`, `session_search`, `session_insights`, `vision_analyze`, `video_analyze`, `image_generate`, `text_to_speech`, `transcribe_audio`, `config_manage`, `datetime`, `timestamp`, `debug_env`, `debug_system`, `osv_check`, `learning_manage`, `tool_backend`, `echo`, `neutts_synthesize`
  - **Feishu**: `feishu_doc_read`, `feishu_drive`
  - `computer_use` is macOS-only (cua-driver); it is registered only on macOS
- Skills_guard scans on install for unsafe patterns (unpinned pip, rm -rf, etc.)
