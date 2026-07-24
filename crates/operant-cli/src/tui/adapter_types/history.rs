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

/// Load a session's messages from the database. Returns an empty vec on
/// error. Used by /resume to populate the transcript after the user
/// picks a session.
pub async fn load_session(session_id: String) -> Vec<(String, String)> {
    load_session_from_path(default_db_path(), session_id).await
}

/// Same as `load_session` but takes an explicit db path (for testing).
pub async fn load_session_from_path(
    db_path: std::path::PathBuf,
    session_id: String,
) -> Vec<(String, String)> {
    tokio::task::spawn_blocking(move || -> Vec<(String, String)> {
        let db = match Database::init(db_path) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let msgs = match db.get_session_messages(&session_id) {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        msgs.into_iter().map(|m| (m.role, m.content)).collect()
    })
    .await
    .unwrap_or_default()
}

fn default_db_path() -> std::path::PathBuf {
    // Match operant_core::platform::operant_home() / "operant.db"
    operant_core::platform::operant_home().join("operant.db")
}
