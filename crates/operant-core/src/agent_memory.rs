//! agentmemory provider — hybrid semantic memory via the agentmemory server.
//!
//! agentmemory (https://github.com/rohitg00/agentmemory) is a local memory
//! server for AI coding agents built on the iii engine. It exposes a REST API
//! on port 3111 and an MCP server with 53 tools (BM25 + local-embedding hybrid
//! retrieval, 4-tier consolidation, decay, knowledge graph).
//!
//! This provider implements the [`crate::memory_provider::MemoryProvider`]
//! trait against agentmemory's REST API:
//!
//! | Hook          | REST call                                   |
//! |---------------|---------------------------------------------|
//! | `prefetch`    | `POST /agentmemory/smart-search`            |
//! | `sync_turn`   | `POST /agentmemory/remember`                |
//! | tools         | `memory_smart_search`, `memory_save` (+MCP) |
//!
//! ## Server lifecycle
//!
//! When `memory.agentmemory_auto_spawn` is enabled (default) and the health
//! endpoint is unreachable, the provider spawns
//! `npx -y @agentmemory/agentmemory@latest` as a child process, waits for it
//! to come up (up to 60s), and kills it on `shutdown()`. If the server can't
//! be reached at all, every hook degrades gracefully (empty prefetch, no-op
//! sync) instead of failing the agent loop.

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::runtime_config;
use crate::error::{Error, Result};

/// Default agentmemory server base URL.
pub const DEFAULT_AGENTMEMORY_URL: &str = "http://localhost:3111";

/// How long to wait for the auto-spawned server to become reachable.
const SPAWN_WARMUP_TIMEOUT: Duration = Duration::from_secs(60);
/// Per-request timeout for REST calls.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// REST client for the agentmemory server.
pub struct AgentMemoryProvider {
    client: reqwest::Client,
    base_url: String,
    secret: Option<String>,
    auto_spawn: bool,
    /// Handle to the auto-spawned server process (None when an external
    /// server is used or spawn failed).
    spawned: Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
    /// Cached reachability flag — set after a successful health check and
    /// cleared after a failed one. Avoids hammering the health endpoint.
    reachable: Arc<AtomicBool>,
}

impl std::fmt::Debug for AgentMemoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMemoryProvider")
            .field("base_url", &self.base_url)
            .field("auto_spawn", &self.auto_spawn)
            .field("reachable", &self.reachable.load(Ordering::Relaxed))
            .finish()
    }
}

impl AgentMemoryProvider {
    /// Create a provider from the runtime config `[memory]` section.
    /// `storage_dir` is accepted for signature parity with other providers;
    /// agentmemory keeps its own state under `~/.agentmemory`.
    pub fn new(_storage_dir: PathBuf) -> Result<Self> {
        let mem = &runtime_config().memory;
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("operant-agentmemory")
            .build()
            .map_err(|e| Error::Agent(format!("failed to build agentmemory HTTP client: {e}")))?;
        Ok(Self {
            client,
            base_url: mem
                .agentmemory_url
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_AGENTMEMORY_URL.to_string()),
            secret: mem.agentmemory_secret.clone(),
            auto_spawn: mem.agentmemory_auto_spawn.unwrap_or(true),
            spawned: Arc::new(tokio::sync::Mutex::new(None)),
            reachable: Arc::new(AtomicBool::new(false)),
        })
    }

        #[expect(clippy::expect_used, reason = "invariant guaranteed by surrounding validation")]
    /// Create a provider pointing at a custom base URL (used by tests).
    pub fn with_url(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .user_agent("operant-agentmemory")
                .build()
                .expect("reqwest client build cannot fail"),
            base_url: base_url.into(),
            secret: None,
            auto_spawn: false,
            spawned: Arc::new(tokio::sync::Mutex::new(None)),
            reachable: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Full health endpoint URL.
    fn health_url(&self) -> String {
        format!("{}/agentmemory/health", self.base_url.trim_end_matches('/'))
    }

    /// Check whether the server is reachable. Updates the cached flag.
    pub async fn check_health(&self) -> bool {
        let url = self.health_url();
        let ok = match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };
        self.reachable.store(ok, Ordering::Relaxed);
        ok
    }

    /// Auto-spawn the agentmemory server if enabled and not already running.
    /// Returns true when a server is (or just became) reachable.
    async fn ensure_server(&self) -> bool {
        if self.check_health().await {
            return true;
        }
        if !self.auto_spawn {
            return false;
        }
        // Guard against double-spawn with a lock held across the spawn.
        let mut spawned = self.spawned.lock().await;
        if let Some(child) = spawned.as_mut() {
            // Already spawned — check if it's still alive.
            if child.try_wait().ok().flatten().is_none() {
                // Poll warmup below.
            } else {
                *spawned = None;
            }
        }
        if spawned.is_none() {
            tracing::info!(
                "agentmemory server unreachable — auto-spawning npx @agentmemory/agentmemory"
            );
            let mut cmd = tokio::process::Command::new("npx");
            cmd.args(["-y", "@agentmemory/agentmemory@latest"])
                .env("CI", "1")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            match cmd.spawn() {
                Ok(child) => {
                    *spawned = Some(child);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "agentmemory auto-spawn failed (is Node.js/npx installed?)"
                    );
                    return false;
                }
            }
        }
        drop(spawned);

        // Wait for warmup: poll health every 1s up to SPAWN_WARMUP_TIMEOUT.
        let deadline = tokio::time::Instant::now() + SPAWN_WARMUP_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if self.check_health().await {
                tracing::info!(
                    "agentmemory server is up ({}); memory hooks active",
                    self.base_url
                );
                return true;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        tracing::warn!(
            "agentmemory server did not become reachable within {SPAWN_WARMUP_TIMEOUT:?}"
        );
        false
    }

    /// POST a JSON body to an agentmemory REST path; returns the parsed body.
    async fn post_json(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self.client.post(&url).json(&body);
        if let Some(secret) = &self.secret {
            req = req.header("Authorization", format!("Bearer {secret}"));
        }
        let resp = req.send().await.map_err(|e| {
            self.reachable.store(false, Ordering::Relaxed);
            Error::Agent(format!("agentmemory request to {path} failed: {e}"))
        })?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::Agent(format!(
                "agentmemory {path} returned HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| Error::Agent(format!("agentmemory {path} returned non-JSON body: {e}")))
    }

    /// Format a smart-search response into a compact text block for prefetch.
    fn format_search_results(value: &Value) -> String {
        let mut out: Vec<String> = Vec::new();
        let results = value
            .get("results")
            .or_else(|| value.get("memories"))
            .or_else(|| value.get("data"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if results.is_empty() {
            // Fall back to a single text field if the shape is unknown.
            if let Some(text) = value
                .get("content")
                .or_else(|| value.get("text"))
                .and_then(|v| v.as_str())
            {
                out.push(text.trim().to_string());
            }
            return out.join("\n");
        }
        for r in results.iter().take(10) {
            let text = r
                .get("content")
                .or_else(|| r.get("text"))
                .or_else(|| r.get("summary"))
                .or_else(|| r.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(text) = text {
                out.push(format!("- {text}"));
            }
        }
        out.join("\n")
    }

    /// Kill the auto-spawned server process, if any.
    pub async fn stop_server(&self) {
        let mut spawned = self.spawned.lock().await;
        if let Some(mut child) = spawned.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            self.reachable.store(false, Ordering::Relaxed);
            tracing::info!("agentmemory auto-spawned server stopped");
        }
    }
}

#[async_trait]
impl crate::memory_provider::MemoryProvider for AgentMemoryProvider {
    fn name(&self) -> &str {
        "agentmemory"
    }

    fn is_available(&self) -> bool {
        self.reachable.load(Ordering::Relaxed)
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        // Best-effort: spawn + warmup if needed. Never fails the startup —
        // an unreachable server degrades gracefully at call time.
        if !self.ensure_server().await {
            tracing::warn!(
                "agentmemory not reachable at {} — memory hooks will no-op until the server starts",
                self.base_url
            );
        }
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "agentmemory hybrid memory active. You can recall past work with memory_smart_search and save facts with memory_save. Recall is automatic each turn; if nothing is retrieved, that is normal for new topics.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        if query.trim().is_empty() {
            return String::new();
        }
        match self
            .post_json(
                "/agentmemory/smart-search",
                serde_json::json!({ "query": query, "limit": 5 }),
            )
            .await
        {
            Ok(value) => {
                let text = Self::format_search_results(&value);
                if text.is_empty() {
                    String::new()
                } else {
                    format!("[agentmemory]\n{text}")
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "agentmemory prefetch failed");
                String::new()
            }
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let content = if assistant.trim().is_empty() {
            user.to_string()
        } else {
            format!("User: {user}\nAssistant: {assistant}")
        };
        // Derive lightweight concepts from the user's own words (first ~3
        // non-trivial tokens) so remember() has something to index on.
        let concepts: Vec<String> = user
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 3)
            .take(3)
            .map(|w| w.to_ascii_lowercase())
            .collect();
        let body = serde_json::json!({
            "content": content.chars().take(4000).collect::<String>(),
            "concepts": concepts,
        });
        match self.post_json("/agentmemory/remember", body).await {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::Agent(format!("agentmemory sync_turn failed: {e}"))),
        }
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "memory_smart_search",
                "description": "Semantic + keyword hybrid search across long-term memory (agentmemory). Returns past work, decisions, and facts relevant to the query.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to recall"},
                        "limit": {"type": "integer", "description": "Max results (default 5)"}
                    },
                    "required": ["query"]
                }
            }),
            serde_json::json!({
                "name": "memory_save",
                "description": "Save a fact, decision, or piece of work to long-term memory (agentmemory) so future sessions can recall it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "The memory content"},
                        "concepts": {"type": "array", "items": {"type": "string"}, "description": "Optional keywords to index"}
                    },
                    "required": ["content"]
                }
            }),
        ]
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        match name {
            "memory_smart_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5)
                    .clamp(1, 20);
                match self
                    .post_json(
                        "/agentmemory/smart-search",
                        serde_json::json!({ "query": query, "limit": limit }),
                    )
                    .await
                {
                    Ok(value) => serde_json::json!({ "results": value }).to_string(),
                    Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                }
            }
            "memory_save" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.is_empty() {
                    return serde_json::json!({ "error": "content is required" }).to_string();
                }
                let concepts = args
                    .get("concepts")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                match self
                    .post_json(
                        "/agentmemory/remember",
                        serde_json::json!({ "content": content, "concepts": concepts }),
                    )
                    .await
                {
                    Ok(value) => serde_json::json!({ "saved": true, "detail": value }).to_string(),
                    Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                }
            }
            _ => serde_json::json!({ "error": format!("unknown tool {name}") }).to_string(),
        }
    }

    async fn shutdown(&self) {
        self.stop_server().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_provider::MemoryProvider;

    #[test]
    fn test_format_search_results_known_shape() {
        let value = serde_json::json!({
            "results": [
                {"content": "We chose jose middleware for auth"},
                {"summary": "Rate limiting via token bucket"},
                {"title": "A title only"},
                {"other": "ignored"}
            ]
        });
        let text = AgentMemoryProvider::format_search_results(&value);
        assert!(text.contains("jose middleware"));
        assert!(text.contains("token bucket"));
        assert!(text.contains("A title only"));
        assert!(!text.contains("ignored"));
    }

    #[test]
    fn test_format_search_results_unknown_shape_falls_back_to_text() {
        let value = serde_json::json!({ "content": "flat response body" });
        let text = AgentMemoryProvider::format_search_results(&value);
        assert_eq!(text, "flat response body");
    }

    #[test]
    fn test_format_search_results_empty() {
        let value = serde_json::json!({ "results": [] });
        assert_eq!(AgentMemoryProvider::format_search_results(&value), "");
    }

    #[tokio::test]
    async fn test_sync_turn_builds_body_and_sends() {
        // No server running — must degrade to an Err (not panic).
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let result = provider.sync_turn("hello world", "hi back").await;
        assert!(result.is_err(), "unreachable server should error cleanly");
    }

    #[tokio::test]
    async fn test_prefetch_unreachable_returns_empty() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let text = provider.prefetch("anything").await;
        assert_eq!(text, "");
    }

    #[tokio::test]
    async fn test_handle_tool_call_unknown_tool() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let out = provider
            .handle_tool_call("nope", serde_json::json!({}))
            .await;
        assert!(out.contains("unknown tool"));
    }

    #[tokio::test]
    async fn test_memory_save_empty_content_rejected() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let out = provider
            .handle_tool_call("memory_save", serde_json::json!({ "content": "" }))
            .await;
        assert!(out.contains("content is required"));
    }
}
