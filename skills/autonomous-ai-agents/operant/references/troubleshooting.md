# Operant Troubleshooting

- **`operant doctor`** — validate config + dependencies (run first)
- **`operant status`** / **`operant logs`** / **`operant debug`** — system overview, logs, debug reports
- **Model issues** — `operant model get/set`; check base_url/key in `[client]`; credential pool rotation for rate limits
- **Stream drops** — the loop retries transient SSE drops with bounded backoff and refunds the iteration budget
- **Memory not saving** — verify `[memory] provider` names the memory-wire family (`memory-wire`/`memory_wire`/`agentmemory`/`memory` are all accepted); the store is a bank-isolated SQLite under the operant home (`~/.operant/memory_wire.sqlite`, personal bank)
- **MCP tools missing** — the server may be deferred: `/mcp r` in the TUI (or a new run, where the agent loop connects servers on first use) materializes tools mid-session
- **Skills not loading** — check `~/.operant/skills/<name>/SKILL.md` exists and frontmatter parses; `operant skills seed --force` re-seeds
- **Browser issues** — verify `igs`/`obscura` on PATH (`install-browser-deps.sh`); Linux may need `libnss3 libatk1.0-0 libcups2 libgbm1 …`
- **Config corrupt** — never hand-edit operant.toml; use `operant config set KEY VAL`; restore from `operant backup`
