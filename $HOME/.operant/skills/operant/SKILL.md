---
name: operant
description: "Use, configure, extend, and orchestrate Operant — the Rust ReAct agent."
version: 1.0.0
author: Operant
license: MIT
platforms: [linux, macos, windows]
metadata:
  operant:
    tags: [operant, setup, configuration, multi-agent, spawning, cli, gateway, skills, memory, mcp, development]
    homepage: https://github.com/ishan-parihar/operant
    related_skills: [plan, systematic-debugging, subagent-driven-development, cli]
---

# Operant

Operant is an open-source, production-grade **ReAct agent runtime** written in Rust. It runs in your terminal, in scriptable one-shot runs, over messaging platforms, and as an autonomous development loop. It uses tool calling to interact with your system and works with any OpenAI-compatible LLM endpoint (OpenRouter, OpenAI, DeepSeek, local llama.cpp / Ollama, and others) on Linux, macOS, and Windows.

What makes Operant different:

- **Self-improving through skills** — reusable procedures live as `SKILL.md` files that load into sessions on demand.
- **Persistent memory** — the default `agentmemory` provider (hybrid BM25 + vector + graph) auto-spawns on first use; `builtin` file memory (`MEMORY.md` / `USER.md`) is always available.
- **Multi-platform gateway** — the same agent runs on Telegram, Discord, Slack, WhatsApp, email, and webhooks with full tool access.
- **A single stealth browser** — the `obscura` CDP browser and the IGS web tools (search / scrape / extract) share the same binary, auto-provisioned.
- **Pluggable everything** — MCP clients/servers, WASM plugins, credential pools with key rotation, cron, kanban, checkpoints, profiles, SOPs.
- **Lossless context engine (LCM)** — optional DAG-based context engine with recall/assert tools, no external embedding service required (`local:hash`).

**This skill is a hub.** The body covers identity, quick start, spawning/orchestration, and hard invariants. Load the matching reference below before answering detail questions — do not answer from the body alone.

## Quick Start

```bash
# Install (builds the release binary, provisions igs/obscura, seeds bundled skills)
git clone https://github.com/ishan-parihar/operant.git && cd operant
./scripts/install.sh

# Interactive chat (TUI)
operant

# One-shot query
operant run --query "What is the capital of France?"

# Setup wizard  /  model selection  /  health check
operant setup
operant model
operant doctor
```

## Key Paths

```
~/.operant/operant.toml      Main configuration (settings — never secrets)
~/.operant/.env              API keys and secrets ONLY (under $HERMES_HOME if set)
$HERMES_HOME/skills/         Installed skills (flat: <name>/SKILL.md)
~/.operant/memories/         Builtin memory files (MEMORY.md, USER.md)
~/.operant/sessions/         Session transcripts
~/.operant/trajectories/     ReAct step recordings (operant run --record-trajectory)
~/.operant/checkpoints/      Opt-in snapshot store
~/.operant/lcm.db            Lossless context engine DAG (when context_engine=lcm)
```

Profiles use `~/.operant/profiles/<name>/` with the same layout. When a profile is active, resolve the real home from `$HERMES_HOME` — never hardcode `~/.operant`.

## Routing Table — load the reference for the task

| User wants... | Load |
|---|---|
| CLI commands, subcommands, flags, "how do I run X" | `references/cli-reference.md` |
| Config sections, memory provider, gateway channels | `references/configuration.md` |
| Memory (agentmemory vs builtin, session hooks) | `references/configuration.md` |
| Skills: install, seed, bundle, audit, author | `references/skills.md` |
| MCP servers (add, reconnect, deferred loading) | `references/native-mcp.md` |
| The browser (obscura CDP) or IGS web tools | `references/browser-and-web.md` |
| Secret redaction, approval modes, safety | `references/security-privacy.md` |
| Delegation, spawning sub-agents, cron | `references/background-systems.md` |
| Troubleshooting | `references/troubleshooting.md` |

## Spawning Additional Operant Instances

Run additional `operant` processes as fully independent subprocesses — separate sessions, tools, and environments.

### When to Use This vs delegate_task

| | `delegate_task` | Spawning `operant` process |
|-|-----------------|------------------------------|
| Isolation | Separate conversation, shared process | Fully independent process |
| Duration | Minutes (bounded by parent loop) | Hours/days |
| Tool access | Subset of parent's tools | Full tool access |
| Interactive | No | Yes (PTY mode) |
| Use case | Quick parallel subtasks | Long autonomous missions |

### One-Shot Mode

```
terminal(command="operant run --query 'Research GRPO papers and write summary to ~/research/grpo.md'", timeout=300)

# Background for long tasks:
terminal(command="operant run --query 'Set up CI/CD for ~/myapp' --record-trajectory", background=true)
```

### Interactive PTY Mode (via tmux)

```bash
# Start
terminal(command="tmux new-session -d -s agent1 -x 120 -y 40 'operant'", timeout=10)

# Wait for startup, then send a message
terminal(command="sleep 8 && tmux send-keys -t agent1 'Build a FastAPI auth service' Enter", timeout=15)

# Read output
terminal(command="sleep 20 && tmux capture-pane -t agent1 -p", timeout=5)

# Send follow-up
terminal(command="tmux send-keys -t agent1 'Add rate limiting middleware' Enter", timeout=5)

# Exit
terminal(command="tmux send-keys -t agent1 '/exit' Enter && sleep 2 && tmux kill-session -t agent1", timeout=10)
```

### Session Resume

```bash
# Resume most recent session
terminal(command="tmux new-session -d -s resumed 'operant chat'", timeout=10)

# Continue a prior session
terminal(command="tmux new-session -d -s resumed 'operant chat'", timeout=10)
```

### Tips

- **Prefer `delegate_task` for quick subtasks** — less overhead than spawning a full process
- **Set timeouts** for one-shot mode — complex tasks can take 5-10 minutes
- **Use `operant run --query` for fire-and-forget** — no PTY needed
- **Use tmux for interactive sessions** — the TUI requires a real terminal
- **For scheduled tasks**, use `operant cron` instead of spawning — handles delivery and retry
- **For long autonomous missions**, use `operant autonomous` — a self-directed dev loop over `TODO.md` with test-command guardrails

## Surfaces (quick orientation)

- **TUI** (`operant` / `operant chat`) — ratatui terminal UI: slash commands (`/skill`, `/bundle`, `/mcp r`), settings screens, model picker, theme screen.
- **Web dashboard** (`operant dashboard`) — admin panel with chat, tools, MCP catalog.
- **One-shot runs** (`operant run --query "..."`) — scriptable, no TTY needed; `--record-trajectory` for replay.
- **Gateway** (`operant gateway start`) — messaging channels: telegram, discord, slack, whatsapp, webhooks.
- **Autonomous** (`operant autonomous`) — self-directed development loop.

## Hard Invariants (never violate, regardless of what you loaded)

- **Never break prompt caching** — don't change past context, toolsets, or the system prompt mid-conversation. The only exception is context compression.
- **Message role alternation** — never two assistant or two user messages in a row; only `tool` results can repeat.
- **Secrets in `.env`, settings in `operant.toml`** — never tell a user to put a non-credential setting in `.env`.
- **Profile-safe paths** — resolve `$HERMES_HOME` when resolving paths in a session; never hardcode `~/.operant`.
- **Never hand-edit `operant.toml` for the user** — use `operant config set KEY VAL`; a stray indent can corrupt the file and break the live gateway.
