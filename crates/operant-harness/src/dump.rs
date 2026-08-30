//! Plan 016: the serializable `dump()` shape.
//!
//! The kernel exposes its state as a `HarnessSnapshot` for `dump()`
//! output and the Phase 3 composition layer to read back. The shape
//! is deliberately stable so a logged dump today can be diffed
//! against a dump tomorrow, and so the planned `operant architecture
//! dump` CLI subcommand has a fixed contract.

use crate::provider::Source;
use serde::{Deserialize, Serialize};

/// The state of a single provider inside the harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderState {
    pub id: String,
    /// `Pending | Activating | Active | Unloading | Disposed | Failed`
    pub state: String,
    pub generation: u64,
    pub source: Source,
    pub provides: Vec<String>,
    pub requires: Vec<String>,
}

/// Top-level snapshot returned by `Harness::dump()`. Stable across
/// kernel versions; the only forward-compat risk is the addition of
/// new optional fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSnapshot {
    pub providers: Vec<ProviderState>,
    /// `claim-key -> provider-id` for every active claim. Keys not
    /// in this map are unresolved.
    pub claims: std::collections::BTreeMap<String, String>,
    /// Monotonic counter; bumped on every successful `mount` and
    /// `swap`. Cheap oracle for "did anything change between two
    /// dumps" without diffing the whole state.
    pub generation: u64,
}

impl HarnessSnapshot {
    pub fn empty() -> Self {
        Self::default()
    }

    /// True iff the snapshot has no providers AND no claims. Used by
    /// the dark-merge acceptance test.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty() && self.claims.is_empty()
    }
}
