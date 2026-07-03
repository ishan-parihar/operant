//! Pluggable memory provider system for Operant-RS.
//!
//! Mirrors operant-agent's `MemoryProvider` ABC.  The active provider is
//! selected by `config.memory.provider`:
//!
//! | Value          | Backend                                                     |
//! |----------------|-------------------------------------------------------------|
//! | `"builtin"`    | File-backed MEMORY.md / USER.md in the working directory    |
//! | `"local-vector"` | SQLite-vec + local embedding model (fastembed)            |
//! | `"hindsight"`  | Hindsight Cloud / local API (`HINDSIGHT_API_KEY`)           |
//! | `"retaindb"`   | RetainDB Cloud API (`RETAINDB_API_KEY`)                     |
//! | `"mem0"`       | Mem0 API (`MEM0_API_KEY`)                                   |

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Lifecycle-matching the Python MemoryProvider ABC.
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Short identifier, e.g. `"builtin"`, `"hindsight"`.
    fn name(&self) -> &str;

    /// True when credentials / deps are present (no network calls).
    fn is_available(&self) -> bool;

    /// Connect, create tables, warm up.  Called once at startup.
    async fn initialize(&self, session_id: &str) -> Result<()>;

    /// Static text for the system prompt (instructions / status line).
    fn system_prompt_block(&self) -> String {
        String::new()
    }

    /// Recall relevant context before a turn. Returns formatted text or "".
    async fn prefetch(&self, query: &str) -> String {
        let _ = query;
        String::new()
    }

    /// Persist a completed turn asynchronously.
    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let _ = (user, assistant);
        Ok(())
    }

    /// Tool schemas this provider exposes to the model (OpenAI format).
    fn tool_schemas(&self) -> Vec<Value> {
        vec![]
    }

    /// Dispatch a tool call; return a JSON result string.
    async fn handle_tool_call(&self, name: &str, _args: Value) -> String {
        format!(
            r#"{{"error":"provider {} does not handle tool {}"}}"#,
            self.name(),
            name
        )
    }

    /// Flush queues and close connections.
    async fn shutdown(&self) {}
}

// ---------------------------------------------------------------------------
// Builtin — wraps existing MemoryManager (MEMORY.md / USER.md)
// ---------------------------------------------------------------------------

pub struct BuiltinProvider {
    manager: crate::memory::MemoryManager,
}

impl BuiltinProvider {
    pub fn new(manager: crate::memory::MemoryManager) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl MemoryProvider for BuiltinProvider {
    fn name(&self) -> &str {
        "builtin"
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, session_id: &str) -> Result<()> {
        self.manager.load_from_disk().await?;
        self.manager.start_session(session_id).await;
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "Built-in file memory active (MEMORY.md / USER.md).".to_string()
    }

    async fn prefetch(&self, _query: &str) -> String {
        self.manager.build_memory_context(2000).await
    }

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        self.manager
            .add_message(crate::client::Message::user(user))
            .await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LocalVector — SQLite keyword store (+ optional fastembed semantic search)
// ---------------------------------------------------------------------------
//
// Stores memories in ~/.operant/memory/vectors.db.
// Without the `fastembed` feature, falls back to full-text keyword search.

pub struct LocalVectorProvider {
    db_path: std::path::PathBuf,
}

impl LocalVectorProvider {
    pub fn new() -> Self {
        let home = crate::platform::operant_home();
        Self {
            db_path: home.join("memory").join("local.db"),
        }
    }

    fn open_db(&self) -> std::result::Result<rusqlite::Connection, rusqlite::Error> {
        if let Some(p) = self.db_path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id   INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                ts   INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts
                USING fts5(text, content='memories', content_rowid='id');",
        )?;
        Ok(conn)
    }
}

#[async_trait]
impl MemoryProvider for LocalVectorProvider {
    fn name(&self) -> &str {
        "local-vector"
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        self.open_db()
            .map(|_| ())
            .map_err(|e| crate::error::Error::Agent(e.to_string()))
    }

    fn system_prompt_block(&self) -> String {
        "Local vector memory active (sqlite FTS5).".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        let db_path = self.db_path.clone();
        let query = query.to_string();
        let result = tokio::task::spawn_blocking(
            move || -> std::result::Result<Vec<String>, rusqlite::Error> {
                if let Some(p) = db_path.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let conn = rusqlite::Connection::open(&db_path)?;
                // FTS5 match query — escape special chars
                let safe_q = query.replace('"', "\"\"");
                let mut stmt = conn.prepare(
                    "SELECT m.text FROM memories_fts f
                 JOIN memories m ON m.id = f.rowid
                 WHERE memories_fts MATCH ?1 LIMIT 5",
                )?;
                let rows = stmt
                    .query_map([format!("\"{}\"", safe_q)], |r| r.get(0))?
                    .flatten()
                    .collect();
                Ok(rows)
            },
        )
        .await;

        match result {
            Ok(Ok(rows)) if !rows.is_empty() => format!("[local-vector]\n{}", rows.join("\n")),
            _ => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let db_path = self.db_path.clone();
        let text = format!("User: {}\nAssistant: {}", user, assistant);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        tokio::task::spawn_blocking(move || -> std::result::Result<(), rusqlite::Error> {
            if let Some(p) = db_path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute(
                "INSERT INTO memories (text, ts) VALUES (?1, ?2)",
                [&text, &ts.to_string()],
            )?;
            let id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO memories_fts(rowid, text) VALUES (?1, ?2)",
                rusqlite::params![id, text],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::error::Error::Agent(e.to_string()))?
        .map_err(|e| crate::error::Error::Agent(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Hindsight — cloud/local API
// ---------------------------------------------------------------------------

pub struct HindsightProvider {
    client: crate::memory::HindsightMemoryClient,
}

impl HindsightProvider {
    pub fn new() -> Self {
        Self {
            client: crate::memory::HindsightMemoryClient::from_config(),
        }
    }
}

#[async_trait]
impl MemoryProvider for HindsightProvider {
    fn name(&self) -> &str {
        "hindsight"
    }

    fn is_available(&self) -> bool {
        !std::env::var("HINDSIGHT_API_KEY")
            .unwrap_or_default()
            .is_empty()
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "Hindsight memory active. Use hindsight_retain to store, hindsight_recall to search, hindsight_reflect to synthesize.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        match self.client.recall(query).await {
            Ok(memories) if !memories.is_empty() => {
                format!("[Hindsight]\n{}", memories.join("\n"))
            }
            _ => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let text = format!("User: {}\nAssistant: {}", user, assistant);
        self.client.retain(&text, vec![]).await
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "hindsight_retain",
                "description": "Store information in Hindsight long-term memory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {"type": "string"},
                        "tags": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["content"]
                }
            }),
            serde_json::json!({
                "name": "hindsight_recall",
                "description": "Retrieve memories from Hindsight semantic search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }),
            serde_json::json!({
                "name": "hindsight_reflect",
                "description": "Synthesize an answer from Hindsight memories given a query.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }),
        ]
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        match name {
            "hindsight_retain" => {
                let content = args["content"].as_str().unwrap_or("");
                let tags: Vec<String> = args["tags"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                match self.client.retain(content, tags).await {
                    Ok(_) => r#"{"success":true}"#.to_string(),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, e),
                }
            }
            "hindsight_recall" => {
                let query = args["query"].as_str().unwrap_or("");
                match self.client.recall(query).await {
                    Ok(m) => serde_json::json!({"memories": m}).to_string(),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, e),
                }
            }
            "hindsight_reflect" => {
                let query = args["query"].as_str().unwrap_or("");
                match self.client.reflect(query).await {
                    Ok(text) => serde_json::json!({"text": text}).to_string(),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, e),
                }
            }
            _ => format!(r#"{{"error":"unknown tool {}"}}"#, name),
        }
    }
}

// ---------------------------------------------------------------------------
// RetainDB provider
// ---------------------------------------------------------------------------

pub struct RetainDbProvider {
    api_key: String,
    base_url: String,
    project: String,
    client: reqwest::Client,
}

impl RetainDbProvider {
    pub fn new() -> Self {
        Self {
            api_key: std::env::var("RETAINDB_API_KEY").unwrap_or_default(),
            base_url: std::env::var("RETAINDB_BASE_URL")
                .unwrap_or_else(|_| "https://api.retaindb.com".to_string()),
            project: std::env::var("RETAINDB_PROJECT").unwrap_or_else(|_| "default".to_string()),
            client: reqwest::Client::new(),
        }
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    async fn search_memories(&self, query: &str) -> Result<Vec<String>> {
        let resp = self
            .client
            .post(format!("{}/v1/memory/search", self.base_url))
            .header("Authorization", self.auth())
            .json(&serde_json::json!({
                "project": self.project,
                "query": query,
                "user_id": "default",
                "top_k": 5
            }))
            .send()
            .await?
            .json::<Value>()
            .await?;

        Ok(resp["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| r["content"].as_str().map(String::from))
            .collect())
    }
}

#[async_trait]
impl MemoryProvider for RetainDbProvider {
    fn name(&self) -> &str {
        "retaindb"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        format!("RetainDB memory active (project: {}).", self.project)
    }

    async fn prefetch(&self, query: &str) -> String {
        match self.search_memories(query).await {
            Ok(m) if !m.is_empty() => format!("[RetainDB]\n{}", m.join("\n")),
            _ => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = self
            .client
            .post(format!("{}/v1/memory/ingest/session", self.base_url))
            .header("Authorization", self.auth())
            .json(&serde_json::json!({
                "project": self.project,
                "user_id": "default",
                "messages": [
                    {"role": "user", "content": user, "timestamp": now},
                    {"role": "assistant", "content": assistant, "timestamp": now}
                ]
            }))
            .send()
            .await;
        Ok(())
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "retaindb_search",
                "description": "Semantic search across stored RetainDB memories.",
                "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}
            }),
            serde_json::json!({
                "name": "retaindb_remember",
                "description": "Persist a fact to RetainDB long-term memory.",
                "parameters": {"type": "object", "properties": {"content": {"type": "string"}}, "required": ["content"]}
            }),
        ]
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        match name {
            "retaindb_search" => {
                let q = args["query"].as_str().unwrap_or("");
                match self.search_memories(q).await {
                    Ok(m) => serde_json::json!({"results": m}).to_string(),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, e),
                }
            }
            "retaindb_remember" => {
                let content = args["content"].as_str().unwrap_or("");
                let r = self.client
                    .post(format!("{}/v1/memory", self.base_url))
                    .header("Authorization", self.auth())
                    .json(&serde_json::json!({"project": self.project, "content": content, "user_id": "default"}))
                    .send().await;
                match r {
                    Ok(_) => r#"{"success":true}"#.to_string(),
                    Err(e) => format!(r#"{{"error":"{}"}}"#, e),
                }
            }
            _ => format!(r#"{{"error":"unknown tool {}"}}"#, name),
        }
    }
}

// ---------------------------------------------------------------------------
// Mem0 provider
// ---------------------------------------------------------------------------

pub struct Mem0Provider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl Mem0Provider {
    pub fn new() -> Self {
        Self {
            api_key: std::env::var("MEM0_API_KEY").unwrap_or_default(),
            base_url: std::env::var("MEM0_BASE_URL")
                .unwrap_or_else(|_| "https://api.mem0.ai".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl MemoryProvider for Mem0Provider {
    fn name(&self) -> &str {
        "mem0"
    }

    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "Mem0 memory active.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        let resp = self
            .client
            .post(format!("{}/v1/memories/search/", self.base_url))
            .header("Authorization", format!("Token {}", self.api_key))
            .json(&serde_json::json!({"query": query, "user_id": "default", "limit": 5}))
            .send()
            .await;
        match resp {
            Ok(r) => {
                let json: Value = r.json().await.unwrap_or_default();
                let empty = vec![];
                let results: Vec<&str> = json
                    .as_array()
                    .unwrap_or(&empty)
                    .iter()
                    .filter_map(|m| m["memory"].as_str())
                    .collect();
                if results.is_empty() {
                    String::new()
                } else {
                    format!("[Mem0]\n{}", results.join("\n"))
                }
            }
            Err(_) => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        let _ = self
            .client
            .post(format!("{}/v1/memories/", self.base_url))
            .header("Authorization", format!("Token {}", self.api_key))
            .json(&serde_json::json!({
                "messages": [{"role": "user", "content": user}],
                "user_id": "default"
            }))
            .send()
            .await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct the appropriate provider from the config `provider` string.
/// Falls back to `BuiltinProvider` on unknown values.
pub fn build_memory_provider(
    provider_name: &str,
    storage_dir: std::path::PathBuf,
) -> Arc<dyn MemoryProvider> {
    match provider_name {
        "hindsight" => Arc::new(HindsightProvider::new()),
        "local-vector" | "local_vector" => Arc::new(LocalVectorProvider::new()),
        "retaindb" => Arc::new(RetainDbProvider::new()),
        "mem0" => Arc::new(Mem0Provider::new()),
        "tdg" => Arc::new(TdgMemoryProvider::new(storage_dir)),
        _ => Arc::new(BuiltinProvider::new(
            crate::memory::MemoryManager::with_storage_dir(storage_dir),
        )),
    }
}

// ---------------------------------------------------------------------------
// TDG — Graph memory via tdg-rust
// ---------------------------------------------------------------------------

pub struct TdgMemoryProvider {
    pool: std::sync::Arc<tdg_rust::ConnectionPool>,
    storage_dir: std::path::PathBuf,
}

impl TdgMemoryProvider {
    pub fn new(storage_dir: std::path::PathBuf) -> Self {
        let db_path = storage_dir.join("tdg").join("graph.db");
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let pool = tdg_rust::ConnectionPool::new(
            db_path.to_str().unwrap_or("~/.operant/tdg/graph.db"),
            5,
            30_000,
        )
        .expect("failed to create TDG connection pool");
        pool.with_connection(|conn| {
            tdg_rust::init_schema(conn)?;
            tdg_rust::init_fts(conn)?;
            tdg_rust::run_migrations(conn)?;
            Ok(())
        })
        .expect("failed to initialize TDG schema");
        Self {
            pool: std::sync::Arc::new(pool),
            storage_dir,
        }
    }
}

#[async_trait]
impl MemoryProvider for TdgMemoryProvider {
    fn name(&self) -> &str {
        "tdg"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "TDG graph memory active. Entities, relationships, and temporal context available via tdg_search, tdg_create, tdg_connect, tdg_get_related.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        let pool = self.pool.clone();
        let query = query.to_string();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
            pool.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, node_type, name, description FROM nodes WHERE valid_to IS NULL AND name LIKE ?1 LIMIT 5"
                )?;
                let pattern = format!("%{}%", query);
                let rows: Vec<String> = stmt
                    .query_map(rusqlite::params![pattern], |row| {
                        let id: String = row.get(0)?;
                        let node_type: String = row.get(1)?;
                        let name: String = row.get(2)?;
                        let desc: String = row.get(3)?;
                        Ok(format!("[{}] {}: {} — {}", node_type, id, name, desc))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .map_err(|e| e.to_string())
        })
        .await;

        match result {
            Ok(Ok(rows)) if !rows.is_empty() => format!("[TDG]\n{}", rows.join("\n")),
            _ => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        let pool = self.pool.clone();
        let user_text = user.to_string();
        let _ = tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            pool.with_connection(|conn| {
                let new_node = tdg_rust::NewNode {
                    node_type: "observation".to_string(),
                    name: user_text.chars().take(100).collect(),
                    description: Some(user_text),
                    properties: None,
                    quadrants: None,
                    drives: None,
                    lifecycle_state: None,
                    teleological_level: None,
                    developmental_stage: Some(0),
                    confidence: Some(0.5),
                    source: Some("operant-session".to_string()),
                    parent_ids: None,
                    agent_id: None,
                    ..Default::default()
                };
                tdg_rust::db::crud::add_node(conn, &new_node)?;
                Ok(())
            })
            .map_err(|e| e.to_string())
        })
        .await
        .ok();
        Ok(())
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "tdg_search",
                "description": "Search graph memory using full-text search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }),
            serde_json::json!({
                "name": "tdg_create",
                "description": "Create a new entity node in the graph.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "node_type": {"type": "string"},
                        "name": {"type": "string"},
                        "description": {"type": "string"}
                    },
                    "required": ["node_type", "name"]
                }
            }),
            serde_json::json!({
                "name": "tdg_connect",
                "description": "Create a relationship between two nodes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source_id": {"type": "string"},
                        "target_id": {"type": "string"},
                        "edge_type": {"type": "string"}
                    },
                    "required": ["source_id", "target_id", "edge_type"]
                }
            }),
            serde_json::json!({
                "name": "tdg_get_related",
                "description": "Get nodes connected to a given node.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "node_id": {"type": "string"}
                    },
                    "required": ["node_id"]
                }
            }),
        ]
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        let pool = self.pool.clone();
        let name = name.to_string();
        let args = args.clone();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            pool.with_connection(|conn| match name.as_str() {
                "tdg_search" => {
                    let query = args["query"].as_str().unwrap_or("");
                    let mut stmt = conn.prepare(
                        "SELECT id, node_type, name, description FROM nodes WHERE valid_to IS NULL AND name LIKE ?1 LIMIT 10"
                    )?;
                    let pattern = format!("%{}%", query);
                    let rows: Vec<serde_json::Value> = stmt
                        .query_map(rusqlite::params![pattern], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, String>(0)?,
                                "node_type": row.get::<_, String>(1)?,
                                "name": row.get::<_, String>(2)?,
                                "description": row.get::<_, String>(3)?
                            }))
                        })?
                        .filter_map(|r| r.ok())
                        .collect();
                    Ok(serde_json::json!({"results": rows}).to_string())
                }
                "tdg_create" => {
                    let node_type = args["node_type"].as_str().unwrap_or("observation");
                    let node_name = args["name"].as_str().unwrap_or("unnamed");
                    let desc = args["description"].as_str().unwrap_or("");
                    let new_node = tdg_rust::NewNode {
                        node_type: node_type.to_string(),
                        name: node_name.to_string(),
                        description: Some(desc.to_string()),
                        properties: None,
                        quadrants: None,
                        drives: None,
                        lifecycle_state: None,
                        teleological_level: None,
                        developmental_stage: Some(0),
                        confidence: Some(0.5),
                        source: Some("operant-agent".to_string()),
                        parent_ids: None,
                        agent_id: None,
                        ..Default::default()
                    };
                    let node = tdg_rust::db::crud::add_node(conn, &new_node)?;
                    Ok(serde_json::json!({"id": node.id, "name": node.name}).to_string())
                }
                "tdg_connect" => {
                    let src = args["source_id"].as_str().unwrap_or("");
                    let tgt = args["target_id"].as_str().unwrap_or("");
                    let edge_type = args["edge_type"].as_str().unwrap_or("RELATES_TO");
                let new_edge = tdg_rust::NewEdge {
                    source_id: src.to_string(),
                    target_id: tgt.to_string(),
                    edge_type: edge_type.to_string(),
                    weight: None,
                    properties: None,
                    agent_id: None,
                };
                    let edge = tdg_rust::db::crud::add_edge(conn, &new_edge)?;
                    Ok(serde_json::json!({"edge_id": edge.id}).to_string())
                }
                "tdg_get_related" => {
                    let node_id = args["node_id"].as_str().unwrap_or("");
                    let edges = tdg_rust::db::crud::get_edges(conn, Some(node_id), None, None, None, 20)?;
                    let results: Vec<serde_json::Value> = edges
                        .iter()
                        .map(|e| {
                            let other = if e.source_id == node_id { &e.target_id } else { &e.source_id };
                            serde_json::json!({"edge_id": e.id, "edge_type": e.edge_type, "connected_to": other})
                        })
                        .collect();
                    Ok(serde_json::json!({"relations": results}).to_string())
                }
                _ => Ok(format!(r#"{{"error":"unknown tool {}"}}"#, name)),
            })
            .map_err(|e| e.to_string())
        })
        .await;

        match result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => format!(r#"{{"error":"{}"}}"#, e),
            Err(e) => format!(r#"{{"error":"task failed: {}"}}"#, e),
        }
    }

    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_builtin() {
        let p = build_memory_provider("builtin", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    #[test]
    fn test_build_local_vector() {
        let p = build_memory_provider("local-vector", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "local-vector");
    }

    #[test]
    fn test_build_hindsight() {
        let p = build_memory_provider("hindsight", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "hindsight");
    }

    #[test]
    fn test_build_retaindb() {
        let p = build_memory_provider("retaindb", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "retaindb");
    }

    #[test]
    fn test_build_unknown_falls_back_to_builtin() {
        let p = build_memory_provider("unknown", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    #[test]
    fn test_hindsight_unavailable_without_key() {
        let p = HindsightProvider::new();
        // Without HINDSIGHT_API_KEY set the provider should report unavailable.
        if std::env::var("HINDSIGHT_API_KEY").is_err() {
            assert!(!p.is_available());
        }
    }
}
