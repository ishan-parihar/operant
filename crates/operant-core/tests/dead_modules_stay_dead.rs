#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Two modules were deleted after their callers were proven absent, not assumed.
//!
//! `retrieval.rs` (1,068 lines, 6 public functions) and `voice.rs` (1,516 lines,
//! 25 public functions) had **zero** callers anywhere in the workspace, tests
//! included. The proof that matters is module-qualified: no site outside those
//! two files names `operant_core::retrieval::` or `voice::`, and no `use
//! crate::retrieval` / `self::voice` internal path exists either. The first scan
//! attempted at this question searched bare function names and reported the
//! modules alive — `new` matches 7,779 sites across the workspace, `as_str`
//! 1,429 — a confident false positive that would have shipped. Only the
//! qualified-path method was trusted.
//!
//! The acid test, not the search, settled it: with both `pub mod` declarations
//! removed from `lib.rs`, `cargo check -p operant-core --lib` finished clean.
//! `lib.rs` was restored immediately after, and the deletion was then performed
//! for real. See
//! `docs/audit/2026-10-27-ponytail-audit-memory-compat-and-dead-code.md`.
//!
//! Unlike the retired memory-provider config fields — which must stay precisely
//! so their "this key is unused, remove it" warning keeps firing — nothing
//! depended on either module, which is what made this a clean deletion rather
//! than a compat decision.
//!
//! This guard exists because a module this size being dead is invisible: nothing
//! fails when it accrues callers again, and 2,584 lines would ride back into the
//! tree unnoticed. A re-added declaration or a new caller fails the build.

#[test]
fn the_proven_dead_modules_stay_dead() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("operant-core lib.rs is readable");

    assert!(
        !lib_rs.contains("pub mod retrieval;"),
        "retrieval.rs (1,068 lines) was deleted after its callers were proven \
         absent and the crate compiled clean without it — do not revive it"
    );
    assert!(
        !lib_rs.contains("pub mod voice;"),
        "voice.rs (1,516 lines) was deleted after its callers were proven \
         absent and the crate compiled clean without it — do not revive it"
    );
}
