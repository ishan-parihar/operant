# Operant

![Rust](https://img.shields.io/badge/Rust-1.78+-orange?logo=rust)
![License](https://img.shields.io/badge/License-MIT-green)
![llama.cpp](https://img.shields.io/badge/llama.cpp-GGUF-purple)
![MCP](https://img.shields.io/badge/MCP-1.0-orange?logo=modelcontextprotocol)
![Local](https://img.shields.io/badge/100%25-Local_First-brightgreen)


**Your personal AI agent. Remembers what matters. Runs in your terminal.**

![Operant terminal](https://github.com/ishan-parihar/operant/raw/main/assets/readme/operant-terminal.png)

---

## What it is

Operant is a local-first AI agent with persistent memory, tool use, and terminal-native UX. No cloud, no API keys required (uses local LLMs via llama.cpp).

---

## Quick start

```bash
# Install
cargo install --git https://github.com/ishan-parihar/operant

# Run
operant
# > You: Remember I prefer Rust over Python for systems work
# > Operant: Got it. I'll bias toward Rust for systems tasks.
```

---

## Features

| Capability | Implementation |
|------------|----------------|
| **Memory** | Vector + graph (TDG-lite), automatic consolidation |
| **Tools** | 15 built-in (fs, git, web, code, shell, notes) |
| **Models** | llama.cpp (GGUF), OpenAI-compatible, Ollama |
| **Interface** | REPL, TUI (ratatui), MCP server |
| **Privacy** | 100% local, zero telemetry |

---


## Configuration

```toml
# ~/.config/operant/config.toml
[model]
backend = "llama.cpp"
path = "~/models/mistral-7b-instruct.Q4_K_M.gguf"
context = 8192

[memory]
backend = "tdg"
path = "~/.operant/memory"
consolidation_interval = 3600

[tools]
enabled = ["fs", "git", "web", "shell", "code", "notes"]
```

---

## Commands

| Command | Description |
|---------|-------------|
| `operant` | Start REPL |
| `operant tui` | Start TUI |
| `operant mcp` | Start MCP server |
| `operant memory query "..."` | Search memory |
| `operant memory export` | Export to JSON |

---



## Visual proof

| REPL session | TUI interface | Memory query |
|:---:|:---:|:---:|
| ![REPL](https://github.com/ishan-parihar/operant/raw/main/assets/readme/repl.png) | ![TUI](https://github.com/ishan-parihar/operant/raw/main/assets/readme/tui.png) | ![Memory](https://github.com/ishan-parihar/operant/raw/main/assets/readme/memory.png) |

| Tool registry | Config | MCP server |
|:---:|:---:|:---:|
| ![Tools](https://github.com/ishan-parihar/operant/raw/main/assets/readme/tools.png) | ![Config](https://github.com/ishan-parihar/operant/raw/main/assets/readme/config.png) | ![MCP](https://github.com/ishan-parihar/operant/raw/main/assets/readme/mcp.png) |

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Terminal  │────▶│   Operant   │────▶│   Memory    │
│   (REPL/TUI)│     │   Core      │     │   (TDG)     │
└─────────────┘     └──────┬──────┘     └─────────────┘
                           │
                    ┌──────▼──────┐
                    │   Tools     │
                    │  (registry) │
                    └─────────────┘
```

---

## Requirements

- Rust 1.89+
- 4 GB RAM (8 GB recommended)
- llama.cpp compatible model

---

## License

MIT — see [LICENSE](LICENSE).
