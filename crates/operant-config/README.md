# operant-config

Configuration schema and related types for Operant.

## Purpose

Holds the config schema (`src/schema/`: `core`, `cost`, `media`,
`memory_store`, `platform_cfg`, `proxy`, `helpers`), the security policy
(`src/policy.rs`), the `[providers]` section types (`src/providers.rs`) and the
runtime config loaders.

This crate has ONE consumer: `operant-core`.

CORRECTED iter-2510, RE-CORRECTED iter-2515: the first pass removed a
`src/secrets.rs` paragraph (the file never existed; `chacha20poly1305` and
`sha2` were among 17 deps deleted with zero references). The replacement still
carried three stale blocks, all disproved by grep against this tree:

- **"derive `Configurable` from `operant-macros`"** — neither exists.
  `crates/operant-macros` is absent and the workspace has no such member, and
  there is no `Configurable` trait to derive (0 `derive(`/`impl`/`trait` sites
  anywhere; the two bare-word matches a plain grep finds are the ordinary
  English adjective in unrelated comments). `HasPropKind`, `ChannelConfig` and
  `OnboardUi` have no hits at all. The parallel config surface that used them
  was retired at iters 1138-1141 (RT-141).
- **"`src/traits.rs`"** — the file does not exist.
- **"`operant-memory`, `operant-providers`, `operant-tools` depend on this
  crate"** — all three directories are absent. Only `operant-core` names it.
- **"the `config` and `security` shim modules at the bottom of `src/lib.rs`"** —
  no such `pub mod` declarations exist.

A README that documents an extension mechanism a caller cannot use is worse
than a short accurate one. Add a config section by adding a type under
`src/schema/` and wiring it into `schema/mod.rs`.

## Module map

- `lib.rs` -- crate root, `#![deny(missing_docs)]`, the default
  `DEFAULT_AGENTMEMORY_VERSION` pin, and the config loaders.
- `policy.rs` -- the security policy types (approval modes, tool policy).
- `providers.rs` -- the `[providers]` section types.
- `schema/` -- the configuration schema, one file per domain: `core`,
  `cost`, `media`, `memory_store`, `platform_cfg`, `proxy`, plus `helpers`
  and `mod`.

## Configuration

- Cargo features: `default = ["schema-export"]`, which enables `schemars`.
  (`voice-wake` was an empty flag that gated nothing; removed with the
  dead-feature sweep.)
- `DEFAULT_AGENTMEMORY_VERSION` (`src/lib.rs`) is the pinned `@agentmemory`
  package version. It is overridable with `[memory] agentmemory_version` or
  the `AGENTMEMORY_VERSION` environment variable.

## Limitations

- `src/lib.rs` enables `#![deny(missing_docs)]`, but the crate-level doc is a
  single line.
- Schema export is only available with the `schema-export` feature, which is on
  by default.
- The crate carries orphan dependencies that no source file references. They
  are buildable but should be pruned (see the iter-2510 dep sweep).

## Model experience

None directly. This crate exposes settings and policy types and no model-facing
surface.