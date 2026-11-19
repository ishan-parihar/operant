#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The three pub fns deleted in A7 must stay deleted.
//!
//! `active_model` (`agent/provider_registry.rs`), `check_hardline`
//! (`approval.rs`) and `enable_toolset` (`tools.rs`) each had exactly one
//! reference in the whole workspace: their own definition. That is the shape
//! `scripts/audit-pub-fn-surface.py` reports, and it is the same shape RT-209
//! shipped twice -- a `pub` item in a library crate emits no `dead_code`
//! warning, so nothing fails while it rots.
//!
//! Each was proven dead the only way that counts: the `pub` was removed,
//! `cargo check -p operant-core --lib` finished clean, and the item was then
//! deleted for real. A name-based grep is not that proof -- it counts comments
//! and test callers, and it cannot see a method called through a trait.
//!
//! This guard exists because a re-added item is invisible: no compiler
//! diagnostic fires for a `pub fn` with no callers. The needles are the
//! qualified definition shape, not the bare name, so a mention in a comment or
//! a test cannot satisfy them.

/// A re-added definition of any of the three is the regression.
#[test]
fn the_three_dead_pub_fns_stay_deleted() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("AGENTS.md").exists() && p.join("crates").exists())
        .expect("workspace root (AGENTS.md + crates/)")
        .to_path_buf();

    // (file, needle) -- the DEFINITION shape, so a call or a comment cannot
    // satisfy it. Split with concat! so this test's own source cannot match.
    let dead: [(&str, &str); 3] = [
        (
            "crates/operant-core/src/agent/provider_registry.rs",
            concat!("pub fn ", "active", "_model"),
        ),
        (
            "crates/operant-core/src/approval.rs",
            concat!("pub fn ", "check", "_hardline"),
        ),
        (
            "crates/operant-core/src/tools.rs",
            concat!("pub async fn ", "enable", "_toolset"),
        ),
    ];

    for (rel, needle) in dead {
        let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(
            !text.contains(needle),
            "{rel} defines `{needle}` again. It was deleted in iter-2709 (A7) because nothing in production called it -- proven by removing the `pub` and watching `cargo check -p operant-core --lib` stay clean. If you re-added it because a caller now needs it, that is fine, but delete this guard's entry in the SAME commit and say what the caller is."
        );
    }
}

/// The auditor itself must keep existing and keep gating.
///
/// A7's whole deliverable is the DETECTOR; deleting the three functions is
/// secondary. If the script goes away, the next orphan ships invisibly.
#[test]
fn the_surface_auditor_still_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("AGENTS.md").exists() && p.join("crates").exists())
        .expect("workspace root")
        .to_path_buf();
    let script = root.join("scripts/audit-pub-fn-surface.py");
    let text = std::fs::read_to_string(&script)
        .unwrap_or_else(|e| panic!("scripts/audit-pub-fn-surface.py: {e}"));
    for needle in ["ACCOUNTED_FOR", "--check", "#[cfg(test)]"] {
        assert!(
            text.contains(needle),
            "scripts/audit-pub-fn-surface.py no longer mentions `{needle}`. That script is the only general detector for callerless pub fns in this repo -- RT-209 needed four iterations without one."
        );
    }
}
