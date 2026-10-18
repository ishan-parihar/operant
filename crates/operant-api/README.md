# operant-api

Trait definitions and shared types for Operant, the interface layer that other crates depend on.

## Purpose

Defines the abstractions the other Operant subsystems build on: `Provider`, `Channel`, `Tool`, `Memory`, `Observer`, `RuntimeAdapter`, `Peripheral` and `Vad`, one per module (see `src/lib.rs`). Per its crate docs it contains no implementations and no heavy dependencies.

## Configuration

None. `Cargo.toml` declares no features, and `src/lib.rs` describes no runtime settings.

## Extension points

Implement one of the public traits in a downstream crate:

- `provider::Provider` (LLM inference backends), `src/provider.rs`
- `channel::Channel` (messaging platforms), `src/channel.rs`
- `tool::Tool` (agent-callable capabilities), `src/tool.rs`
- `memory_traits::Memory` (conversation memory backends), `src/memory_traits.rs`
- `observability_traits::Observer` (metrics and tracing), `src/observability_traits.rs`
- `runtime_traits::RuntimeAdapter` (execution environments), `src/runtime_traits.rs`
- `peripherals_traits::Peripheral` (hardware boards), `src/peripherals_traits.rs`
- `vad::Vad` (voice activity detection), `src/vad.rs`

## Limitations

The crate is trait-only, so runtime behaviour lives in the implementing crates. The crate docs do not state error or lifecycle rules for the traits; read the trait module named above for the contract.

## Model experience

The `Provider` and `Tool` traits are the seams through which model backends and agent-callable tools plug in (`src/lib.rs`). The crate docs do not describe what the model sees in a prompt; read `src/tool.rs` for the tool interface.
