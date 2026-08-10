//! agentmemory backend — hybrid semantic memory via the agentmemory server.
//!
//! agentmemory (<https://github.com/rohitg00/agentmemory>) is a local memory
//! server for AI coding agents built on the iii engine. It exposes a REST API
//! on port 3111 (BM25 + local-embedding hybrid retrieval, consolidation,
//! decay, knowledge graph) and an MCP server with 50+ tools.
//!
//! This module implements the [`Memory`] trait against agentmemory's REST
//! API so the runtime/daemon memory layer gets a real agentmemory provider —
//! previously `memory.backend = "agentmemory"` silently fell back to the
//! markdown backend via the custom extension-point profile. The mapping
//! mirrors the hermes-agent `integrations/hermes` memory plugin:
//!
//! | `Memory` method   | REST call                      |
//! |-------------------|--------------------------------|
//! | `store`           | `POST /agentmemory/remember`   |
//! | `recall`          | `POST /agentmemory/smart-search` |
//! | `get` / `list`    | `POST /agentmemory/smart-search` (filtered) |
//! | `health_check`    | `GET /agentmemory/health`      |
//!
//! The agentmemory REST API does not expose per-key deletion or a total
//! count, so [`Memory::forget`] and [`Memory::count`] return clear
//! unsupported errors (same posture as the hermes plugin, which only mirrors
//! add/update writes). All state lives in the external server
//! (`~/.agentmemory`); construction is lazy (no server contact), so an
//! unreachable server degrades to per-call errors instead of failing boot.
//!
//! **Keyless API note:** agentmemory has no per-entry keys, so `store`
//! ignores the `key` argument and recalls return a content-derived key. As a
//! consequence the reserved-prefix context filtering used by the SQLite
//! backend (`assistant_resp_*` / `user_msg_*` exclusion) does **not** apply
//! to entries stored through this backend — agentmemory is the authoritative
//! relevance filter instead. This mirrors the hermes plugin design (its
//! `prefetch`/`context` return whatever agentmemory deems relevant).

use crate::error::Result;
use crate::traits::{
    Memory, MemoryCategory, MemoryEntry, MemoryError, MemoryResult, normalize_recent_recall_query,
};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// Default agentmemory server base URL (matches the agentmemory package).
pub const DEFAULT_AGENTMEMORY_URL: &str = "http://localhost:3111";
/// Per-request HTTP timeout — short enough to never stall the agent loop,
/// long enough for a local server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on results mapped back from a single smart-search call.
const MAX_RECALL_LIMIT: usize = 50;

/// agentmemory REST client implementing the [`Memory`] trait.
///
/// Thin proxy: all entries live in the external agentmemory server. The
/// client is cheap to clone (internally Arc-backed), so it can be shared
/// across `Arc<dyn Memory>` handles.
#[derive(Clone)]
pub struct AgentMemory {
    client: reqwest::Client,
    base_url: String,
    secret: Option<String>,
}

impl std::fmt::Debug for AgentMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMemory")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AgentMemory {
    /// Create a backend from the `AGENTMEMORY_URL` / `AGENTMEMORY_SECRET`
    /// environment variables. Falls back to [`DEFAULT_AGENTMEMORY_URL`] when
    /// `AGENTMEMORY_URL` is unset or empty.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("operant-memory-agentmemory")
            .build()
            .map_err(|e| {
                crate::error::Error::message(format!(
                    "failed to build agentmemory HTTP client: {e}"
                ))
            })?;
        let base_url = std::env::var("AGENTMEMORY_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_AGENTMEMORY_URL.to_string());
        let secret = std::env::var("AGENTMEMORY_SECRET")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Self {
            client,
            base_url,
            secret,
        })
    }

    /// Create a backend pointing at an explicit base URL (used by tests).
    pub fn with_url(base_url: impl Into<String>, secret: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .user_agent("operant-memory-agentmemory")
                .build()
                .expect("reqwest client build cannot fail"),
            base_url: base_url.into(),
            secret,
        }
    }

    /// Absolute URL for a REST path under the agentmemory API.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// POST a JSON body to an agentmemory REST path; returns the parsed body.
    async fn post_json(&self, path: &str, body: Value) -> MemoryResult<Value> {
        let mut req = self.client.post(self.url(path)).json(&body);
        if let Some(secret) = &self.secret {
            req = req.header("Authorization", format!("Bearer {secret}"));
        }
        let resp = req.send().await.map_err(|e| {
            MemoryError::message(format!("agentmemory request to {path} failed: {e}"))
        })?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(MemoryError::message(format!(
                "agentmemory {path} returned HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        serde_json::from_str::<Value>(&text).map_err(|e| {
            MemoryError::message(format!("agentmemory {path} returned non-JSON body: {e}"))
        })
    }

    /// True when an entry's RFC 3339 timestamp is within the inclusive
    /// `since`/`until` bounds (RFC 3339 strings compare lexicographically).
    fn entry_within_time_bounds(
        entry: &MemoryEntry,
        since: Option<&str>,
        until: Option<&str>,
    ) -> bool {
        let ts = entry.timestamp.as_str();
        if let Some(since) = since
            && !since.is_empty()
            && ts < since
        {
            return false;
        }
        if let Some(until) = until
            && !until.is_empty()
            && ts > until
        {
            return false;
        }
        true
    }

    /// Map a single smart-search result object into a [`MemoryEntry`].
    /// Accepts the common shapes agentmemory returns (`content`/`text`/
    /// `summary`/`title`, `id`, `sessionId`, `score`).
    fn entry_from_result(result: &Value, fallback_index: usize) -> MemoryEntry {
        let content = result
            .get("content")
            .or_else(|| result.get("text"))
            .or_else(|| result.get("summary"))
            .or_else(|| result.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("agentmemory-{fallback_index}"));
        // The key must never be empty: memory tools key on it (get/forget).
        // Fall back to the content prefix, then to the id when even the
        // content is empty.
        let key = result
            .get("key")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let derived: String = content.chars().take(48).collect();
                (!derived.is_empty()).then_some(derived)
            })
            .unwrap_or_else(|| id.clone());
        let category = match result
            .get("type")
            .or_else(|| result.get("category"))
            .and_then(|v| v.as_str())
        {
            Some("core") => MemoryCategory::Core,
            Some("daily") => MemoryCategory::Daily,
            Some("conversation") => MemoryCategory::Conversation,
            _ => MemoryCategory::Conversation,
        };
        let timestamp = result
            .get("timestamp")
            .or_else(|| result.get("createdAt"))
            .or_else(|| result.get("created_at"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            });
        let session_id = result
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let score = result.get("score").and_then(|v| v.as_f64());
        MemoryEntry {
            id,
            key,
            content,
            category,
            timestamp,
            session_id,
            score,
            namespace: "default".into(),
            importance: None,
            superseded_by: None,
        }
    }

    /// Extract the `results` array from a smart-search response, tolerating
    /// the `results`/`memories`/`data` shapes agentmemory may use.
    fn search_results(value: &Value) -> Vec<Value> {
        value
            .get("results")
            .or_else(|| value.get("memories"))
            .or_else(|| value.get("data"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl Memory for AgentMemory {
    fn name(&self) -> &str {
        "agentmemory"
    }

    async fn store(
        &self,
        _key: &str,
        content: &str,
        _category: MemoryCategory,
        session_id: Option<&str>,
    ) -> MemoryResult<()> {
        if content.trim().is_empty() {
            return Err(MemoryError::message(
                "cannot store empty memory content via agentmemory",
            ));
        }
        let mut body = serde_json::Map::new();
        body.insert("content".into(), Value::String(content.to_string()));
        body.insert("type".into(), Value::String("fact".into()));
        if let Some(session_id) = session_id {
            body.insert("sessionId".into(), Value::String(session_id.to_string()));
        }
        self.post_json("/agentmemory/remember", Value::Object(body))
            .await?;
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        _session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> MemoryResult<Vec<MemoryEntry>> {
        // Bare / empty queries mean "recent memories" — send the normalized
        // empty query so agentmemory returns recent entries.
        let query = normalize_recent_recall_query(query);
        let body = serde_json::json!({
            "query": query,
            "limit": limit.clamp(1, MAX_RECALL_LIMIT),
        });
        let value = self.post_json("/agentmemory/smart-search", body).await?;
        // The agentmemory REST API has no since/until query parameters, so
        // apply the trait contract (inclusive RFC 3339 bounds) client-side
        // on the mapped entries — the runtime memory_recall tool passes
        // these straight through.
        let entries: Vec<MemoryEntry> = Self::search_results(&value)
            .iter()
            .enumerate()
            .map(|(i, r)| Self::entry_from_result(r, i))
            .collect();
        Ok(entries
            .into_iter()
            .filter(|e| Self::entry_within_time_bounds(e, since, until))
            .collect())
    }

    async fn get(&self, key: &str) -> MemoryResult<Option<MemoryEntry>> {
        // Best-effort: agentmemory's API is semantic; the closest equivalent
        // to an exact-key lookup is a smart-search scoped to the key.
        let entries = self.recall(key, 1, None, None, None).await?;
        Ok(entries.into_iter().next())
    }
    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> MemoryResult<Vec<MemoryEntry>> {
        // Best-effort: agentmemory has no list endpoint, so list() is a
        // recent-memories search. Request a larger window than recall() and
        // document the truncation: entries beyond MAX_RECALL_LIMIT are not
        // returned (the API does not expose a full enumeration).
        let mut entries = self
            .recall("", MAX_RECALL_LIMIT, session_id, None, None)
            .await?;
        if let Some(category) = category {
            entries.retain(|e| &e.category == category);
        }
        Ok(entries)
    }

    async fn forget(&self, _key: &str) -> MemoryResult<bool> {
        Err(MemoryError::message(
            "agentmemory REST API does not expose per-key deletion; remove entries via the agentmemory CLI/viewer",
        ))
    }

    async fn count(&self) -> MemoryResult<usize> {
        Err(MemoryError::message(
            "agentmemory REST API does not expose a total memory count; use memory_search to inspect entries",
        ))
    }

    async fn health_check(&self) -> bool {
        match self
            .client
            .get(self.url("/agentmemory/health"))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serializes env-mutating tests (cargo runs tests in parallel threads;
    /// two tests mutating `AGENTMEMORY_URL` concurrently race).
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn name_identifies_agentmemory() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        assert_eq!(backend.name(), "agentmemory");
    }

    #[test]
    fn new_resolves_env_url_and_secret() {
        let _guard = env_lock().lock().unwrap();
        let prev_url = std::env::var_os("AGENTMEMORY_URL");
        let prev_secret = std::env::var_os("AGENTMEMORY_SECRET");
        // SAFETY: test-only env mutation under exclusive lock, restored below.
        unsafe {
            std::env::set_var("AGENTMEMORY_URL", "http://localhost:4321");
            std::env::set_var("AGENTMEMORY_SECRET", "s3cret");
        }
        let backend = AgentMemory::new().expect("construction is infallible");
        assert_eq!(backend.base_url, "http://localhost:4321");
        assert_eq!(backend.secret.as_deref(), Some("s3cret"));
        match prev_url {
            Some(v) => unsafe { std::env::set_var("AGENTMEMORY_URL", v) },
            None => unsafe { std::env::remove_var("AGENTMEMORY_URL") },
        }
        match prev_secret {
            Some(v) => unsafe { std::env::set_var("AGENTMEMORY_SECRET", v) },
            None => unsafe { std::env::remove_var("AGENTMEMORY_SECRET") },
        }
    }

    #[test]
    fn new_defaults_to_localhost_when_env_unset() {
        let _guard = env_lock().lock().unwrap();
        let prev_url = std::env::var_os("AGENTMEMORY_URL");
        // SAFETY: test-only env mutation under exclusive lock, restored below.
        unsafe { std::env::remove_var("AGENTMEMORY_URL") };
        let backend = AgentMemory::new().expect("construction is infallible");
        assert_eq!(backend.base_url, DEFAULT_AGENTMEMORY_URL);
        match prev_url {
            Some(v) => unsafe { std::env::set_var("AGENTMEMORY_URL", v) },
            None => unsafe { std::env::remove_var("AGENTMEMORY_URL") },
        }
    }

    #[tokio::test]
    async fn health_check_unreachable_returns_false() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        assert!(!backend.health_check().await);
    }

    #[tokio::test]
    async fn store_unreachable_errors_gracefully() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        let result = backend
            .store("k", "some fact", MemoryCategory::Core, Some("s-1"))
            .await;
        assert!(result.is_err(), "unreachable server must error, not panic");
    }

    #[tokio::test]
    async fn store_rejects_empty_content() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        let result = backend.store("k", "  ", MemoryCategory::Core, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn recall_unreachable_errors_gracefully() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        let result = backend.recall("query", 5, None, None, None).await;
        assert!(result.is_err(), "unreachable server must error, not panic");
    }

    #[tokio::test]
    async fn forget_and_count_explain_unsupported() {
        let backend = AgentMemory::with_url("http://127.0.0.1:1", None);
        let forget_err = backend.forget("k").await.unwrap_err().to_string();
        assert!(forget_err.contains("does not expose per-key deletion"));
        let count_err = backend.count().await.unwrap_err().to_string();
        assert!(count_err.contains("does not expose a total memory count"));
    }

    #[test]
    fn entry_from_result_maps_common_shape() {
        let result = serde_json::json!({
            "id": "m-1",
            "content": "We chose jose middleware for auth",
            "type": "core",
            "sessionId": "sess-9",
            "score": 0.92,
            "timestamp": "2026-01-02T03:04:05Z"
        });
        let entry = AgentMemory::entry_from_result(&result, 0);
        assert_eq!(entry.id, "m-1");
        assert_eq!(entry.content, "We chose jose middleware for auth");
        assert_eq!(entry.category, MemoryCategory::Core);
        assert_eq!(entry.session_id.as_deref(), Some("sess-9"));
        assert_eq!(entry.score, Some(0.92));
        assert_eq!(entry.timestamp, "2026-01-02T03:04:05Z");
        assert_eq!(entry.namespace, "default");
    }

    #[test]
    fn entry_from_result_falls_back_to_summary_and_synthetic_id() {
        let result = serde_json::json!({ "summary": "Rate limiting via token bucket" });
        let entry = AgentMemory::entry_from_result(&result, 3);
        assert_eq!(entry.id, "agentmemory-3");
        assert_eq!(entry.content, "Rate limiting via token bucket");
        assert_eq!(entry.category, MemoryCategory::Conversation);
    }

    #[test]
    fn entry_within_time_bounds_applies_inclusive_rfc3339_filter() {
        let entry = |ts: &str| MemoryEntry {
            id: "x".into(),
            key: "x".into(),
            content: "c".into(),
            category: MemoryCategory::Core,
            timestamp: ts.into(),
            session_id: None,
            score: None,
            namespace: "default".into(),
            importance: None,
            superseded_by: None,
        };
        let e = entry("2026-03-10T00:00:00Z");
        // No bounds → always in range.
        assert!(AgentMemory::entry_within_time_bounds(&e, None, None));
        // Inclusive bounds: equal edges pass, outside fails.
        assert!(AgentMemory::entry_within_time_bounds(
            &e,
            Some("2026-03-10T00:00:00Z"),
            Some("2026-03-10T00:00:00Z")
        ));
        assert!(!AgentMemory::entry_within_time_bounds(
            &e,
            Some("2026-03-11T00:00:00Z"),
            None
        ));
        assert!(!AgentMemory::entry_within_time_bounds(
            &e,
            None,
            Some("2026-03-09T00:00:00Z")
        ));
        // Empty strings are treated as absent bounds.
        assert!(AgentMemory::entry_within_time_bounds(
            &e,
            Some(""),
            Some("")
        ));
    }

    #[test]
    fn entry_from_result_empty_content_gets_synthetic_key() {
        let result = serde_json::json!({});
        let entry = AgentMemory::entry_from_result(&result, 0);
        assert!(!entry.key.is_empty(), "key must not be empty");
    }

    #[test]
    fn search_results_accepts_known_shapes() {
        assert_eq!(
            AgentMemory::search_results(&serde_json::json!({ "results": [1, 2] })).len(),
            2
        );
        assert_eq!(
            AgentMemory::search_results(&serde_json::json!({ "memories": [1] })).len(),
            1
        );
        assert_eq!(
            AgentMemory::search_results(&serde_json::json!({ "data": [1, 2, 3] })).len(),
            3
        );
        assert_eq!(
            AgentMemory::search_results(&serde_json::json!({ "other": [] })).len(),
            0
        );
    }
}
