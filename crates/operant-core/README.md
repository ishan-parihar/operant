# operant-core

Core library for Operant: the ReAct agent loop, tools, memory, gateway, and MCP client.

## Purpose

The library behind the `operant` CLI. It contains no CLI or TUI code (per `AGENTS.md`). The crate doc in `src/lib.rs` lists its features: streaming tool-call detection, built-in tools, re-prompting the model on tool failure, context compression, file-backed memory, trajectory export, a multi-platform gateway, an MCP client, and skill discovery.

## Configuration

- Cargo features: `anthropic`. Default is empty.
- Runtime config is TOML-first and read from `src/config.rs` (`AppConfig`, `BehaviorSettings`, `ToolSettings`), per `AGENTS.md`.
- Provider profiles and the cross-provider fallback chain come from the `[providers]` section (see the comment in `Cargo.toml`).
- Config field changes must also update `operant.example.toml` at the repo root.

## Extension points

- Built-in tools: register them in `src/tools/builtin.rs` (`register_builtin_tools()`).
- Traits: `OperantTool` (`src/tools.rs`), `ModelClient` (`src/agent/model_client.rs`), `MemoryProvider` (`src/memory_provider.rs`), `PlatformAdapter` (`src/gateway/types.rs`), `BrowserProvider` (`src/browser_provider.rs`).
- Model adapters for OpenAI and Anthropic live under `src/agent/clients/`.

## Limitations

- Gateway: per `AGENTS.md`, the Telegram, Discord and Slack adapters are implemented. The webhook adapter is a stub, and WhatsApp, Email and SMS are config-only.
- The crate doc gives a feature summary and does not say which behaviours are behind Cargo flags beyond `anthropic`.

## Model experience

- Streaming-first: tool calls are detected incrementally from the model's output stream (`src/lib.rs`).
- Self-healing: on tool failure the model is re-prompted with the error context (`src/lib.rs`).
- Long conversations are compressed automatically (`src/lib.rs`; the mechanism is in `src/context_management.rs`).
