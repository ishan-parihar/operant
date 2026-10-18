# operant-providers

LLM provider implementations for Operant, behind a factory keyed by provider name.

## Purpose

Each model inference backend implements the `Provider` trait in `src/traits.rs` and is registered in the factory by a canonical string key such as `openai`, `anthropic`, `ollama`, or `gemini` (`src/lib.rs`). Provider modules include `anthropic.rs`, `openai.rs`, `gemini.rs`, `ollama.rs`, `bedrock.rs`, `azure_openai.rs`, `openrouter.rs`, and `llamacpp.rs`, among others under `src/`.

## Configuration

- No Cargo features. `Cargo.toml` declares dependencies only.
- Provider profiles and the cross-provider fallback chain are set in the `[providers]` section (see the comment in `operant-core/Cargo.toml`).
- Provider aliases are resolved internally so user-facing keys stay stable (`src/lib.rs`).

## Extension points

- Implement `Provider` in a new submodule and register it in `create_provider_with_url` (`src/lib.rs`). `AGENTS.md` section 7.1 has the full change playbook.
- Route across providers with `create_routed_provider` (`src/lib.rs`).

## Limitations

- The crate docs do not list per-provider limits or supported models; read the module for each provider.

## Model experience

Calls go through the `Provider` trait in `src/traits.rs`. `ReliableProvider` (`src/reliable.rs`) wraps a provider chain with fallback and automatic retry, and `create_routed_provider` routes requests across providers (`src/lib.rs`).
