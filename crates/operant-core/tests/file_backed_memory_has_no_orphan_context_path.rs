//! The file-backed `MemoryManager` does not carry a second, orphaned memory-context path.
//!
//! # Why
//!
//! `crates/operant-core/src/memory.rs` is the FILE-BACKED memory manager. It
//! held three `pub async` methods with no caller anywhere:
//!
//!   * `get_session_messages_by_id` — only ever served the CURRENT session;
//!     every other id returned an empty `Vec`. A lookup that cannot look
//!     anything up.
//!   * `get_by_type` — a `long_term` filter nothing called.
//!   * `build_memory_context` — built a `<memory_context>` block from the
//!     file-backed manager's `long_term` map.
//!
//! The third is the one worth pinning, because the name is live elsewhere.
//! The REAL memory-context path is the memory-wire recall in `agent/run.rs`
//! (the `<memory_context>` tags, `neutralize_wrapper_tag` at run.rs:4350-4395)
//! and it never came through here. Two mechanisms built the same-named block
//! and only one ran — the orphan was the file-backed one. That is the
//! two-implementations-one-name failure (RG-008/RG-009 family): a reader
//! grepping `build_memory_context` found a plausible answer that was never
//! executed.
//!
//! All three were proven dead the only way that counts, and the same way this
//! test keeps them dead: the `pub` removed, `cargo check -p operant-core
//! --lib` reporting "methods ... are never used". A `pub` method on a `pub`
//! type in a library emits no `dead_code` warning on its own, so the compiler
//! cannot see this class — the gate (`scripts/audit-pub-fn-surface.py`) and
//! tests like this one are what do.

use operant_core::memory::MemoryManager;

/// The file-backed manager keeps the methods that ARE used and does not grow
/// back a context builder of its own.
///
/// This test is deliberately a compile-time contract: it calls the methods
/// that must stay, and the absence of `build_memory_context` is enforced by
/// the gate (a callerless `pub fn` fails `--check`) rather than by a runtime
/// assertion — there is nothing to assert at runtime about a method that does
/// not exist.
#[tokio::test]
async fn the_file_backed_manager_exposes_the_live_methods_only() {
    let manager = MemoryManager::new();

    // These are the methods the rest of the tree actually calls. `store` and
    // `search` and `get_important` are the real surface; `get_session_messages`
    // is what the session path reads.
    manager
        .store(operant_core::memory::MemoryBlock {
            id: "b1".to_string(),
            block_type: "fact".to_string(),
            content: "the sky is blue".to_string(),
            importance: 80,
            created_at: 1_700_000_000,
            last_accessed: 1_700_000_000,
            tags: vec!["nature".to_string()],
        })
        .await;

    let found = manager.search("sky").await;
    assert_eq!(found.len(), 1, "search must find the stored block");
    assert_eq!(found[0].id, "b1");

    let important = manager.get_important(70).await;
    assert_eq!(important.len(), 1, "importance 80 clears a 70 threshold");

    let messages = manager.get_session_messages().await;
    assert!(messages.is_empty(), "no session messages were stored");

    // `clear_all` is the fourth live method; it empties long_term.
    manager.clear_all().await;
    assert!(
        manager.search("sky").await.is_empty(),
        "clear_all emptied long_term"
    );
}

/// `MemoryManager` is `Send + Sync` — it is held across `.await` in the agent
/// loop, so a non-`Send` field would be a compile error at every call site
/// rather than a bug anyone could find by reading.
#[test]
fn the_manager_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MemoryManager>();
}
