#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Configuration schema, secrets, and related types for Operant.

/// Pinned `@agentmemory` npm package version used by every spawn site
/// (auto-spawn of the memory server in `operant-core`, the MCP server entries
/// here and in `operant-core`'s AppConfig path). Pinning keeps a cold boot
/// from fetching whatever npm serves at that moment and running it with the
/// operator's privileges; bump deliberately when you want the new version.
/// Overridable per deployment via `[memory] agentmemory_version` (AppConfig)
/// or `AGENTMEMORY_VERSION` (schema/env world).
pub const DEFAULT_AGENTMEMORY_VERSION: &str = "0.9.29";

/// Autonomy levels and agent control settings.
pub mod autonomy;
/// Property helpers shared by the derive-generated code.
/// Security policy, autonomy, and tool-execution gating types.
pub mod policy;
pub mod provider_aliases;
/// Provider section (`[providers]`) types.
pub mod providers;
/// The full config schema (`Config` and all section types).
pub mod schema;
/// Secret encryption / storage helpers.
pub mod secrets;
/// Derive-support traits (`HasPropKind`, `ChannelConfig`, `OnboardUi`, ...).
pub mod traits;
