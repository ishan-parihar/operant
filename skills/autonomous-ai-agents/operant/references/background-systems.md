# Operant Background Systems

- **delegate_task** — spawn an isolated sub-agent (subset of parent's tools; disabled-tool bans propagate so children can never regain a banned tool). Prefer for quick parallel subtasks.
- **Spawning processes** — run `operant run --query "…"` via the `terminal` tool (optionally in tmux / background) for long independent missions; see the parent skill body.
- **kanban** — `operant kanban` task boards.
- **checkpoints** — opt-in snapshot store (`[checkpoints] enabled`).
- **LCM context engine** — `agent.context_engine = "lcm"`: lossless DAG, `lcm_recall`/`lcm_stats`/`lcm_assert` tools, background rollup maintenance for long-lived processes.
- **Memory sync** — `sync_turn` posts observations after each turn; drained on shutdown so writes aren't lost.
- **Autonomous** — `operant autonomous`: self-directed loop over TODO.md with test-command guardrails.
- **SOPs** — `operant sop list/show/validate`: reusable standard operating procedures (markdown runbooks) that inject constraints and workflows into sessions.
- **Webhooks** — `operant webhook subscribe/list/remove/test`: inbound event subscriptions that can trigger gateway behavior.

## Messaging gateway

The gateway runs the same agent loop (full tool access, not just chat) over
telegram, discord, slack, and whatsapp. Manage it with `operant gateway`:

| Command | What it does |
|---|---|
| `operant gateway status` | Enabled platforms + runtime state |
| `operant gateway channels` / `sessions` / `stats` | Live registry views |
| `operant gateway start` / `stop` / `restart` | Service lifecycle (systemd) |
| `operant gateway install` / `uninstall` | Install/remove the autostart systemd unit |
| `operant gateway run` | Foreground run (Ctrl+C to stop) — good for live testing |
| `operant channel list` | All configured channels |
| `operant channel start` | Start all configured channels |
| `operant channel doctor` | Health checks per platform |
| `operant channel add <type> [--token <TOKEN>] [config-json]` | Configure a platform (accepted types: `telegram`, `discord`, `slack`, `whatsapp`, `email`, `webhooks`; pass the bot token with `--token` — never the global `--api-key`, which overrides the LLM key) |
| `operant channel remove <type>` | Remove a channel configuration |
| `operant channel send --channel-id <name> --recipient <id> "<msg>"` | Send a test message to verify delivery |
| `operant channel bind-telegram` | Allowlist a telegram identity |

Workflow for wiring a platform: pass the bot token with `operant channel add <type> --token <TOKEN>` (or put it in `~/.operant/.env` as `TELEGRAM_BOT_TOKEN=…`), enable the channel in `[gateway]`, run
`operant channel doctor` to confirm the handshake, then `operant gateway run`
(or `install` for autostart). Delivery goes through the normal agent loop, so
an interactive gateway message can trigger any tool, skill, or cron job.

## Cron jobs

`operant cron` schedules agent runs:

| Command | What it does |
|---|---|
| `operant cron create <name> <schedule> <command>` | Schedule an agent run (cron syntax, e.g. `"0 9 * * *"` or `"every 6h"`; `--repeat N` to cap runs; prints the new job's ID) |
| `operant cron list` | List jobs — **note the `ID` column**: `get` / `pause` / `resume` / `delete` / `run` all take the **job ID** (e.g. `cron_f3b2cf6b`), not the name |
| `operant cron get <id>` / `update <id> [name] [schedule] [command]` | Inspect / manage a job by ID |
| `operant cron pause <id>` / `resume <id>` / `delete <id>` | Lifecycle by ID (status flips reflected in `cron status` totals) |
| `operant cron run <id>` | Trigger a run immediately (test before trusting the schedule) |
| `operant cron status` / `tick` | Scheduler health / force a due-check |
| `operant cron blueprint` | Create from a pre-built blueprint (morning brief, weekly digest — includes delivery presets) |

Jobs run with the same model/tools as interactive sessions. Delivery is set
per job via its `deliver` field (`[cron]` config — e.g. `local`, or a
configured channel such as `telegram`); `operant cron blueprint` comes with
delivery presets. `operant cron status` is the first thing to check when a
scheduled job is silent — then `operant cron run` to isolate schedule vs
execution problems.
