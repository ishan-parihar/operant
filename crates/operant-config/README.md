# operant-config

Configuration schema, secrets, and related types for Operant.

## Purpose

Holds the config schema (`src/schema/`), security policy (`src/policy.rs`), the `[providers]` section types (`src/providers.rs`) and the runtime config loaders. Plain config structs plus one proxy helper.

CORRECTED 2026-11-16 (iter-2510): this paragraph used to claim a `src/secrets.rs` with "secret storage helpers" and said `Cargo.toml` pulls in `chacha20poly1305` and `sha2` for it. Neither was true - `src/secrets.rs` does not exist, and both crates had ZERO references in `src/` (verified by grep before removal). They were among 17 dependencies this crate declared and never read, now deleted. A README that names a file and a dependency that do not exist is worse than no README: it sends the next reader looking for a module that was never written.

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
