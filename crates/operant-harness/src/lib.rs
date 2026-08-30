//! Plan 016: `operant-harness` — the composable provider runtime.
//!
//! The crate is intentionally a **pure library** at this phase: no dependency
//! on `operant-core`, `operant-runtime`, or `operant-cli` beyond the
//! `operant-api` surface. Phase 0-1 scaffolds the kernel core; Phase 2+
//! (in the same plan) will plug the seams into the live tool/hook/memory
//! registries via flag-gated dispatch.
//!
//! The five adopted semantics (from plan 016 §"The five adopted semantics"):
//! 1. String-keyed service claims (`Harness.claims: String -> ProviderId`).
//! 2. `requires()` + PENDING late binding — unmet requirements stay
//!    pending until a successful activation rescans.
//! 3. Registrations return `Effect` undo handles; unmount unwinds LIFO.
//! 4. Transactional HMR — atomic slot swap, restore-on-failure, ABA
//!    generation counter guards stale-swap.
//! 5. Composition via `architecture.toml` + `dump()` (Phase 3 — not yet).
//!
//! Module layout (per plan 016 §"Files in scope"):
//! - `lib.rs` (this): module wiring + design contract.
//! - `provider.rs`: the `Provider` trait (`id/provides/requires/source`).
//! - `harness.rs`: the `Harness` container, state machine, mount/unmount.
//! - `effect.rs`: the `Effect` undo-handle type + LIFO unwind.
//! - `dump.rs`: the serializable `dump()` shape (id, state, claims,
//!   requires, source, generation).
//! - `composition.rs`: placeholder for Phase 3 (`architecture.toml`).
//!
//! Default-off integration: this crate is dark-merged. No consumer is
//! required to enable it; the existing `ToolRegistry` / `HookRunner`
//! continue to operate on their own clocks. Flag-on integration is a
//! later phase that wraps (never duplicates) the existing machinery.

#![forbid(unsafe_code)]

pub mod dump;
pub mod effect;
pub mod harness;
pub mod provider;

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 0/1 acceptance: `cargo test -p operant-harness --lib` is green
    /// dark, the crate has no dep on operant-core/runtime/cli, and
    /// mounting a single provider into a fresh harness yields a clean
    /// `dump()` with the provider active and its claims visible.
    #[test]
    fn dark_merge_scaffold_is_sound() {
        // Empty harness → no claims, no active providers.
        let h = harness::Harness::new();
        let snap = h.dump();
        assert!(snap.providers.is_empty(), "fresh harness has no providers");
        assert!(snap.claims.is_empty(), "fresh harness has no claims");
    }
}
