# Hermes-RS ⚡

**The Hardened Rust Substrate for Agentic Tool Orchestration.**

[![Rust 1.86+](https://img.shields.io/badge/rust-1.86%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT/Apache](https://img.shields.io/badge/License-MIT/Apache-yellow.svg)](LICENSE)
[![TUI](https://img.shields.io/badge/UI-Ratatui-blue.svg)](https://github.com/ratatui-org/ratatui)
[![Architecture](https://img.shields.io/badge/Architecture-ReAct-purple.svg)](https://arxiv.org/abs/2210.03629)

`Hermes-RS` is a high-performance, production-grade implementation of the Hermes-Agent orchestration loop. It provides the critical infrastructure needed for LLMs to move from "text generation" to "system operation" by implementing a robust, streaming-first ReAct (Reason-Act) loop.

Written entirely in Rust, `Hermes-RS` focuses on **Tolerant Parsing**, **Deterministic Execution**, and **Autonomous Workspace Management**, ensuring that agentic loops remain stable even when LLM outputs are malformed or inconsistent.

---

## 🚩 The Problem: The "Loop Fragility" Gap

Building a reliable agentic loop (Think $\to$ Act $\to$ Observe) is deceptively difficult in production:
1. **Malformed Tool Calls**: LLMs often fail to close XML tags or produce invalid JSON, causing standard parsers to crash the entire loop.
2. **The "Wait" Bottleneck**: Waiting for a full LLM response before executing a tool introduces massive latency.
3. **Context Drift**: As loops iterate, the "observational" data can overwhelm the prompt, leading to "lost-in-the-middle" reasoning.
4. **Autonomous Risk**: Running an agent in "Autonomous Mode" (24/7) without a strict validation gate leads to catastrophic workspace corruption.

## 💡 The Solution: A Hardened Orchestration Substrate

`Hermes-RS` solves these problems through four core engineering pillars:

### 1. Streaming-First "Tolerant" Parsing
Instead of waiting for a complete response, `Hermes-RS` uses a custom state-machine parser that detects tool calls **incrementally**. If an LLM produces a ` <tool_call> ` but the connection drops before the closing tag, the parser can still recover the intent and initiate execution.

### 2. The "Validated" Autonomous Loop
The `hermes autonomous` mode transforms the agent into a disciplined developer. It follows a strict **Plan $\to$ Implement $\to$ Validate $\to$ Push** cycle:
- **Source of Truth**: Reads a `TODO.md` to determine the next task.
- **Guardrails**: The agent *cannot* push to Git unless the configured `test_command` (e.g., `cargo test`) returns a success exit code.
- **State Persistence**: Tracks progress in `autonomous-status.toml` to survive process restarts and avoid redundant work.

### 3. High-Fidelity TUI (Terminal User Interface)
Designed for "Human-in-the-Loop" monitoring, the Ratatui-based interface provides:
- **Reasoning Rails**: Block-style rendering of model thinking.
- **Activity Feed**: Compact, real-time logs of tool execution.
- **Workspace Panes**: A multi-pane layout that allows the user to monitor the conversation and the agent's actions simultaneously.

### 4. Dynamic Tool Registry & MCP
Implements a decoupled `ToolRegistry` that can dynamically load tools and generate JSON Schemas at runtime, making it compatible with the **Model Context Protocol (MCP)** for extended capability discovery.

---

## ✨ Engineering Highlights

### 🏗 System Architecture
- **ReAct Loop**: A clean implementation of the Reason-Act cycle with early-exit and self-healing prompts.
- **Zero-Cost Abstractions**: Leverages Rust's `async-trait` and `Tokio` for highly concurrent tool execution without runtime overhead.
- **Unified Config**: A single TOML-based configuration model shared across the CLI and the Core library.
- **Shared State**: Use of `Arc` and `Mutex` for thread-safe access to the Tool Registry and Session State.

### 🛠 Technical Specifications
- **Language**: Rust (Edition 2021)
- **Async Runtime**: Tokio
- **TUI**: Ratatui / Crossterm
- **Networking**: Reqwest (Rustls-TLS)
- **Serialization**: Serde / Schemars

---

## 🌌 Potentialities & Future Scope

`Hermes-RS` is a foundation for **Autonomous Engineering Infrastructure**:

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
cargo install --path crates/hermes-cli
```

### Usage Example
```bash
# Start the interactive TUI
hermes chat

# Execute a one-shot task
hermes run --query "Refactor the error handling in src/main.rs"

# Launch the 24/7 autonomous developer
hermes autonomous
```

## 🛠 Tech Stack
- **Core**: Rust
- **Async**: Tokio
- **UI**: Ratatui
- **Client**: Reqwest / OpenAI API
- **Config**: TOML

---
Developed by [Ishan Parihar](https://github.com/ishan-parihar) as a high-performance port of the Hermes-Agent orchestration logic.
