# operant-config

Configuration schema, secrets, and related types for Operant.

## Purpose

Holds the config schema (`src/schema/`, with the full `Config` type), secret storage helpers (`src/secrets.rs`), security policy and autonomy types (`src/policy.rs`, `src/autonomy.rs`), the `[providers]` section types (`src/providers.rs`), and the derive-support traits. `Cargo.toml` pulls in `chacha20poly1305` and `sha2`; read `src/secrets.rs` for how they are used.

## Configuration

- Cargo features: `default = ["schema-export"]`, which enables `schemars`. `channel-nostr` and `voice-wake` are empty feature flags.
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
