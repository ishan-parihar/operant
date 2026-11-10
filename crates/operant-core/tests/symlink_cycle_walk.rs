//! RT-446: the file tools' recursive walkers FOLLOWED symlinks.
//!
//! `file_state.rs`'s collector and `file_tools.rs`'s search and list
//! walkers all asked `path.is_dir()` — which resolves a symlink. A
//! `loop -> .` symlink therefore made them recurse forever: a stack
//! overflow, which ABORTS the process rather than returning an error.
//!
//! The repo's own rule is already written down in `checkpoint_tool.rs`:
//! "symlinks are never followed, so a symlink cycle cannot loop forever
//! … `entry.file_type()` does NOT follow symlinks — `path.is_dir()` would
//! follow a `loop -> .` cycle forever."
//!
//! BEHAVIORAL: each walker must return on a directory holding such a
//! symlink. (With the defect, the test process ABORTS on the stack
//! overflow — the failure mode this pins.)

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::tools::file_state::FileStateTool;
use operant_core::tools::file_tools::{FileListTool, FileSearchTool};
use operant_core::tools::{OperantTool, ToolContext};
use serde_json::json;

/// A directory holding one real file and a `loop -> .` symlink.
fn dir_with_a_symlink_cycle() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let sub = dir.path().join("tree");
    std::fs::create_dir_all(&sub).expect("tree");
    std::fs::write(sub.join("real.txt"), "hello\n").expect("real file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&sub, sub.join("loop")).expect("the cycle symlink");
    }
    (dir, sub)
}

#[tokio::test]
async fn the_file_state_collector_survives_a_symlink_cycle() {
    let (_dir, tree) = dir_with_a_symlink_cycle();
    let result = FileStateTool
        .execute(
            json!({ "path": tree.to_string_lossy(), "action": "check" }),
            ToolContext::default(),
        )
        .await;
    // The call RETURNS (with the defect the walker chased the cycle, and
    // every level's files were hashed again).
    let rendered = format!("{result:?}");
    assert!(
        !rendered.contains("loop/loop"),
        "the walker must not descend through a symlink: {rendered}"
    );
}

#[tokio::test]
async fn the_file_search_survives_a_symlink_cycle() {
    let (_dir, tree) = dir_with_a_symlink_cycle();
    let result = FileSearchTool
        .execute(
            json!({ "path": tree.to_string_lossy(), "pattern": "hello" }),
            ToolContext::default(),
        )
        .await;
    let rendered = format!("{result:?}");
    assert!(
        !rendered.contains("loop/loop"),
        "the walker must not descend through a symlink: {rendered}"
    );
}

#[tokio::test]
async fn the_file_list_survives_a_symlink_cycle() {
    let (_dir, tree) = dir_with_a_symlink_cycle();
    let result = FileListTool
        .execute(
            json!({ "path": tree.to_string_lossy(), "recursive": true }),
            ToolContext::default(),
        )
        .await;
    let payload: serde_json::Value =
        serde_json::from_str(&result.content).expect("the list returns JSON");
    assert_eq!(
        payload["count"], 2,
        "the listing must hold the directory's two entries, not the same tree \
         at dozens of levels: {payload}"
    );
    let rendered = format!("{payload}");
    assert!(
        !rendered.contains("loop/loop"),
        "the walker must not descend through a symlink: {rendered}"
    );
}

/// RT-446: and the walker does not read THROUGH a symlink — a link to a
/// sensitive file inside an allowed directory would be read past the
/// deny list (the requested path is validated; the files found inside it
/// were not).
#[tokio::test]
async fn the_search_does_not_read_through_a_symlink() {
    let dir = tempfile::tempdir().expect("tempdir");
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "TOP-SECRET-MARKER\n").expect("secret");
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).expect("tree");
    std::fs::write(tree.join("real.txt"), "hello\n").expect("real file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, tree.join("link.txt")).expect("the link");
    }

    let result = FileSearchTool
        .execute(
            json!({ "path": tree.to_string_lossy(), "pattern": "TOP-SECRET-MARKER" }),
            ToolContext::default(),
        )
        .await;
    // The pattern is echoed in the result, so the assertion is on the
    // MATCHES: the symlinked file must not have been read through.
    let payload: serde_json::Value =
        serde_json::from_str(&result.content).expect("the search returns JSON");
    assert_eq!(
        payload["count"], 0,
        "a symlinked file must not be read through: {payload}"
    );
    assert!(
        payload["matches"].as_array().is_some_and(|m| m.is_empty()),
        "a symlinked file must not be read through: {payload}"
    );
}
