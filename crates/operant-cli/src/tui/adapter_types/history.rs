use operant_core::database::Database;

pub struct SessionRecord {
    pub id: String,
    pub title: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<String>,
    pub total_cost: f64,
}

/// List recent sessions from the operant-core database. Returns an empty
/// vec if the database can't be opened (e.g. fresh install with no DB yet)
/// so the TUI's session browser shows "no sessions" instead of crashing.
///
/// `db_path` is typically `config.database_path` from AppConfig.
/// The parent and forks of `session_id` as branch rows (TUI-11). Runs on a
/// blocking thread with its own connection; a database error is returned, so
/// the caller can say so instead of showing an empty tree.
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

pub async fn list_sessions() -> Vec<SessionRecord> {
    list_sessions_from_path(default_db_path()).await
}

/// Same as `list_sessions` but takes an explicit db path (for testing).
pub async fn list_sessions_from_path(db_path: std::path::PathBuf) -> Vec<SessionRecord> {
    // Run the blocking DB call on a spawn_blocking so we don't stall the
    // async runtime. The DB lock is held only for the duration of the query.
    tokio::task::spawn_blocking(move || -> Vec<SessionRecord> {
        let db = match Database::init(db_path) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let sessions = match db.list_sessions(50) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        sessions
            .into_iter()
            .map(|s| SessionRecord {
                id: s.id,
                title: s.title,
                // DatabaseSession stores updated_at (ended_at) as an
                // rfc3339 string; parse it into DateTime<Utc> for the
                // TUI's relative-time formatting. Fall back to now()
                // only if the stored string can't be parsed.
                updated_at: chrono::DateTime::parse_from_rfc3339(&s.updated_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                // The list view only needs the message *count* (the
                // consumer calls .messages.len()), so carry the DB's
                // message_count as placeholders rather than loading
                // every message body per session.
                // ponytail: count-carrier vec, not real content — make
                // this a usize field if a caller ever needs the bodies.
                messages: vec![String::new(); s.message_count],
                // R3: real accumulated cost, persisted by the agent via
                // Database::update_session_cost after each completed turn.
                total_cost: s.actual_cost_usd.unwrap_or(0.0),
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn default_db_path() -> std::path::PathBuf {
    // Match operant_core::platform::operant_home() / "operant.db"
    operant_core::platform::operant_home().join("operant.db")
}
