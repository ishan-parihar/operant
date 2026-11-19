#![allow(clippy::expect_used, clippy::unwrap_used)]

//! RT-275: the six database surfaces that had no caller, and the three config
//! types that were never reachable, are deleted and stay deleted.
//!
//! Each was verified absent of production callers before removal, not assumed:
//!
//! ```text
//! rg -n '\.(set_tool_state|get_tool_state|clear_tool_state|clear_all_tool_states)\(' \
//!     --type rust crates/ | grep -v database.rs   -> (empty)
//! rg -n '\.(store_checkpoint|get_checkpoint)\(' \
//!     --type rust crates/ | grep -v database.rs \
//!                          | grep -v checkpoint_tool.rs   -> (empty)
//! rg -n 'ReliabilityConfig|MemoryPolicyConfig|MultimodalConfig' \
//!     --type rust crates/ | grep -v operant-config/src/schema   -> (empty)
//! ```
//!
//! The live replacements are both named in the tree: tool state is
//! `tools/tool_state.rs` (`tool_state.json`, what `operant tools disable`
//! writes), and checkpoints are the filesystem shadow-git manager in
//! `tools/checkpoint_tool.rs`.
//!
//! Two things were deliberately KEPT, and this guard says so:
//!
//! * the `tools_state` and `checkpoints` TABLES — a live database already
//!   carries them, and dropping a table is a migration;
//! * `Database::list_checkpoints` and `Database::delete_checkpoint` — they have
//!   callers and RT-612 repointed `operant checkpoints` at the shadow store
//!   through them.
//!
//! A re-added method or a revived config type fails the build, which is the
//! only way a deletion this quiet stays deleted.

fn database_source() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/database.rs"))
        .unwrap_or_else(|e| panic!("database.rs is readable: {e}"))
}

#[test]
fn the_callerless_database_methods_stay_deleted() {
    let db = database_source();

    // The needle is split so this test's own source cannot satisfy it.
    let needles = [
        ["pub fn set_", "tool_state"],
        ["pub fn get_", "tool_state"],
        ["pub fn clear_", "tool_state"],
        ["pub fn clear_all_", "tool_states"],
        ["pub fn store_", "checkpoint"],
        ["pub fn get_", "checkpoint"],
    ];
    for [a, b] in needles {
        let needle = format!("{a}{b}");
        assert!(
            !db.contains(&needle),
            "{needle}() was deleted in RT-275: it had zero production callers and \
             the live store is tools/tool_state.rs / tools/checkpoint_tool.rs. Do \
             not revive it without a caller."
        );
    }
}

#[test]
fn the_tables_and_the_live_checkpoint_methods_stay() {
    let db = database_source();

    // The tables must survive: a database that already carries them keeps opening.
    assert!(
        db.contains("CREATE TABLE IF NOT EXISTS tools_state"),
        "the tools_state table stays -- dropping it is a migration, not a deletion"
    );
    assert!(
        db.contains("CREATE TABLE IF NOT EXISTS checkpoints"),
        "the checkpoints table stays -- dropping it is a migration, not a deletion"
    );

    // The two methods `operant checkpoints` (RT-612) reads through stay.
    assert!(
        db.contains("pub fn list_checkpoints"),
        "list_checkpoints has callers (cmd_checkpoints.rs) and stays"
    );
    assert!(
        db.contains("pub fn delete_checkpoint"),
        "delete_checkpoint has callers (cmd_checkpoints.rs) and stays"
    );
}

#[test]
fn the_unreachable_config_types_stay_deleted() {
    let schema = std::fs::read_dir(format!(
        "{}/../operant-config/src/schema",
        env!("CARGO_MANIFEST_DIR")
    ))
    .map(|d| {
        d.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect::<Vec<_>>()
            .join("\n")
    })
    .expect("the operant-config schema dir is readable");

    for name in [
        "ReliabilityConfig",
        "MemoryPolicyConfig",
        "MultimodalConfig",
    ] {
        assert!(
            !schema.contains(&format!("pub struct {name}")),
            "{name} was deleted in RT-275: it was never reachable from AppConfig, \
             and AppConfig uses deny_unknown_fields, so an operator naming its \
             table already got a hard load error"
        );
    }
}
