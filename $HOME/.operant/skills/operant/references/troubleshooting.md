# Operant Troubleshooting

- **`operant doctor`** — validate config + dependencies (run first)
- **`operant status`** / **`operant logs`** / **`operant debug`** — system overview, logs, debug reports
- **Model issues** — `operant model get/set`; check base_url/key in `[client]`; credential pool rotation for rate limits
- **Stream drops** — the loop retries transient SSE drops with bounded backoff and refunds the iteration budget
- **Memory not saving** — verify `[memory] provider`; agentmemory needs node/npx (auto-spawn); builtin writes MEMORY.md under `~/.operant/memories`
- **MCP tools missing** — the server may be deferred: `/mcp r` in the TUI (or a new run, where the agent loop connects servers on first use) materializes tools mid-session
- **Skills not loading** — check `~/.operant/skills/<name>/SKILL.md` exists and frontmatter parses; `operant skills seed --force` re-seeds
- **Browser issues** — verify `igs`/`obscura` on PATH (`install-browser-deps.sh`); Linux may need `libnss3 libatk1.0-0 libcups2 libgbm1 …`
- **Config corrupt** — never hand-edit operant.toml; use `operant config set KEY VAL`; restore from `operant backup`
