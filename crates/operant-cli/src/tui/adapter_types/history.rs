use operant_core::database::Database;

// (plan 2026-10-28 iter-B, audit R1 2026-10-28) the old
// `list_sessions`/`SessionRecord` loaders and `fork_tree` (the
// session_branching overlay's feeder) are deleted: the hub loads
// `SessionEntryV2` rows through `Database::list_sessions_rich` and the
// branch view is a FILTER over those rows, not a second listing.
// `save_session_title` remains the only live function in this module.

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
