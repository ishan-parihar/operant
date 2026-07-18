# Operant ⚡

**Your personal AI agent. Remembers what matters. Runs in your terminal.**

[![Rust 1.86+](https://img.shields.io/badge/rust-1.86%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT/Apache](https://img.shields.io/badge/License-MIT/Apache-yellow.svg)](LICENSE)
[![TUI](https://img.shields.io/badge/UI-Ratatui-blue.svg)](https://github.com/ratatui-org/ratatui)

Operant is a personal AI agent that lives in your terminal. It remembers
your conversations, learns your patterns, and is available across every
messaging platform you use — Telegram, Discord, Slack, and more.

Unlike a chatbot, operant:
- **Remembers** — uses a graph memory (TDG) that self-organizes across sessions
- **Acts** — runs tools, writes code, manages files, browses the web
- **Grows** — installs skills, tracks goals, and surfaces patterns over time
- **Shows up** — runs scheduled jobs and can message you proactively

## Quick Start

```bash
# Clone and build
git clone https://github.com/ishan-parihar/operant.git
cd operant
cargo build --release

# Set your API key
export ANTHROPIC_API_KEY=sk-ant-...  # or OPENAI_API_KEY=sk-...

# Start chatting
./target/release/operant chat
```

That's it. The first run will guide you through a quick setup (provider,
model, API key). Type `/help` in the TUI to see everything operant can do.

## What Can It Do?

- **Code** — write, review, debug, refactor across your entire project
- **Research** — search the web, read pages, synthesize findings
- **Autonomous** — runs as a disciplined developer: plans, implements, validates, pushes
- **Remember** — every conversation feeds a graph memory that grows smarter
- **Connect** — talk to operant via Telegram, Discord, Slack, WhatsApp, Email
- **Learn** — install skills (reusable prompt+tool bundles) or draft your own
- **Debug** — headless TUI simulator for autonomous testing and CI regression

## Architecture

Operant is written entirely in Rust, focusing on **Tolerant Parsing**,
**Deterministic Execution**, and **Autonomous Workspace Management**. It
implements a streaming-first ReAct (Reason-Act) loop that stays stable
even when LLM outputs are malformed.

---

## 🚩 The Problem: The "Loop Fragility" Gap
Building a reliable agentic loop (Think $\to$ Act $\to$ Observe) is deceptively difficult in production. Standard parsers typically crash when LLMs produce malformed tool calls—such as unclosed XML tags or invalid JSON—leading to complete loop failure. Furthermore, the "Wait" bottleneck—waiting for a full LLM response before executing a tool—introduces massive latency. Finally, running agents in "Autonomous Mode" without strict validation gates often results in catastrophic workspace corruption when an agent misinterprets a command.

## 💡 The Solution: A Hardened Orchestration Substrate

`Operant` solves these problems through five core engineering pillars:

### 1. Streaming-First "Tolerant" Parsing
Instead of waiting for a complete response, `Operant` uses a custom state-machine parser that detects tool calls **incrementally**. If an LLM produces a `<tool_call>` but the connection drops before the closing tag, the parser can still recover the intent and initiate execution.

### 2. The "Validated" Autonomous Loop
The `operant autonomous` mode transforms the agent into a disciplined developer. It follows a strict **Plan $\to$ Implement $\to$ Validate $\to$ Push** cycle:
- **Source of Truth**: Reads a `TODO.md` to determine the next task.
- **Guardrails**: The agent *cannot* push to Git unless the configured `test_command` (e.g., `cargo test`) returns a success exit code.
- **State Persistence**: Tracks progress in `autonomous-status.toml` to survive process restarts and avoid redundant work.

### 3. Cross-Platform Gateway Architecture
To ensure total conversation continuity, `Operant` implements a centralized gateway process. This architecture bridges the core agent loop to multiple platforms (Telegram, Discord, Slack, etc.) through a single interface, allowing a user to start a task on the CLI and receive a completion notification via messaging without losing any state.

### 4. Kanban-Based Work Orchestration
For complex, multi-agent collaboration, `Operant` provides a durable, SQLite-backed Kanban board. This allows tasks to be dispatched, claimed, and tracked across multiple profiles and worker agents, transforming the agent from a simple chatbot into a coordinated workforce.

### 5. Scheduled Automations (Cron)
Built-in cron scheduling allows for high-reliability, unattended tasks. From daily reports to nightly backups, the system handles natural-language schedules and delivers results to any connected messaging platform.

---

## Engineering Highlights

### Autonomous Coding Mode
Operant can work as a disciplined developer. `operant autonomous` reads a `TODO.md` task ledger, implements changes, runs validation tests, and only pushes to git when tests pass. State persists across restarts via `autonomous-status.toml`, so interrupted work resumes cleanly.

### Long-Term Memory
Every conversation feeds a graph memory (TDG) that self-organizes across sessions. Durable facts are extracted and injected into the agent's system prompt as `<long_term_memory>` context, so the agent remembers your patterns, preferences, and project conventions.

### Context Management
Tiered eviction (tool results → reasoning → conversation) with a decay curve keeps the context window healthy. Auto-compression kicks in when the window overflows, so long sessions don't crash.

### Zero-Cost Orchestration in Rust
By leveraging `async-trait` and `Tokio`, I built a high-concurrency orchestration substrate with minimal runtime overhead. The system uses a decoupled `ToolRegistry` to dynamically generate JSON Schemas at runtime, ensuring compatibility with the Model Context Protocol (MCP). The final binary is LTO-optimized and stripped, providing a production-grade core that remains stable and performant under heavy agentic load.

### High-Fidelity TUI (Terminal User Interface)
Designed for "Human-in-the-Loop" monitoring, the Ratatui-based interface provides:
- **Reasoning Rails**: Block-style rendering of model thinking.
- **Activity Feed**: Compact, real-time logs of tool execution.
- **Workspace Panes**: A multi-pane layout that allows the user to monitor the conversation and the agent's actions simultaneously.
- **Headless simulator**: `operant tui debug simulate` drives the real TUI loop against a test backend, injects mock agent events, and asserts on state and rendered screen — enabling autonomous debug loops and CI regression tests. See [docs/tui-debugging.md](docs/tui-debugging.md).

---

## 🌌 Potentialities & Future Scope

`Operant` is a foundation for **Autonomous Engineering Infrastructure**:

- **Multi-Agent Swarms**: Evolving the `delegate_to_sub_agent` tool into a full-fledged orchestration layer where multiple Hermes instances collaborate.
- **Self-Optimizing Prompts**: Using the execution telemetry to automatically refine the system prompts based on which "reasoning paths" led to the fastest success.
- **Hardware-Level Integration**: Moving beyond shell tools to native Rust drivers for direct hardware/OS control in specialized environments.

---

## 🚀 Quick Start

### Installation
```bash
# Build the optimized release
cargo build --release

# Install the CLI
cargo install --path crates/operant-cli
```

### Usage Example
```bash
# Start the interactive TUI
operant chat

# Execute a one-shot task
operant run --query "Refactor the error handling in src/main.rs"

# Launch the 24/7 autonomous developer
operant autonomous
```

## 🛠 Tech Stack
- **Core**: Rust
- **Async**: Tokio
- **UI**: Ratatui
- **Client**: Reqwest / OpenAI API
- **Config**: TOML

---
---

Developed by [Ishan Parihar](https://github.com/ishan-parihar) as a high-performance port of the Hermes-Agent orchestration logic.

If you find this project useful, [consider supporting its development](https://rzp.io/rzp/ishan-parihar) ☕
