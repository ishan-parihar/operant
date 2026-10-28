use operant_core::database::Database;

// (plan 2026-10-28 iter-B) the old `list_sessions`/`SessionRecord`
// loaders were deleted with the session_browser modal — the hub loads
// `SessionEntryV2` rows through `Database::list_sessions_rich`
// (app/mod.rs). `save_session_title` and `fork_tree` remain live.

// The parent and forks of `session_id` as branch rows (TUI-11). Runs on a
// blocking thread with its own connection; a database error is returned, so
// the caller can say so instead of showing an empty tree.
pub fn fork_tree(
    session_id: &str,
) -> Result<Vec<crate::tui::session_branching::BranchInfo>, String> {
    use crate::tui::session_branching::BranchInfo;
    let db = Database::init(default_db_path()).map_err(|e| e.to_string())?;
    let mut branches = Vec::new();
    if let Some(parent) = db.session_parent(session_id).map_err(|e| e.to_string())? {
        let short: String = parent.chars().take(8).collect();
        branches.push(BranchInfo {
            id: parent,
            name: format!("{short} (parent)"),
            created_at: String::new(),
            is_current: false,
        });
    }
    for (id, title) in db.child_sessions(session_id).map_err(|e| e.to_string())? {
        let name =
            title.unwrap_or_else(|| format!("{} (fork)", id.chars().take(8).collect::<String>()));
        branches.push(BranchInfo {
            id,
            name,
            created_at: String::new(),
            is_current: false,
        });
    }
    Ok(branches)
}

/// Saves a session's title to the database, so a `/rename` survives a restart
/// (TUI-8). A failure is logged: the name is still shown for this run.
pub async fn save_session_title(id: String, title: String) {
    let db_path = default_db_path();
    let result = tokio::task::spawn_blocking(move || -> operant_core::Result<()> {
        Database::init(db_path)?.update_session_title(&id, &title)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "could not save the session title"),
        Err(e) => tracing::warn!(error = %e, "could not save the session title"),
    }
}

fn default_db_path() -> std::path::PathBuf {
    // Match operant_core::platform::operant_home() / "operant.db"
    operant_core::platform::operant_home().join("operant.db")
}
