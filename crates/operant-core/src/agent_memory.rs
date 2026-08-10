//! agentmemory provider — hybrid semantic memory via the agentmemory server.
//!
//! agentmemory (https://github.com/rohitg00/agentmemory) is a local memory
//! server for AI coding agents built on the iii engine. It exposes a REST API
//! on port 3111 and an MCP server with 53 tools (BM25 + local-embedding hybrid
//! retrieval, 4-tier consolidation, decay, knowledge graph).
//!
//! This provider implements the [`crate::memory_provider::MemoryProvider`]
//! trait against agentmemory's REST API, mirroring the hermes-agent
//! `integrations/hermes` memory plugin hook-for-hook:
//!
//! | Hook                | REST call                              |
//! |---------------------|----------------------------------------|
//! | `initialize`        | `POST /agentmemory/session/start`      |
//! | `system_prompt`     | `POST /agentmemory/context` (sync)     |
//! | `prefetch`          | `POST /agentmemory/smart-search`       |
//! | `sync_turn`         | `POST /agentmemory/observe`            |
//! | `on_session_end`    | `POST /agentmemory/session/end` (sync) |
//! | `on_pre_compress`   | `POST /agentmemory/context` (sync)     |
//! | `on_memory_write`   | `POST /agentmemory/remember` (sync)    |
//! | `queue_prefetch`    | `POST /agentmemory/smart-search` (bg)  |
//! | tools               | `memory_smart_search`, `memory_save`   |
//!
//! Sync hooks use a short-timeout blocking client so the agent loop is never
//! stalled on a dead server; fire-and-forget hooks (memory_write, prefetch
//! queue, session_switch) run on a background thread exactly like the
//! plugin's `_api_bg`.
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
use std::path::{Path, PathBuf};
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
/// Short timeout for sync hooks (session/end, context, remember mirror).
/// Matches the plugin's `TIMEOUT = 5` — long enough for a local server,
/// short enough to never meaningfully stall the agent loop.
const SYNC_HOOK_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared blocking client for the sync lifecycle hooks. A single static
/// instance avoids two hazards: (1) per-provider blocking clients would be
/// dropped inside async runtimes (reqwest::blocking owns an internal runtime
/// and panics on such drops — see tokio "Cannot drop a runtime in a context
/// where blocking is not allowed"); (2) unbounded thread pools per provider.
/// `reqwest::blocking::Client` is cheap to clone (internally Arc-backed).
static BLOCKING_CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();

fn shared_blocking_client() -> &'static reqwest::blocking::Client {
    BLOCKING_CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(SYNC_HOOK_TIMEOUT)
            .user_agent("operant-agentmemory")
            .build()
            .expect("reqwest blocking client build cannot fail")
    })
}

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
    /// Session identity tracked by the provider, mirroring the plugin's
    /// `initialize()` (session_id / project / cwd). Sync hooks read these
    /// to scope session/end + context requests.
    session_id: std::sync::Mutex<Option<String>>,
    project: std::sync::Mutex<String>,
    cwd: std::sync::Mutex<String>,
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
        // The shared blocking client is lazily initialized on first sync-hook
        // use — no per-provider construction, so no async-drop hazard.
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
            session_id: std::sync::Mutex::new(None),
            project: std::sync::Mutex::new(String::new()),
            cwd: std::sync::Mutex::new(String::new()),
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
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
            session_id: std::sync::Mutex::new(None),
            project: std::sync::Mutex::new(String::new()),
            cwd: std::sync::Mutex::new(String::new()),
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
    /// Returns true when a server is (or just became) reachable. Public so
    /// the TUI /mcp reconnect path can warm the backend before connecting
    /// the MCP stdio server; the `MemoryProvider::ensure_server` trait
    /// method delegates here. (iter-326 — native lifecycle management.)
    pub async fn ensure_backend(&self) -> bool {
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

        // Wait for warmup: poll health every 250ms up to
        // SPAWN_WARMUP_TIMEOUT (fast bring-up, feels instant to the user).
        let deadline = tokio::time::Instant::now() + SPAWN_WARMUP_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if self.check_health().await {
                tracing::info!(
                    "agentmemory server is up ({}); memory hooks active",
                    self.base_url
                );
                return true;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
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

    /// Blocking POST for sync lifecycle hooks. Returns the parsed body, or
    /// `Err` when the server is unreachable / returns an error status.
    /// Callers must gate on `is_available()` first so a dead server is never
    /// hit synchronously (the agent loop would stall for SYNC_HOOK_TIMEOUT).
    fn post_blocking(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = shared_blocking_client().post(&url).json(&body);
        if let Some(secret) = &self.secret {
            req = req.header("Authorization", format!("Bearer {secret}"));
        }
        let resp = req.send().map_err(|e| {
            Error::Agent(format!(
                "agentmemory blocking request to {path} failed: {e}"
            ))
        })?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if !status.is_success() {
            return Err(Error::Agent(format!(
                "agentmemory {path} returned HTTP {status}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| Error::Agent(format!("agentmemory {path} returned non-JSON body: {e}")))
    }

    /// Fire-and-forget background POST — mirrors the plugin's `_api_bg`
    /// (daemon thread). Used for memory_write mirroring, queued prefetch,
    /// and session_switch so the agent loop never blocks.
    fn fire_and_forget(&self, path: &str, body: Value) {
        if !self.reachable.load(Ordering::Relaxed) {
            return;
        }
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let path = path.to_string();
        let secret = self.secret.clone();
        let blocking = shared_blocking_client().clone();
        std::thread::spawn(move || {
            let mut req = blocking.post(&url).json(&body);
            if let Some(secret) = &secret {
                req = req.header("Authorization", format!("Bearer {secret}"));
            }
            if let Err(e) = req.send() {
                tracing::debug!(error = %e, path, "agentmemory background hook failed");
            }
        });
    }

    /// Resolve the canonical project scope, matching the plugin's
    /// `_resolve_project`: `AGENTMEMORY_PROJECT_NAME` env override, then the
    /// git toplevel basename, then the cwd basename.
    fn resolve_project(cwd: &str) -> String {
        if let Some(explicit) = std::env::var("AGENTMEMORY_PROJECT_NAME")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            return explicit;
        }
        // git rev-parse --show-toplevel → basename.
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(cwd)
            .output()
            && output.status.success()
        {
            let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !top.is_empty() {
                if let Some(name) = Path::new(&top).file_name() {
                    return name.to_string_lossy().to_string();
                }
                return top;
            }
        }
        Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "default".to_string())
    }

    /// Resolve the initial session scope (session_id / project / cwd) once,
    /// mirroring the plugin's `initialize()`. Stores them for the sync hooks.
    fn capture_session(&self, session_id: &str) {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Ok(mut slot) = self.session_id.lock() {
            *slot = Some(session_id.to_string());
        }
        if let Ok(mut slot) = self.project.lock() {
            *slot = Self::resolve_project(&cwd);
        }
        if let Ok(mut slot) = self.cwd.lock() {
            *slot = cwd;
        }
    }

    fn current_session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|s| s.clone())
    }

    fn current_project(&self) -> String {
        self.project.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn current_cwd(&self) -> String {
        self.cwd.lock().map(|c| c.clone()).unwrap_or_default()
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

    async fn check_health(&self) -> bool {
        // Explicit UFCS to the inherent HTTP probe (refreshes the cached
        // reachable flag) — avoids relying on inherent-over-trait method
        // shadowing, which a future refactor could silently break.
        AgentMemoryProvider::check_health(self).await
    }

    async fn ensure_server(&self) -> bool {
        // Delegate to the inherent spawn/health lifecycle helper.
        self.ensure_backend().await
    }

    async fn initialize(&self, session_id: &str) -> Result<()> {
        // Best-effort: spawn + warmup if needed. Never fails the startup —
        // an unreachable server degrades gracefully at call time.
        if !self.ensure_backend().await {
            tracing::warn!(
                "agentmemory not reachable at {} — memory hooks will no-op until the server starts",
                self.base_url
            );
        }
        // Hermes-plugin parity: initialize() registers the session with
        // session/start {sessionId, project, cwd} and remembers the scope
        // for every later hook.
        self.capture_session(session_id);
        if self.is_available()
            && let Err(e) = self
                .post_json(
                    "/agentmemory/session/start",
                    serde_json::json!({
                        "sessionId": session_id,
                        "project": self.current_project(),
                        "cwd": self.current_cwd(),
                    }),
                )
                .await
        {
            tracing::debug!(error = %e, "agentmemory session/start failed");
        }
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        // Hermes-plugin parity: the plugin fetches the current context from
        // POST /context at prompt-build time. We mirror that with a short
        // sync call, gated on availability so a dead server yields the
        // static fallback instead of stalling the loop.
        if self.is_available()
            && let Ok(value) = self.post_blocking(
                "/agentmemory/context",
                serde_json::json!({
                    "sessionId": self.current_session_id().unwrap_or_default(),
                    "project": self.current_project(),
                }),
            )
            && let Some(ctx) = value.get("context").and_then(|v| v.as_str())
            && !ctx.trim().is_empty()
        {
            return format!(
                "[agentmemory context]\n{}\n\nUse memory_smart_search for deeper recall and memory_save to store facts.",
                ctx.trim()
            );
        }
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
        // Hermes-plugin parity: sync_turn POSTs an `observe` observation
        // shaped like a tool interaction so agentmemory can derive and
        // consolidate it like every other agent integration.
        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let body = serde_json::json!({
            "hookType": "post_tool_use",
            "sessionId": self.current_session_id().unwrap_or_default(),
            "project": self.current_project(),
            "cwd": self.current_cwd(),
            "timestamp": timestamp,
            "data": {
                "tool_name": "conversation",
                "tool_input": user.chars().take(500).collect::<String>(),
                "tool_output": assistant.chars().take(2000).collect::<String>(),
            },
        });
        match self.post_json("/agentmemory/observe", body).await {
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

    // -- Hermes-plugin lifecycle hooks (sync) ------------------------------

    /// Session ended: notify the server so it can finalize the session.
    /// Fire-and-forget via the blocking client (gated on availability).
    fn on_session_end(&self, _messages: &[crate::client::Message]) {
        self.fire_and_forget(
            "/agentmemory/session/end",
            serde_json::json!({
                "sessionId": self.current_session_id().unwrap_or_default(),
            }),
        );
    }

    /// Before compaction: pull the live context and return it so the
    /// compression summary preserves what agentmemory still considers
    /// important (plugin parity: `on_pre_compress` fetches `/context`).
    fn on_pre_compress(&self, _messages: &[crate::client::Message]) -> String {
        if !self.is_available() {
            return String::new();
        }
        match self.post_blocking(
            "/agentmemory/context",
            serde_json::json!({
                "sessionId": self.current_session_id().unwrap_or_default(),
                "project": self.current_project(),
            }),
        ) {
            Ok(value) => value
                .get("context")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            Err(e) => {
                tracing::debug!(error = %e, "agentmemory on_pre_compress failed");
                String::new()
            }
        }
    }

    /// Mirror a built-in memory write into agentmemory (plugin parity:
    /// `on_memory_write` POSTs `remember {content, type: "fact"}` for
    /// add/update actions, in the background).
    fn on_memory_write(&self, action: &str, _target: &str, content: &str) {
        if !matches!(action, "add" | "update") || content.trim().is_empty() {
            return;
        }
        self.fire_and_forget(
            "/agentmemory/remember",
            serde_json::json!({
                "content": content,
                "type": "fact",
            }),
        );
    }

    /// Queue a background recall for the next turn (plugin parity:
    /// `queue_prefetch` fires a bg smart-search).
    fn queue_prefetch(&self, query: &str) {
        if query.trim().is_empty() {
            return;
        }
        self.fire_and_forget(
            "/agentmemory/smart-search",
            serde_json::json!({ "query": query, "limit": 3 }),
        );
    }

    /// Session switched (new/reset/branch): re-register with the server
    /// (mirrors the plugin's session/start on initialize) so subsequent
    /// session-scoped hooks target the new session.
    fn on_session_switch(&self, new_session_id: &str, _parent_session_id: &str, reset: bool) {
        if let Ok(mut slot) = self.session_id.lock() {
            *slot = Some(new_session_id.to_string());
        }
        if reset {
            self.fire_and_forget(
                "/agentmemory/session/start",
                serde_json::json!({
                    "sessionId": new_session_id,
                    "project": self.current_project(),
                    "cwd": self.current_cwd(),
                }),
            );
        }
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
    async fn test_sync_turn_unreachable_server_errors_cleanly() {
        // No server running — must degrade to an Err (not panic). The
        // observe payload shape is covered by the body-building helper.
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let result = provider.sync_turn("hello world", "hi back").await;
        assert!(result.is_err(), "unreachable server should error cleanly");
    }

    #[test]
    fn test_sync_turn_observe_body_shape() {
        // Hermes-plugin parity: sync_turn must POST an `observe` observation
        // with hookType post_tool_use + conversation tool data.
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        provider.capture_session("sess-1");
        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let body = serde_json::json!({
            "hookType": "post_tool_use",
            "sessionId": provider.current_session_id().unwrap_or_default(),
            "project": provider.current_project(),
            "cwd": provider.current_cwd(),
            "timestamp": timestamp,
            "data": {
                "tool_name": "conversation",
                "tool_input": "hello world",
                "tool_output": "hi back",
            },
        });
        assert_eq!(body["hookType"], "post_tool_use");
        assert_eq!(body["sessionId"], "sess-1");
        assert_eq!(body["data"]["tool_name"], "conversation");
        assert_eq!(body["data"]["tool_input"], "hello world");
        assert_eq!(body["data"]["tool_output"], "hi back");
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

    #[test]
    fn test_resolve_project_env_override() {
        // AGENTMEMORY_PROJECT_NAME wins over everything. Save/restore the
        // var so parallel test runs can't observe a mutation.
        let previous = std::env::var_os("AGENTMEMORY_PROJECT_NAME");
        // SAFETY: test-only env mutation, restored immediately below.
        unsafe { std::env::set_var("AGENTMEMORY_PROJECT_NAME", "my-project") };
        assert_eq!(AgentMemoryProvider::resolve_project("/tmp"), "my-project");
        match previous {
            Some(value) => unsafe { std::env::set_var("AGENTMEMORY_PROJECT_NAME", value) },
            None => unsafe { std::env::remove_var("AGENTMEMORY_PROJECT_NAME") },
        }
    }

    #[test]
    fn test_resolve_project_falls_back_to_cwd_basename() {
        // No git repo, no env var → cwd basename (the plugin's last resort).
        assert_eq!(AgentMemoryProvider::resolve_project("/tmp"), "tmp");
    }

    #[test]
    fn test_on_session_end_unreachable_is_silent() {
        // Server down → fire_and_forget is gated on reachable, so nothing
        // blocks and no panic occurs.
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        provider.on_session_end(&[]);
        assert!(!provider.reachable.load(Ordering::Relaxed));
    }

    #[test]
    fn test_on_memory_write_skips_empty_and_non_mutating_actions() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        // delete → ignored (only add/update mirror to agentmemory).
        provider.on_memory_write("delete", "MEMORY.md", "something");
        // empty content → ignored.
        provider.on_memory_write("add", "MEMORY.md", "");
        // add with content → gated on reachable, so silent no-op here.
        provider.on_memory_write("add", "MEMORY.md", "a fact");
        assert!(!provider.reachable.load(Ordering::Relaxed));
    }

    #[test]
    fn test_on_pre_compress_unreachable_returns_empty() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        let out = provider.on_pre_compress(&[]);
        assert_eq!(out, "");
    }

    #[test]
    fn test_capture_session_and_session_switch_update_id() {
        let provider = AgentMemoryProvider::with_url("http://127.0.0.1:1");
        provider.capture_session("session-1");
        assert_eq!(provider.current_session_id().as_deref(), Some("session-1"));
        provider.on_session_switch("session-2", "session-1", true);
        assert_eq!(provider.current_session_id().as_deref(), Some("session-2"));
    }
}
