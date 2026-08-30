//! Plan 016: the `Provider` trait and its source kinds.
//!
//! Every composable unit in the harness declares:
//! - `id` (stable across the boot, used for ABA guard on swap),
//! - `provides` (the string-keyed claim slots the provider occupies),
//! - `requires` (the slots the provider needs; unmet = PENDING),
//! - `source` (Native | Wasm | ConfigRow | Pool — what built this provider),
//! - `apply(&mut self, ctx)` (the side effect performed on activation;
//!   it returns an `Effect` undo handle).
//!
//! The trait is intentionally object-safe and synchronous — the kernel
//! wraps async work in a blocking closure stored in the `Effect` undo
//! handle (Phase 0/1; the async-aware variant is Phase 2+).

use crate::effect::Effect;
use std::fmt;

/// Where a `Provider` came from. Influences the swap/refresh policy
/// (Phase 4+ for the WASM hot-swap path; the kernel uses this only for
/// `dump()` introspection in Phase 0/1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Built into the binary (e.g. the family-level built-in tool sets).
    Native,
    /// Loaded from a compiled WASM module (Phase 4+).
    Wasm { path: String },
    /// Materialized from a `architecture.toml` row — pure config, no code.
    ConfigRow,
    /// Compiled from a `~/.hermes/systems/<pool>/_pool.yaml` (Phase 6+).
    Pool { name: String },
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Native => f.write_str("native"),
            Source::Wasm { path } => write!(f, "wasm({path})"),
            Source::ConfigRow => f.write_str("config-row"),
            Source::Pool { name } => write!(f, "pool({name})"),
        }
    }
}

/// A composable unit. The kernel calls `apply` exactly once per
/// successful activation, then stores the returned `Effect` for LIFO
/// unwind at unmount time.
pub trait Provider: Send + Sync + fmt::Debug {
    /// Stable identifier for the provider (used as the `Effect` key +
    /// ABA generation counter). The same id MUST always represent the
    /// same logical provider across reloads — only a swap (Phase 4)
    /// may temporarily duplicate an id under a new generation.
    fn id(&self) -> &str;

    /// The string-keyed claim slots this provider occupies. Two
    /// providers may not declare overlapping `provides()` keys unless
    /// one of them has been `disabled` in `architecture.toml`.
    fn provides(&self) -> &[String];

    /// The claim slots this provider needs to be present before it
    /// activates. The kernel keeps the provider in `Pending` state
    /// until every key in this list is provided by some other active
    /// provider.
    fn requires(&self) -> &[String];

    /// Where the provider came from. Affects Phase 4+ swap policy.
    fn source(&self) -> &Source;

    /// Perform the side effect (register a tool, claim a hook, install
    /// a memory provider, etc.) and return the `Effect` undo handle.
    /// The kernel stores the handle and calls it in LIFO order on
    /// unmount. Returning `Effect::noop()` is legal for
    /// `ConfigRow` providers that just claim a slot.
    fn apply(&mut self) -> Effect;
}
