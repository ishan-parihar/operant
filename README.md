# Operant

![Rust](https://img.shields.io/badge/Rust-1.89+-orange?logo=rust)
![LOC](https://img.shields.io/badge/LOC-537K-informational?style=flat-square)
![Status](https://img.shields.io/badge/Status-Active-brightgreen)
[![CI](https://github.com/ishan-parihar/operant/actions/workflows/ci.yml/badge.svg)](https://github.com/ishan-parihar/operant/actions/workflows/ci.yml)
![Edition](https://img.shields.io/badge/Edition-2024-blueviolet)
![License](https://img.shields.io/badge/License-MIT_OR_Apache--2.0-green)
![MCP](https://img.shields.io/badge/MCP-1.0-orange?logo=modelcontextprotocol)
![Local](https://img.shields.io/badge/local--first-34d399)

**Your terminal-native AI agent.** Persistent memory, 60+ JSON-schema tools, skills, MCP, and messaging channels — built in Rust, run from your shell, and fully under your control.

![Operant hero](assets/readme/hero.svg)

---

## Quick start

```bash
git clone https://github.com/ishan-parihar/operant.git && cd operant

./scripts/install.sh          # release build → /usr/local/bin/operant
# or: cargo build --release -p operant-cli

operant setup                 # interactive wizard: provider, memory, TTS, gateway
operant chat                  # start chatting (TUI)
```

One-shot runs need no TUI at all:

```bash
operant run --query "Audit the browser stack: check IGS web tools and the CDP browser"
```

---

## What it is

Operant is a production-grade **ReAct agent runtime** written in Rust. It replaces the "script per task" pattern with one agent that can **think, use tools, and remember** — over a long-lived terminal session or in scriptable one-shot runs.

Why it is different:

- **A real agentic loop, not a chat wrapper** — think → act → observe with a JSON-schema tool registry, automatic memory-context injection, provider fallbacks, and self-healing retries.
- **Memory that behaves like a plugin** — the default `agentmemory` provider (BM25 + vector + graph, hybrid search) auto-spawns on first use and mirrors the hermes-agent memory-plugin lifecycle: `session/start` on init, `observe` after every turn, context recall before each turn, session/end on exit.
- **Skills you can point at a directory** — import an entire skill tree (with recursive security scan), bundle multiple skills, autoload at boot, and let the agent curate new ones.
- **Only enabled, functional tools reach the model** — the registry serves the intersection of *registered ∩ available ∩ not-disabled*, so the agent never sees tools that can't run.
- **One stealth browser for everything** — the `obscura` CDP browser and the IGS web tools (search / scrape / extract) share the same Obscura binary, auto-provisioned on first use.
- **Local-first** — no telemetry, no account required; bring any OpenAI-compatible endpoint or a local model.

---

## How it works

The agent loop is a classic ReAct cycle, executed with bounded iterations and tool timeouts:

```
 user ──▶ model ──▶ act (tool registry) ──▶ observe ──▶ next step
              ▲                                │
              └──────── memory context ◀───────┘
```

- **Registry** — every tool exposes a runtime JSON schema; `get_schemas()` feeds exactly what is enabled.
- **Memory** — pre-turn `prefetch()` injects `<memory_context>`; post-turn `queue_prefetch()` warms the provider; compression hooks fire on long sessions.
- **Resilience** — fallback model chains, rate-limit buckets with exponential backoff, and tool call healing (`max_healing_attempts`).

### Crate map

```
operant
├── crates/
│   ├── operant-core         agent loop · tool registry · memory provider · config
│   ├── operant-cli          TUI (ratatui) · commands · app adapter
│   ├── operant-tools        built-in tool implementations
│   ├── operant-providers    LLM provider adapters
│   ├── operant-memory       memory backends (agentmemory / builtin / …)
│   ├── operant-plugins      WASM plugin bridge
│   ├── operant-gateway      messaging gateway (telegram, discord, …)
│   ├── operant-channels     channel orchestrator
│   ├── operant-runtime      autonomous runtime agent
│   ├── operant-config       config schema · validation · defaults
│   └── …                    api · infra · macros · eval · hardware · robot-kit
```

---

## Features

| Capability | Implementation |
|---|---|
| **Memory** | `agentmemory` hybrid semantic memory (BM25 + vector + graph), auto-spawned; or `builtin` file memory (`MEMORY.md` / `USER.md`) |
| **Tools** | 60+ JSON-schema tools: fs, git, web (IGS search/scrape/extract), browser (CDP), shell, code, http, memory, skills, cron, kanban, process, notes, checkpoints |
| **Browser** | Stealth **Obscura** CDP — persistent socket, page sessions, shared binary with IGS web tools |
| **Skills** | Directory import with recursive security scan · bundles · autoload · curator |
| **Models** | Any OpenAI-compatible endpoint (`base_url`), local llama.cpp, Ollama; fallback chains + token-bucket rate limiting |
| **MCP** | Native client (stdio + HTTP, deferred loading) **and** server; reconnect materializes tools mid-session |
| **Channels** | Telegram · Discord · Slack · WhatsApp · email · webhooks via the gateway |
| **Autonomy** | `operant autonomous` — a self-directed dev loop over `TODO.md` with test-command guardrails |
| **Plugins** | WASM plugin tools + hermes-agent hook parity (before/after tool, turn, memory hooks) |
| **Interface** | ratatui TUI · interactive chat · scriptable `run` · one-shot `test` |

---

## Configuration

Config lives at `~/.operant/operant.toml` (secrets in `~/.operant/.env`). Start from the annotated reference:

```bash
cp operant.example.toml ~/.operant/operant.toml
operant doctor        # validate config + dependencies
operant status        # system overview
```

Highlights:

```toml
[client]              # any OpenAI-compatible endpoint
base_url = "https://api.openai.com/v1"

[agent]
model = "gpt-4"
fallbacks = [ { model = "gpt-4o-mini" } ]
max_iterations = 20

[memory]
provider = "agentmemory"      # or "builtin"

[tools]
igs_enabled = true            # IGS web tools + browser
# obscura_stealth = true

[skills]
autoload = true
```

See [`operant.example.toml`](operant.example.toml) for the full reference — every section is annotated.

---

## CLI reference

| Command | What it does |
|---|---|
| `operant` / `operant chat` | Interactive chat (TUI) |
| `operant run --query "…"` | One-shot run (scriptable, `--record-trajectory` for replay) |
| `operant autonomous` | Self-directed development loop |
| `operant setup` | Interactive setup wizard |
| `operant tools list` | Inspect the enabled tool registry |
| `operant skills list / install / audit` | Skill management (install a directory or URL) |
| `operant mcp list / connect` | MCP servers; `/mcp r` in the TUI reconnects deferred servers |
| `operant memory query / export` | Search / export memory |
| `operant sessions list` | Session history |
| `operant model get / set` | Active model configuration |
| `operant cron` / `operant kanban` | Scheduled jobs / task boards |
| `operant gateway start` | Messaging gateway (telegram, discord, slack, whatsapp) |
| `operant plugins list` | WASM plugins |
| `operant doctor` / `operant status` | Health checks and system overview |

`operant --help` lists the full set (sessions, checkpoints, profiles, auth, completion, backups, SOPs, hardware, and more).

---

## Requirements

- **Rust 1.89+** (edition 2024)
- 4 GB RAM (8 GB recommended)
- A model: any OpenAI-compatible endpoint, or a local llama.cpp / Ollama server
- Optional: [igs](https://github.com/ishan-parihar/igs-rust) binary for IGS web tools and the shared Obscura browser

---

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
./scripts/self-test.sh       # full pre-PR validation
```

Architecture, porting notes, and parity decisions live in [`AGENTS.md`](AGENTS.md) and [`docs/`](docs/) (audits, BUGS, CHANGELOG, TODO).

---

## License

MIT **or** Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) or [LICENSE-APACHE](LICENSE-APACHE).

---

## ☕ Support & Sponsorship

If you find this project useful, consider supporting ongoing development:

[![Sponsor](https://img.shields.io/badge/Sponsor-GitHub%20Sponsors-ea4aaa?style=flat-square&logo=github)](https://github.com/sponsors/ishan-parihar)
[![Donate](https://img.shields.io/badge/Donate-Razorpay-3395FF?style=flat-square)](https://rzp.io/rzp/ishan-parihar)

Your support funds new features, releases, and infrastructure for the whole ecosystem.