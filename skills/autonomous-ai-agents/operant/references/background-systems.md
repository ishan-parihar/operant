# Operant Background Systems

- **delegate_task** — spawn an isolated sub-agent (subset of parent's tools; disabled-tool bans propagate so children can never regain a banned tool). Prefer for quick parallel subtasks.
- **Spawning processes** — run `operant run --query "…"` via the `terminal` tool (optionally in tmux / background) for long independent missions; see the parent skill body.
- **cron** — `operant cron create <name> <schedule> <command>`; scheduled agent runs with delivery.
- **kanban** — `operant kanban` task boards.
- **checkpoints** — opt-in snapshot store (`[checkpoints] enabled`).
- **LCM context engine** — `agent.context_engine = "lcm"`: lossless DAG, `lcm_recall`/`lcm_stats`/`lcm_assert` tools, background rollup maintenance for long-lived processes.
- **Memory sync** — `sync_turn` posts observations after each turn; drained on shutdown so writes aren't lost.
- **Autonomous** — `operant autonomous`: self-directed loop over TODO.md with test-command guardrails.
