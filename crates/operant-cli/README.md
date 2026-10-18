# operant-cli

The `operant` binary: command-line and terminal UI for the Operant agent framework.

## Purpose

Builds the `operant` binary (`src/main.rs`, declared as `[[bin]]` in `Cargo.toml`). It contains the `cmd_*.rs` subcommand handlers, the ratatui TUI under `src/tui/`, autonomous coding mode in `src/autonomous.rs`, and the dashboard backend in `src/dashboard_server.rs` (per the `AGENTS.md` architecture overview). The crate doc in `src/main.rs` is only "Operant-RS CLI".

## Configuration

- Cargo features: `default = ["agent-runtime"]`. `agent-runtime` pulls in `operant-tools`. Others: `browser-native`, `plugins-wasm`, `memory-postgres`, `rag-pdf`, `anthropic`, and the `ci-all` meta-feature.
- Runtime config is TOML-first and shared through `crates/operant-core/src/config.rs` (see `AGENTS.md`).
- Config field changes must also update `operant.example.toml` at the repo root (see `AGENTS.md`).

## Extension points

- New subcommand: add a `cmd_*.rs` handler and wire it into the Clap enum in `src/main.rs`.
- Traits in `src/commands.rs` (`CommandHandler`) and `src/provider.rs` (`ProviderProfile`).
- Gateway platforms are registered in `platform_registry()` in `src/gateway_runner.rs`.

## Limitations

- The crate docs do not describe the command surface; read the Clap definitions in `src/main.rs`.
- Gateway mode: per `AGENTS.md`, only the Telegram, Discord and Slack adapters are fully implemented; the webhook adapter is a stub and WhatsApp, Email and SMS are config-only.

## Model experience

The TUI shows Conversation, Reasoning, Activity, and management panels for MCP, Skills, and Behavior (`AGENTS.md`, workspace view). These panels are how a user watches model turns; the crate has no separate model-facing API.
