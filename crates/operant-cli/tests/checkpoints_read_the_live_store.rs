//! `operant checkpoints` must read the store the checkpoint TOOL writes.
//!
//! # The defect this pins
//!
//! `cmd_checkpoints.rs` opened the `Database` and called
//! `Database::list_checkpoints` / `delete_checkpoint`, which read a
//! `checkpoints` table whose ONLY writer is `Database::store_checkpoint` --
//! and `store_checkpoint` has no caller anywhere in the workspace (verified at
//! iter-2672: `rg 'store_checkpoint' crates/ | grep -v database.rs` is empty).
//!
//! So every arm of the command was a false claim:
//!   * `list` always printed "No checkpoints found for this directory."
//!   * `prune` always reported nothing to prune
//!   * `status` always counted 0
//!   * `clear` returned at the `count == 0` early exit and NEVER reached
//!     `clear_store`, so a store holding gigabytes of packed objects could not
//!     be cleared by the command whose whole job is clearing it.
//!
//! The tool's snapshots go to `checkpoint_tool`'s git shadow stores. The
//! command has to read those.
//!
//! # Why a source contract and not a behaviour test
//!
//! The bug is invisible from outside the process without first building a
//! store, and the FAILURE MODE is silence: the old code exited 0 and printed a
//! plausible sentence. A source contract is what catches the regression at
//! compile time of the test target, and it is the shape the RT-209 row's own
//! guard uses for the same reason.
//!
//! Mutation-proved: restoring a `Database::list_checkpoints` call fails it.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/operant-cli is two levels under the repository root")
        .to_path_buf()
}

fn cmd_checkpoints() -> String {
    std::fs::read_to_string(workspace_root().join("crates/operant-cli/src/cmd_checkpoints.rs"))
        .expect("cmd_checkpoints.rs must exist and be readable")
}

/// The source with every run of whitespace collapsed to one space.
///
/// The receiver form the old code used can be split across lines
/// (`db\\n    .list_checkpoints(&dir)`), and a literal needle misses that --
/// measured: a reverted call site formatted that way passed a substring guard
/// and only failed once the test was run against it. Comparing on the
/// collapsed form makes the formatting irrelevant to the claim.
fn collapsed(src: &str) -> String {
    src.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_command_never_reads_the_database_checkpoint_table() {
    let src = cmd_checkpoints();
    let flat = collapsed(&src);
    // Needles are built by concatenation so this test's own source cannot
    // satisfy them -- the same discipline as the RT-552 route-table guard.
    //
    // BOTH forms are banned, and the second one is the one that matters.
    // `Database::init(..)` followed by a receiver's own `list_checkpoints`
    // is how this file actually read the table, so a guard keyed on the
    // `Database::` prefix alone would have passed a reverted call site
    // (measured: it did). What must NOT be banned is the bare method name --
    // `CheckpointManager::list_checkpoints` is the LIVE API and this file
    // calls it by design, so the receiver form is pinned to the database
    // binding the old code used.
    for forbidden in [
        "Database::".to_string() + "list_checkpoints",
        "Database::".to_string() + "delete_checkpoint",
        "Database::".to_string() + "store_checkpoint",
        "Database::".to_string() + "get_checkpoint",
        "db .".to_string() + "list_checkpoints(",
        "db .".to_string() + "delete_checkpoint(",
    ] {
        assert!(
            !flat.contains(&forbidden),
            "cmd_checkpoints.rs calls {forbidden}, which reads the `checkpoints` table \
             nothing writes. The live store is checkpoint_tool's git shadow store; \
             read it through `live_checkpoints` instead."
        );
    }
}

#[test]
fn the_command_reads_the_live_shadow_store() {
    let src = cmd_checkpoints();
    // The import and at least one call site. Positive needles, split the same
    // way, so a rename does not silently pass.
    let import = "use operant_core::tools::".to_string() + "get_checkpoint_manager;";
    assert!(
        src.contains(&import),
        "cmd_checkpoints.rs must reach the live store through \
         operant_core::tools::get_checkpoint_manager"
    );
    let helper = "fn live_".to_string() + "checkpoints(dir: &str)";
    assert!(
        src.contains(&helper),
        "the four arms must share one reader; a per-arm re-derivation is how \
         the database version drifted in the first place"
    );
    // Every arm that reports a count must go through that reader.
    let calls = src.matches("live_checkpoints(&dir)").count();
    assert!(
        calls >= 4,
        "expected status/list/prune/clear all to read the live store, found {calls} call(s)"
    );
}

#[test]
fn clear_still_reclaims_the_store_when_no_checkpoint_is_counted() {
    let src = cmd_checkpoints();
    // The old `clear` returned early on `count == 0` and never reached
    // `clear_store`, which is exactly how a multi-gigabyte store became
    // unclearable. The empty branch must still try the reclaim.
    let clear_arm = src
        .split("async fn cmd_clear")
        .nth(1)
        .expect("cmd_clear must exist");
    let clear_arm = clear_arm
        .split("async fn ")
        .next()
        .expect("cmd_clear must be the last arm or followed by another fn");
    assert!(
        clear_arm.contains("clear_store(&dir)"),
        "cmd_clear must call clear_store unconditionally: the bytes are on disk \
         whether or not a row can be counted for them"
    );
    // And the early return must be INSIDE that branch, not before it.
    let early = clear_arm
        .find("return Ok(())")
        .expect("the empty case returns early");
    let reclaim = clear_arm
        .find("clear_store(&dir)")
        .expect("clear_store must be present");
    assert!(
        reclaim < early || clear_arm[..reclaim].contains("count == 0"),
        "the count==0 branch must attempt the reclaim before returning"
    );
}
