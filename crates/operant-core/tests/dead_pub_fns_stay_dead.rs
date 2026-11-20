#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The pub fns deleted as callerless must stay deleted.
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

/// Workspace root, found by the two markers every checkout has.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("AGENTS.md").exists() && p.join("crates").exists())
        .expect("workspace root (AGENTS.md + crates/)")
        .to_path_buf()
}

/// A re-added definition of any of the three A7 deletions is the regression.
/// Kept separate from `the_pub_fn_batch_stays_deleted` below because this is
/// the ORIGINAL A7 trio; the other holds the iter-2753/2754 batches.
#[test]
fn the_three_dead_pub_fns_stay_deleted() {
    let root = workspace_root();

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

/// The pub-fn batch, deleted after the auditor reported them (iter-2753).
///
/// Twenty-two needles, one per deleted definition: the tests=0 half of the
/// peer plan `2026-11-20-close-the-pub-fn-surface-queue-*` plus the
/// candidates the same scan surfaced at this tip (structural_check, evaluate,
/// session_parent, historian_system_prompt, list_subagents).
///
/// Every one was acid-proven dead the same way A7 proved its three: drop the
/// `pub`, `cargo check -p operant-core --lib`, restore. Three of them were
/// not merely unused but DUPLICATED -- `from_app_config`'s path rule existed
/// three times (the core helper plus one private copy each in `cmd_kanban.rs`
/// and `dashboard_server.rs`), so the canonical one was the dead one; those
/// two call sites now route through the core helper, which is why the name
/// moved from the "dead" list to a live caller. `format_terminal` was the
/// same shape (docs/library/observability.md named `operant insights` as an
/// engine surface while the command bypassed the engine), so it is WIRED and
/// live-verified, not deleted.
///
/// `enable_tool` is deliberately NOT in this list: it is the symmetric half of
/// the live `disable_tool`, and the audit's verdict for it was a DECISION, not
/// a deletion.
#[test]
fn the_pub_fn_batch_stays_deleted() {
    let root = workspace_root();

    let dead: [(&str, &str); 22] = [
        (
            "crates/operant-core/src/skill_usage.rs",
            concat!("pub fn ", "all_records", "_mut"),
        ),
        (
            "crates/operant-core/src/org/seat_authority.rs",
            concat!("pub fn ", "approver", "_of"),
        ),
        (
            "crates/operant-core/src/skills_guard.rs",
            concat!("pub fn ", "determine", "_trust"),
        ),
        (
            "crates/operant-core/src/skills_guard.rs",
            concat!("pub fn ", "format", "_report"),
        ),
        (
            "crates/operant-core/src/skills_guard.rs",
            concat!("pub fn ", "structural", "_check"),
        ),
        (
            "crates/operant-core/src/skills_guard.rs",
            concat!("pub fn ", "evalu", "ate"),
        ),
        (
            "crates/operant-core/src/gateway/dispatch_breaker.rs",
            concat!("pub fn ", "from_str", "_key"),
        ),
        (
            "crates/operant-core/src/database.rs",
            concat!("pub fn ", "session", "_parent"),
        ),
        (
            "crates/operant-core/src/database.rs",
            concat!("pub fn ", "child", "_sessions"),
        ),
        (
            "crates/operant-core/src/context_window.rs",
            concat!("pub fn ", "output", "_ceiling"),
        ),
        (
            "crates/operant-core/src/estop.rs",
            concat!("pub fn ", "sentinel", "_path()"),
        ),
        (
            "crates/operant-core/src/config_heal.rs",
            concat!("pub fn ", "render", "_parked"),
        ),
        (
            "crates/operant-core/src/agent/retry_policy.rs",
            concat!("pub fn ", "is", "_announced"),
        ),
        (
            "crates/operant-core/src/agent/builders.rs",
            concat!("pub async fn ", "list", "_subagents"),
        ),
        (
            "crates/operant-core/src/model_roles.rs",
            concat!("pub fn ", "single", "_model"),
        ),
        (
            "crates/operant-core/src/memory.rs",
            concat!("pub fn ", "resolve_memory", "_provider"),
        ),
        (
            "crates/operant-core/src/accessibility.rs",
            concat!("pub fn ", "has", "_text"),
        ),
        (
            "crates/operant-core/src/historian.rs",
            concat!("pub fn ", "historian", "_system_prompt"),
        ),
        (
            "crates/operant-core/src/daemon_pool.rs",
            concat!("pub fn ", "spawn", "_anonymous"),
        ),
        (
            "crates/operant-core/src/tools.rs",
            concat!("pub fn ", "is", "_direct"),
        ),
        (
            "crates/operant-core/src/oauth/store.rs",
            concat!("pub fn ", "with", "_lifetime"),
        ),
        (
            "crates/operant-core/src/oauth/store.rs",
            concat!("pub fn ", "with", "_default_path"),
        ),
    ];

    for (rel, needle) in dead {
        let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(
            !text.contains(needle),
            "{rel} defines `{needle}` again. It was deleted as callerless -- proven by dropping the `pub` and watching `cargo check -p operant-core --lib` stay clean. The audit that found them is `scripts/audit-pub-fn-surface.py`; if a caller now needs it, say what the caller is in the same commit and drop this entry."
        );
    }
}

/// `from_app_config` is the INVERSE case: it was dead and is now live.
///
/// The audit reported `KanbanManager::from_app_config` as callerless, and the
/// acid test agreed -- but the honest fix was not to delete it. Two private
/// copies of its exact `database_path.parent()` rule existed in the CLI
/// (`cmd_kanban::kanban_dir`, `dashboard_server::build_app`), so deleting the
/// core helper would have left the rule duplicated twice and the canonical
/// version gone. Both call sites now route through it.
///
/// This assertion is the guard that it STAYS wired: if someone reintroduces a
/// private copy, the canonical helper goes back to being dead, which is
/// exactly the state this iteration closed.
#[test]
fn kanban_dir_is_derived_in_one_place() {
    let cli = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../operant-cli/src/cmd_kanban.rs"
    ))
    .unwrap_or_else(|e| panic!("cmd_kanban.rs: {e}"));
    let dash = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../operant-cli/src/dashboard_server.rs"
    ))
    .unwrap_or_else(|e| panic!("dashboard_server.rs: {e}"));

    // The private copies must be GONE.
    assert!(
        !cli.contains("fn kanban_dir"),
        "cmd_kanban.rs grew a private `kanban_dir()` again -- the core `KanbanManager::from_app_config` is the one rule; route through it"
    );
    // And the call sites must go through the canonical helper.
    assert!(
        cli.contains("KanbanManager::from_app_config"),
        "cmd_kanban.rs must build its manager with KanbanManager::from_app_config"
    );
    assert!(
        dash.contains("KanbanManager::from_app_config"),
        "dashboard_server.rs must derive the kanban dir through KanbanManager::from_app_config"
    );
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
