# Operant CLI Reference

Run `operant --help` for the authoritative list. Key commands:

| Command | What it does |
|---|---|
| `operant` / `operant chat` | Interactive chat (ratatui TUI) |
| `operant run --query "…"` | One-shot run; `--record-trajectory` saves ReAct steps; `--autonomous` reads TODO.md |
| `operant autonomous` | Self-directed development loop |
| `operant setup` | Interactive setup wizard (provider, memory, TTS, gateway, agent) |
| `operant model get/set` | Active model configuration |
| `operant tools list` | Enabled tool registry |
| `operant skills list/search/inspect/install/uninstall/audit/seed/market/tap/toggle` | Skill management (note: `bundle` is a TUI slash command, not a CLI subcommand) |
| `operant mcp list/add/remove/test/serve/login/configure` | MCP servers (deferred loading; `/mcp r` in the TUI reconnects and materializes tools mid-session) |
| `operant memory` | Memory management |
| `operant sessions list` | Session history |
| `operant cron` | Scheduled jobs |
| `operant kanban` | Task boards |
| `operant gateway start` | Messaging gateway |
| `operant channel` | Telegram/discord/slack/whatsapp channel setup |
| `operant cookies` | Multi-browser cookie import for the Obscura session |
| `operant doctor` / `operant status` | Health checks / system overview |
| `operant profile` | Isolated instances with own config/skills/memory |
| `operant plugins` | WASM plugins |
| `operant context` | Inspect the lossless DAG context engine |
| `operant backup/import/uninstall/update` | Lifecycle management |

Flags: `--config <path>`, `--model`, `--base-url`, `--max-iterations`, `--tool-timeout`, `--no-stream`, `--dangerously-skip-permissions`, `--record-trajectory`.
