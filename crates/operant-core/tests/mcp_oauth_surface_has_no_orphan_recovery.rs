//! The MCP OAuth surface carries no second, unreachable recovery path.
//!
//! # Why
//!
//! `crates/operant-core/src/mcp_oauth.rs` held eight `pub` items with no caller
//! anywhere in the workspace, each proven dead by the acid test (the `pub`
//! removed, `cargo check -p operant-core --lib` reporting "never used"):
//!
//!   * `OAuthToken::remaining_seconds` — an expiry countdown nothing read.
//!   * `TokenStorage::has_tokens` — a file-existence check nothing made.
//!   * `OAuthProvider::build_authorization_url` — the PKCE authorization-URL
//!     builder. The live path builds its URL elsewhere.
//!   * `OAuthProvider::load_cached_metadata` / `load_cached_client_info` —
//!     disk-to-memory warmers nothing called before a request.
//!   * `OAuthManager::invalidate_if_disk_changed` — the mtime-based cache
//!     invalidation.
//!   * `OAuthManager::handle_401` — the 401 refresh path.
//!   * `reset_manager_for_tests` — a "reset" that only logged
//!     "OnceLock cannot be reset" and changed nothing.
//!
//! The last two were the load-bearing pair. `ProviderEntry` carried four fields
//! (`config`, `last_mtime_ns`, `lock`, `_pending_401`) read ONLY by those two
//! methods, so deleting them orphaned the whole mtime-invalidation + 401-dedup
//! machinery. The manager's own doc comment still advertises both as features.
//! **Two mechanisms, one description, only one reachable** — the same shape as
//! the file-backed memory context path (iter-2727): a reader who greps "does
//! MCP OAuth recover from a 401?" finds a confident answer describing code that
//! never runs.
//!
//! This is NOT an OAuth security hole. The live recovery is the provider's own
//! `refresh_token`, reached through `get_provider(...).authenticate()`;
//! `oauth/mod.rs` owns the provider-account path separately. It is a
//! *correctness-of-description* defect — the tree claimed a recovery surface it
//! did not have.
//!
//! What this test pins: the three calls the CLI and TUI actually make
//! (`get_provider`, `get_token`, `remove`) still behave as documented.

use operant_core::mcp_oauth::{McpOAuthConfig, get_manager};

/// The manager's live surface: a provider per server, cached per URL, and a
/// `remove` that drops the entry. These are the calls `cmd_mcp.rs` and the TUI
/// actually make.
///
/// Deliberately NOT `#[tokio::test]`: `get_provider` takes a
/// `blocking_read()` on the entry, which panics inside a runtime ("Cannot block
/// the current thread from within a runtime"). The real callers are sync CLI /
/// TUI code paths, so a plain test matches how it is actually used.
#[test]
fn the_live_manager_surface_behaves_as_documented() {
    let mgr = get_manager();

    // A URL no real deployment uses, so the test cannot read or delete a live
    // operator's tokens.
    let url = "https://mcp-oauth-surface-test.invalid/live";

    let mut cfg = McpOAuthConfig::default();
    cfg.redirect_port = Some(0);

    // get_provider must succeed and be repeatable (the second call takes the
    // cached path, which is the behaviour ProviderEntry exists for).
    let _p1 = mgr.get_provider(url, Some(cfg.clone()));
    let _p2 = mgr.get_provider(url, Some(cfg));

    // remove() must not panic on a server with nothing on disk, and must leave
    // the manager usable afterwards.
    mgr.remove(url);
    let _p3 = mgr.get_provider(url, Some(McpOAuthConfig::default()));
}

/// `McpOAuthConfig` is `Clone + Default`: `cmd_mcp.rs` builds one from CLI
/// flags with `..Default::default()`, and `get_provider` takes it by value.
#[test]
fn the_config_type_is_clone_and_default() {
    let cfg = McpOAuthConfig::default();
    let _cloned = cfg.clone();
}
