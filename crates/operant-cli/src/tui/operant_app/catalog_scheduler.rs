// Vendored from jcode (crates/operant-base/src/provider/mod.rs + provider/
// catalog_scheduler.rs), MIT License, Copyright (c) 2025 Jeremy Huang.
// [port-decision] scheduler exeides: the stale-refresh loop
// (profile_catalog_cache_needs_refresh, sweep_stale_profile_catalogs,
// ensure_started) operates on operant-base's provider app-core (provider_catalog,
// local profile metadata, cached live models, openrouter scheduling), which is
// not ported at batch-3 — operant's provider core lives elsewhere in
// crates/operant-core. What remains is exactly the one public leaf bus.rs calls
// (operant_app/bus.rs:157), verbatim, with its single dependency installed
// verbatim from the source. Deleting the loop would have silently stopped the
// "schedule refresh" arm; the gate pattern (`#[cfg(any())]` on the internals)
// instead leaves their shapes recoverable when W8 ports the provider surface.
// The port is direct outwards from operant-base/src/provider/catalog_scheduler.rs
// and :430-446 of operant-base/src/provider/mod.rs; any earlier intermediate
// layering (a partial stickers mod.rs port of this module) is flowed through.

// The catalog-generation dynamic counter. Owned here instead of the upstream
// provider module because the rest of provider/mod.rs (the block that generated
// the static's home) stays unported in batch-3; the verbatim value and character
// of the leaf that actually needs it are preserved.
static CATALOG_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Upstream operant-base/src/provider/mod.rs:433-446 (signature and body verbatim).
/// Rotated whenever a catalog refresh has happened so memoized routes know to
/// rebuild. The displayed code path at ui_header.rs reads this generation via
/// `catalog_scheduler::last_catalog_refresh_generation`. Producing a fresh one
/// is the only render-time-facing contract of this module in batch-3.
pub fn bump_catalog_generation() {
    CATALOG_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The last catalog bump's generation — used by tui_fns.rs to key its model-name
/// cache. Same upstream shape as bump_catalog_generation.
pub fn last_catalog_refresh_generation() -> u64 {
    CATALOG_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}
