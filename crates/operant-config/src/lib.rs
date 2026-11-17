#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Configuration types for Operant.
//!
//! (p2c, 2026-11-16) This crate held ~2,400 lines of modules with no caller
//! outside it: a parallel security-policy surface, a secret store, a provider
//! alias table, derive-support traits and a property-kind schema module. They
//! are deleted. What remains is what `operant_core::config::AppConfig`
//! actually composes.

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
