# Operant CLI Reference

Run `operant --help` for the authoritative list. Key commands:

| Command | What it does |
|---|---|
| `operant` / `operant chat` | Interactive chat (ratatui TUI) |
| `operant run --query "…"` | One-shot run; `--record-trajectory` saves ReAct steps; `--autonomous` reads TODO.md |
| `operant autonomous` | Self-directed development loop |
| `operant setup` | Interactive setup wizard (provider, memory, TTS, gateway, agent) |
| `operant model show/set` | Active model configuration |
| `operant tools list` | Enabled tool registry |
| `operant skills list/search/inspect/install/uninstall/audit/seed/market/tap/toggle` | Skill management (note: `bundle` is a TUI slash command, not a CLI subcommand) |
| `operant mcp list/add/remove/test/serve/login/configure` | MCP servers (deferred loading; `/mcp r` in the TUI reconnects and materializes tools mid-session) |
| `operant memory` | Memory management |
| `operant sessions list/show/search/export/prune/rename` | Session history, FTS5 search, export |
| `operant cron` | Scheduled jobs |
| `operant kanban` | Task boards |
| `operant gateway start` | Messaging gateway |
| `operant channel` | Telegram/discord/slack/whatsapp channel setup |
| `operant cookies` | Multi-browser cookie import for the Obscura session |
| `operant doctor` / `operant status` | Health checks / system overview |
| `operant profile` | Isolated instances with own config/skills/memory |
| `operant plugins` | WASM plugins |
| `operant sop list/show/validate` | Standard operating procedures (SOPs) — reusable runbooks loaded into the loop |
| `operant webhook subscribe/list/remove/test` | Webhook subscriptions (inbound events) |
| `operant auth list/add/remove/reset/status` + `login` / `logout` | Credential pool per provider (key hints only) + provider login |
| `operant hardware discover/introspect/info` / `peripheral` | USB device discovery, chip introspection (probe-rs), hardware peripherals |
| `operant service` | OS service lifecycle (systemd/launchd) |
| `operant checkpoints status/list/prune/clear` | Snapshot store management |
| `operant context` | Inspect the lossless DAG context engine |
| `operant backup/import/uninstall/update` | Lifecycle management |
| `operant trajectory` / `insights` / `dump` / `test` / `completion` / `migrate` | Trajectory recordings, usage insights, setup report, tool smoke-tests, shell completion, migration from other runtimes |

Flags: `--config <path>`, `--model`, `--base-url`, `--max-iterations`, `--tool-timeout`, `--no-stream`, `--dangerously-skip-permissions`, `--record-trajectory`.
