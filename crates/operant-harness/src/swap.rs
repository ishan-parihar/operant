//! Phase 4 — WASM hot-swap (kernel-level protocol).
//!
//! The kernel-level swap contract that WASM/file-watch swaps must honor.
//! The actual Extism host + signature verification live in `operant-plugins`
//! (untouched here); this module is the protocol that wraps them.
//!
//! ## Protocol
//!
//! 1. **Instantiate candidate** (host-side: load .wasm, parse manifest,
//!    build Extism plugin). Failures here abort; the previous instance
//!    is untouched.
//! 2. **Validate candidate** (host-side: run validation export, check
//!    capabilities against the mount allowlist). Failures here dispose
//!    the candidate.
//! 3. **Atomic slot swap** — the kernel replaces the provider entry's
//!    generation counter AND the new provider's claims take effect for
//!    late-binding on the next mount/rescan. Pending dependents
//!    satisfying requirements still see the old generation until the
//!    next rescan (Cordis fiber-ABA safe).
//! 4. **Unwind old** — call old provider's effect-undo chain. Old's
//!    effects die AFTER new's effects are live, so concurrent dispatch
//!    always sees a valid state.
//! 5. On ANY failure in steps 1-3, the previous instance is restored and
//!    the candidate is disposed. The generation counter is bumped on
//!    success only.
//!
//! ## Statelessness contract
//!
//! WASM providers must be stateless across swaps — durable state lives
//! host-side (in the seam slots, registries, etc.). The swap protocol
//! does NOT preserve any in-plugin state.

use std::sync::atomic::{AtomicU64, Ordering};

/// One generation counter per swap-managed provider slot. Cordis-parity
/// ABA guard: any swap bumps generation; readers that captured the prior
/// generation detect "is this still the same instance?" by comparing.
#[derive(Debug, Default)]
pub struct SwapGeneration {
    generation: AtomicU64,
}

impl SwapGeneration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current generation. Starts at 0.
    pub fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Bump the generation (caller's success path). Returns the new value.
    pub fn bump(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// True when `observed` matches the current generation. ABA-safe:
    /// concurrent readers that snapshotted before a swap will see `false`.
    pub fn matches(&self, observed: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == observed
    }
}

/// Outcome of a swap attempt. The kernel uses this to decide whether to
/// commit the candidate, restore the previous, or log and continue.
#[derive(Debug)]
pub enum SwapOutcome {
    /// Candidate was instantiated, validated, and committed. Previous is
    /// wound down. Generation bumped to `new_generation`.
    Committed { new_generation: u64 },
    /// Candidate failed at one of the validation steps. Previous still
    /// active; generation unchanged.
    Rejected { reason: String },
    /// Candidate passed validation but the commit failed (e.g. claim
    /// conflict). Previous is restored; generation unchanged.
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn generation_starts_at_zero() {
        let g = SwapGeneration::new();
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn generation_bumps_monotonically() {
        let g = SwapGeneration::new();
        let a = g.bump();
        let b = g.bump();
        let c = g.bump();
        assert!(a < b);
        assert!(b < c);
        assert_eq!(g.current(), c);
    }

    #[test]
    fn generation_matches_aba_safely() {
        let g = SwapGeneration::new();
        let observed = g.current();
        g.bump();
        g.bump();
        // observed was captured at gen 0; current is 2 — does not match
        assert!(!g.matches(observed));
        // matching the new value does match
        assert!(g.matches(g.current()));
    }
}
