#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Configuration types for Operant.
//!
//! (p2c, 2026-11-16) This crate held ~2,400 lines of modules with no caller
//! outside it: a parallel security-policy surface, a secret store, a provider
//! alias table, derive-support traits and a property-kind schema module. They
//! are deleted. What remains is what `operant_core::config::AppConfig`
//! actually composes.

/// Pinned `@agentmemory` npm package version used by every spawn site
/// (auto-spawn of the memory server in `operant-core`, the MCP server entries
/// here and in `operant-core`'s AppConfig path). Pinning keeps a cold boot
/// from fetching whatever npm serves at that moment and running it with the
/// operator's privileges; bump deliberately when you want the new version.
/// Overridable per deployment via `[memory] agentmemory_version` (AppConfig)
/// or `AGENTMEMORY_VERSION` (schema/env world).
pub const DEFAULT_AGENTMEMORY_VERSION: &str = "0.9.29";

/// The shell-command splitter the Holon command fence applies (RT-454).
///
/// (p2c, 2026-11-16) This crate used to also export `autonomy`, `traits`,
/// `secrets`, `provider_aliases` and a `prop_kinds` schema module. None had a
/// caller outside this crate, and the `secrets`/`traits` names collided with
/// live types elsewhere (`operant_core::fs_secrets`, `memory_traits`), which
/// is how they read as live for months. All deleted.
pub mod policy;
/// Provider section (`[providers]`) types.
pub mod providers;
/// The config types the live `AppConfig` composes.
pub mod schema;
