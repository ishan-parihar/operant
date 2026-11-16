# operant-config

Configuration schema and related types for Operant.

## Purpose

Holds the config schema (`src/schema/`), security policy (`src/policy.rs`), the `[providers]` section types (`src/providers.rs`) and the runtime config loaders. Plain config structs plus one proxy helper.

CORRECTED iter-2510: this paragraph used to cite a `src/secrets.rs` and name `chacha20poly1305`/`sha2` as its deps. Neither existed - the file is absent and both crates had zero references, among 17 deps deleted. A README naming a module that was never written is worse than none.

## Configuration

- Cargo features: `default = ["schema-export"]`, which enables `schemars`. (`voice-wake` was an empty flag that gated nothing; removed with the dead-feature sweep.)
- `DEFAULT_AGENTMEMORY_VERSION` (`src/lib.rs`) is the pinned `@agentmemory` package version. It is overridable with `[memory] agentmemory_version` or the `AGENTMEMORY_VERSION` environment variable.

## Extension points

- Add a config section as a type in `src/schema/` and derive `Configurable` from `operant-macros`.
- Derive-support traits (`HasPropKind`, `ChannelConfig`, `OnboardUi`) live in `src/traits.rs`.
- The `config` and `security` shim modules at the bottom of `src/lib.rs` exist so that derive-generated paths resolve.

## Limitations

- `src/lib.rs` enables `#![deny(missing_docs)]`, but the crate-level doc is a single line.
- Schema export is only available with the `schema-export` feature, which is on by default.

## Model experience

None directly. This crate exposes settings and policy types and no model-facing surface. Crates that depend on it (per `Cargo.toml`) are `operant-core`, `operant-memory`, `operant-providers`, and `operant-tools`.
