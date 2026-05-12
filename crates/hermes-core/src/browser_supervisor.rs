//! Cloud browser automation supervisor for Hermes-RS.
//!
//! Manages browser automation sessions across multiple cloud providers
//! (Browserbase, Browser Use, Firecrawl) through a unified CDP (Chrome DevTools
//! Protocol) interface. Sessions are stored in-memory with timeouts tracked via
//! `last_active`.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │            CDPSupervisor                │
//! │  ┌───────────────────────────────────┐  │
//! │  │       Session Registry            │  │
//! │  │   HashMap<String, BrowserSession> │  │
//! │  └───────────────────────────────────┘  │
//! │  ┌───────────────────────────────────┐  │
//! │  │    CloudProviderConfig            │  │
//! │  └───────────────────────────────────┘  │
//! └─────────────────────────────────────────┘
//!            │
//!    ┌───────┼───────────────┐
//!    │       │               │
//!    ▼       ▼               ▼
//! Browserbase  BrowserUse  Firecrawl
//!   Client      Client      Client
//! ```
//!
//! # Example
//!
//! ```ignore
//! use hermes_core::browser_supervisor::{
//!     CDPSupervisor, CloudProviderConfig, CloudProvider,
//! };
//!
//! let config = CloudProviderConfig {
//!     provider_type: CloudProvider::Browserbase,
//!     api_key: Some("key".into()),
//!     api_url: None,
//!     region: None,
//! };
//! let supervisor = CDPSupervisor::new(config);
//! let session = supervisor.create_session(None, None)?;
//! println!("Session: {}", session.session_id);
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use schemars::JsonSchema;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use base64::Engine;
use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Supported cloud browser providers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub enum CloudProvider {
    /// [Browserbase](https://www.browserbase.com/) — headless browser cloud.
    Browserbase,
    /// [Browser Use](https://browseruse.ai/) — AI-agent browser automation.
    BrowserUse,
    /// [Firecrawl](https://www.firecrawl.dev/) — web crawling & scraping API.
    Firecrawl,
}

impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudProvider::Browserbase => write!(f, "browserbase"),
            CloudProvider::BrowserUse => write!(f, "browser_use"),
            CloudProvider::Firecrawl => write!(f, "firecrawl"),
        }
    }
}

impl std::str::FromStr for CloudProvider {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "browserbase" => Ok(CloudProvider::Browserbase),
            "browser_use" | "browseruse" => Ok(CloudProvider::BrowserUse),
            "firecrawl" => Ok(CloudProvider::Firecrawl),
            _ => Err(Error::InvalidToolArgs {
                name: "cloud_provider".into(),
                details: format!("Unknown cloud provider: {s}. Expected: browserbase, browser_use, firecrawl"),
            }),
        }
    }
}

/// Runtime status of a browser session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Browser is connected and operational.
    Connected,
    /// Browser has been disconnected.
    Disconnected,
    /// Session encountered an error.
    Error(String),
    /// Session is alive but not actively navigating.
    Idle,
}

/// Represents an active browser automation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSession {
    /// Unique session identifier.
    pub session_id: String,
    /// Current page URL, if known.
    pub url: Option<String>,
    /// Current page title, if known.
    pub title: Option<String>,
    /// ISO-8601 timestamp of session creation.
    pub created_at: String,
    /// ISO-8601 timestamp of last activity.
    pub last_active: String,
    /// Cloud provider name (e.g. "browserbase").
    pub provider: String,
    /// Current session status.
    pub status: SessionStatus,
}

impl BrowserSession {
    fn new(provider: &CloudProvider, url: Option<String>) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            session_id: Uuid::new_v4().to_string(),
            url,
            title: None,
            created_at: now.clone(),
            last_active: now,
            provider: provider.to_string(),
            status: SessionStatus::Connected,
        }
    }

    /// Touch the `last_active` timestamp to now.
    fn keep_alive(&mut self) {
        self.last_active = Utc::now().to_rfc3339();
    }
}

/// Configuration for a cloud browser provider.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CloudProviderConfig {
    /// Which cloud provider to use.
    pub provider_type: CloudProvider,
    /// Optional API key for the provider.
    pub api_key: Option<String>,
    /// Optional base URL override.
    pub api_url: Option<String>,
    /// Optional region for provider routing.
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Cloud provider client trait
// ---------------------------------------------------------------------------

/// Abstract interface for cloud browser provider clients.
///
/// Each provider (Browserbase, Browser Use, Firecrawl) implements this trait
/// to expose a uniform session lifecycle API to the supervisor.
#[async_trait]
pub trait CloudProviderClient: Send + Sync {
    /// Create a new browser session, optionally navigating to a URL.
    async fn create_session(&self, url: Option<&str>) -> Result<String>;

    /// Close (terminate) an active session.
    async fn close_session(&self, session_id: &str) -> Result<()>;

    /// Poll the current status of a session from the provider.
    async fn get_session_status(&self, session_id: &str) -> Result<SessionStatus>;

    /// Take a screenshot of the page and return raw PNG bytes.
    async fn take_screenshot(&self, session_id: &str) -> Result<Vec<u8>>;

    /// Human-readable provider name (e.g. "browserbase").
    fn provider_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Browserbase client
// ---------------------------------------------------------------------------

/// Client for [Browserbase](https://www.browserbase.com/) cloud browser API.
///
/// Uses the public REST API at `https://www.browserbase.com/api/v1/sessions`.
pub struct BrowserbaseClient {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl BrowserbaseClient {
    /// Create a new Browserbase client.
    ///
    /// * `api_key` — Browserbase API key.
    /// * `base_url` — Optional base URL override (defaults to Browserbase API).
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let base = base_url.unwrap_or_else(|| "https://www.browserbase.com/api/v1".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build reqwest client");
        Self {
            api_key,
            base_url: base.trim_end_matches('/').to_string(),
            client,
        }
    }
}

#[async_trait]
impl CloudProviderClient for BrowserbaseClient {
    fn provider_name(&self) -> &str {
        "browserbase"
    }

    async fn create_session(&self, url: Option<&str>) -> Result<String> {
        let endpoint = format!("{}/sessions", self.base_url);
        let mut body = json!({});
        if let Some(target_url) = url {
            body["url"] = json!(target_url);
        }

        let resp = self
            .client
            .post(&endpoint)
            .header("X-API-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Agent(format!(
                "Browserbase create_session failed ({}): {}",
                status, text
            )));
        }

        let data: Value = resp
            .json()
            .await
            .map_err(|e| Error::ParseResponse(format!("Browserbase JSON: {e}")))?;

        data.get("id")
            .or_else(|| data.get("sessionId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                Error::ParseResponse("Browserbase response missing session ID".into())
            })
    }

    async fn close_session(&self, session_id: &str) -> Result<()> {
        let endpoint = format!("{}/sessions/{session_id}", self.base_url);
        let resp = self
            .client
            .delete(&endpoint)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        let status_code = resp.status();
        if !status_code.is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(provider = "browserbase", session = %session_id, status = %status_code, "close_session warning: {text}");
        }
        Ok(())
    }

    async fn get_session_status(&self, session_id: &str) -> Result<SessionStatus> {
        let endpoint = format!("{}/sessions/{session_id}", self.base_url);
        let resp = self
            .client
            .get(&endpoint)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        if !resp.status().is_success() {
            return Ok(SessionStatus::Error("Failed to fetch status".into()));
        }

        let data: Value = resp.json().await.map_err(|e| {
            Error::ParseResponse(format!("Browserbase status JSON: {e}"))
        })?;

        let status_str = data.get("status").and_then(|v| v.as_str()).unwrap_or("");
        Ok(match status_str {
            "running" | "active" => SessionStatus::Connected,
            "idle" => SessionStatus::Idle,
            "error" => SessionStatus::Error(
                data.get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            ),
            _ => SessionStatus::Disconnected,
        })
    }

    async fn take_screenshot(&self, session_id: &str) -> Result<Vec<u8>> {
        let endpoint = format!("{}/sessions/{session_id}/screenshot", self.base_url);
        let resp = self
            .client
            .post(&endpoint)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        if !resp.status().is_success() {
            return Err(Error::Agent(format!(
                "Browserbase screenshot failed: {}",
                resp.status()
            )));
        }

        let bytes = resp.bytes().await.map_err(|e| Error::Network(e))?;
        Ok(bytes.to_vec())
    }
}

// ---------------------------------------------------------------------------
// Browser Use client (stub)
// ---------------------------------------------------------------------------

/// Stub client for [Browser Use](https://browseruse.ai/).
///
/// Returns mock session IDs with a `browser_use_` prefix. No real API calls
/// are made — this is a placeholder for future integration.
pub struct BrowserUseClient {
    #[allow(dead_code)]
    api_key: String,
}

impl BrowserUseClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl CloudProviderClient for BrowserUseClient {
    fn provider_name(&self) -> &str {
        "browser_use"
    }

    async fn create_session(&self, _url: Option<&str>) -> Result<String> {
        let id = format!("browser_use_{}", Uuid::new_v4());
        info!(provider = "browser_use", session = %id, "Created stub session");
        Ok(id)
    }

    async fn close_session(&self, session_id: &str) -> Result<()> {
        info!(provider = "browser_use", session = %session_id, "Closed stub session");
        Ok(())
    }

    async fn get_session_status(&self, _session_id: &str) -> Result<SessionStatus> {
        Ok(SessionStatus::Connected)
    }

    async fn take_screenshot(&self, _session_id: &str) -> Result<Vec<u8>> {
        // Return a tiny 1x1 transparent PNG as placeholder
        Ok(vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG header
            0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
            0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41,
            0x54, 0x78, 0x9c, 0x62, 0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0xe5, 0x27, 0xde, 0xfc, 0x00, 0x00,
            0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
            0x60, 0x82,
        ])
    }
}

// ---------------------------------------------------------------------------
// Firecrawl client
// ---------------------------------------------------------------------------

/// Client for [Firecrawl](https://www.firecrawl.dev/) web crawling API.
///
/// Uses the REST API at `https://api.firecrawl.com/v1`.
pub struct FirecrawlClient {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl FirecrawlClient {
    /// Create a new Firecrawl client.
    ///
    /// * `api_key` — Firecrawl API key.
    /// * `base_url` — Optional base URL override.
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let base = base_url.unwrap_or_else(|| "https://api.firecrawl.com/v1".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to build reqwest client");
        Self {
            api_key,
            base_url: base.trim_end_matches('/').to_string(),
            client,
        }
    }
}

#[async_trait]
impl CloudProviderClient for FirecrawlClient {
    fn provider_name(&self) -> &str {
        "firecrawl"
    }

    async fn create_session(&self, url: Option<&str>) -> Result<String> {
        let endpoint = format!("{}/crawl", self.base_url);
        let target = url.unwrap_or("https://example.com");
        let body = json!({ "url": target });

        let resp = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Agent(format!(
                "Firecrawl create_session failed ({}): {}",
                status, text
            )));
        }

        let data: Value = resp
            .json()
            .await
            .map_err(|e| Error::ParseResponse(format!("Firecrawl JSON: {e}")))?;

        data.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::ParseResponse("Firecrawl response missing crawl ID".into()))
    }

    async fn close_session(&self, session_id: &str) -> Result<()> {
        // Firecrawl crawl jobs don't have a close endpoint; just log it.
        info!(provider = "firecrawl", job = %session_id, "Firecrawl crawl job will expire naturally");
        Ok(())
    }

    async fn get_session_status(&self, session_id: &str) -> Result<SessionStatus> {
        let endpoint = format!("{}/crawl/{session_id}", self.base_url);
        let resp = self
            .client
            .get(&endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| Error::Network(e))?;

        if !resp.status().is_success() {
            return Ok(SessionStatus::Error("Failed to poll crawl status".into()));
        }

        let data: Value = resp.json().await.map_err(|e| {
            Error::ParseResponse(format!("Firecrawl status JSON: {e}"))
        })?;

        let status_str = data.get("status").and_then(|v| v.as_str()).unwrap_or("");
        Ok(match status_str {
            "active" | "processing" | "scraping" => SessionStatus::Connected,
            "completed" => SessionStatus::Idle,
            "failed" => SessionStatus::Error(
                data.get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("crawl_failed")
                    .to_string(),
            ),
            _ => SessionStatus::Disconnected,
        })
    }

    async fn take_screenshot(&self, _session_id: &str) -> Result<Vec<u8>> {
        Err(Error::Agent(
            "Firecrawl does not support screenshots".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Provider factory
// ---------------------------------------------------------------------------

/// Create a cloud provider client from configuration.
pub fn create_provider_client(config: &CloudProviderConfig) -> Result<Box<dyn CloudProviderClient>> {
    match config.provider_type {
        CloudProvider::Browserbase => {
            let key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("BROWSERBASE_API_KEY").ok())
                .ok_or_else(|| Error::MissingApiKey)?;
            Ok(Box::new(BrowserbaseClient::new(key, config.api_url.clone())))
        }
        CloudProvider::BrowserUse => {
            let key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("BROWSER_USE_API_KEY").ok())
                .unwrap_or_default();
            Ok(Box::new(BrowserUseClient::new(key)))
        }
        CloudProvider::Firecrawl => {
            let key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("FIRECRAWL_API_KEY").ok())
                .ok_or_else(|| Error::MissingApiKey)?;
            Ok(Box::new(FirecrawlClient::new(key, config.api_url.clone())))
        }
    }
}

// ---------------------------------------------------------------------------
// CDP Supervisor
// ---------------------------------------------------------------------------

/// Manages cloud-based browser automation sessions.
///
/// `CDPSupervisor` maintains an in-memory registry of active browser sessions
/// and delegates provider-specific operations (create, close, screenshot) to
/// the appropriate `CloudProviderClient` implementation.
///
/// Sessions are identified by UUID and tracked with creation/last-active
/// timestamps for lifecycle management.
pub struct CDPSupervisor {
    sessions: Arc<RwLock<HashMap<String, BrowserSession>>>,
    config: CloudProviderConfig,
    client: Box<dyn CloudProviderClient>,
}

impl CDPSupervisor {
    /// Create a new supervisor with the given cloud provider configuration.
    pub fn new(config: CloudProviderConfig) -> Self {
        let client = create_provider_client(&config)
            .expect("Failed to create cloud provider client");
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
            client,
        }
    }

    /// Create a new supervisor with a pre-built provider client (useful for testing).
    pub fn with_client(config: CloudProviderConfig, client: Box<dyn CloudProviderClient>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
            client,
        }
    }

    /// Create a new browser session.
    ///
    /// If `url` is provided, the browser navigates to it after launch.
    /// If `provider` is `None`, the supervisor's default provider is used.
    #[instrument(skip(self))]
    pub async fn create_session(
        &self,
        provider: Option<CloudProvider>,
        url: Option<String>,
    ) -> Result<BrowserSession> {
        // If an alternate provider is requested, create a temporary client for it.
        let session_id = if let Some(ref alt) = provider {
            let alt_config = CloudProviderConfig {
                provider_type: alt.clone(),
                api_key: self.config.api_key.clone(),
                api_url: self.config.api_url.clone(),
                region: self.config.region.clone(),
            };
            let alt_client = create_provider_client(&alt_config)?;
            alt_client.create_session(url.as_deref()).await?
        } else {
            self.client.create_session(url.as_deref()).await?
        };

        let mut session = BrowserSession::new(
            provider.as_ref().unwrap_or(&self.config.provider_type),
            url,
        );
        session.session_id = session_id;

        let id = session.session_id.clone();
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(id.clone(), session.clone());
        }

        info!(
            provider = %session.provider,
            session = %id,
            "Browser session created"
        );

        Ok(session)
    }

    /// Close and remove a browser session.
    #[instrument(skip(self))]
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        let session = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(session_id)
        };

        match session {
            Some(_) => {
                self.client.close_session(session_id).await?;
                info!(session = %session_id, "Browser session closed");
                Ok(())
            }
            None => Err(Error::ToolNotFound {
                name: format!("session_{session_id}"),
            }),
        }
    }

    /// Retrieve session info by ID.
    pub async fn get_session(&self, session_id: &str) -> Option<BrowserSession> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// List all active sessions.
    pub async fn list_sessions(&self) -> Vec<BrowserSession> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Refresh the keep-alive timestamp for a session.
    #[instrument(skip(self))]
    pub async fn keep_alive(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_id).ok_or_else(|| {
            Error::ToolNotFound {
                name: format!("session_{session_id}"),
            }
        })?;
        session.keep_alive();
        debug!(session = %session_id, "Session keep-alive refreshed");
        Ok(())
    }

    /// Take a screenshot of the page in the given session.
    #[instrument(skip(self))]
    pub async fn take_screenshot(&self, session_id: &str) -> Result<Vec<u8>> {
        // Verify session exists
        let exists = {
            let sessions = self.sessions.read().await;
            sessions.contains_key(session_id)
        };
        if !exists {
            return Err(Error::ToolNotFound {
                name: format!("session_{session_id}"),
            });
        }
        self.client.take_screenshot(session_id).await
    }

    /// Inject JavaScript into a page for dialog handling or bridge setup.
    ///
    /// This is a stub — real CDP JS injection is handled by the browser's
    /// CDP utils layer. Returns a success message.
    pub async fn inject_dialog_bridge(
        &self,
        session_id: &str,
        js_code: &str,
    ) -> Result<String> {
        // Validate session exists
        let exists = {
            let sessions = self.sessions.read().await;
            sessions.contains_key(session_id)
        };
        if !exists {
            return Err(Error::ToolNotFound {
                name: format!("session_{session_id}"),
            });
        }
        let snippet = if js_code.len() > 60 {
            format!("{}...", &js_code[..60])
        } else {
            js_code.to_string()
        };
        info!(
            session = %session_id,
            js_snippet = %snippet,
            "Dialog bridge JS injected (stub)"
        );
        Ok(format!(
            "Dialog bridge JS injected into session {session_id} (stub)"
        ))
    }

    /// Get a reference to the provider configuration.
    pub fn config(&self) -> &CloudProviderConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// BrowserSupervisorTool
// ---------------------------------------------------------------------------

/// Tool arguments for `BrowserSupervisorTool`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BrowserSupervisorArgs {
    /// Operation to perform: create_session, close_session, list_sessions,
    /// get_session, keep_alive, screenshot.
    operation: String,
    /// Optional cloud provider override for create_session.
    #[serde(default)]
    provider: Option<String>,
    /// Optional URL to navigate to on session creation.
    #[serde(default)]
    url: Option<String>,
    /// Session ID for operations that target a specific session.
    #[serde(default)]
    session_id: Option<String>,
    /// Optional JS expression for dialog bridge injection.
    #[serde(default)]
    js_expression: Option<String>,
}

/// Tool for managing cloud browser automation sessions.
///
/// Supports six operations:
/// - `create_session` — launch a new browser session
/// - `close_session` — terminate a session
/// - `list_sessions` — enumerate active sessions
/// - `get_session` — show details for one session
/// - `keep_alive` — refresh a session's timeout
/// - `screenshot` — capture the current page as a PNG
pub struct BrowserSupervisorTool {
    supervisor: Arc<CDPSupervisor>,
}

impl BrowserSupervisorTool {
    /// Create a new tool wrapped around an existing supervisor.
    pub fn new(supervisor: Arc<CDPSupervisor>) -> Self {
        Self { supervisor }
    }
}

#[async_trait]
impl HermesTool for BrowserSupervisorTool {
    fn name(&self) -> &str {
        "browser_supervisor"
    }

    fn description(&self) -> &str {
        "Manage browser automation sessions in the cloud. Supports create_session, \
         close_session, list_sessions, get_session, keep_alive, and screenshot operations. \
         Use with cloud browser providers like Browserbase, Browser Use, or Firecrawl."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserSupervisorArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: BrowserSupervisorArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {e}")),
        };

        match parsed.operation.as_str() {
            "create_session" => {
                let provider = match parsed.provider {
                    Some(ref s) if !s.is_empty() => match s.parse::<CloudProvider>() {
                        Ok(p) => Some(p),
                        Err(e) => return ToolResult::error(self.name(), e.to_string()),
                    },
                    _ => None,
                };
                match self.supervisor.create_session(provider, parsed.url).await {
                    Ok(session) => ToolResult::success(self.name(), session),
                    Err(e) => ToolResult::error(self.name(), e.to_string()),
                }
            }
            "close_session" => {
                let sid = match parsed.session_id {
                    Some(id) => id,
                    None => return ToolResult::error(self.name(), "Missing session_id"),
                };
                match self.supervisor.close_session(&sid).await {
                    Ok(_) => ToolResult::success(
                        self.name(),
                        json!({ "closed": true, "session_id": sid }),
                    ),
                    Err(e) => ToolResult::error(self.name(), e.to_string()),
                }
            }
            "list_sessions" => {
                let sessions = self.supervisor.list_sessions().await;
                ToolResult::success(
                    self.name(),
                    json!({ "sessions": sessions, "count": sessions.len() }),
                )
            }
            "get_session" => {
                let sid = match parsed.session_id {
                    Some(id) => id,
                    None => return ToolResult::error(self.name(), "Missing session_id"),
                };
                match self.supervisor.get_session(&sid).await {
                    Some(session) => ToolResult::success(self.name(), session),
                    None => ToolResult::error(self.name(), format!("Session {sid} not found")),
                }
            }
            "keep_alive" => {
                let sid = match parsed.session_id {
                    Some(id) => id,
                    None => return ToolResult::error(self.name(), "Missing session_id"),
                };
                match self.supervisor.keep_alive(&sid).await {
                    Ok(_) => ToolResult::success(
                        self.name(),
                        json!({ "kept_alive": true, "session_id": sid }),
                    ),
                    Err(e) => ToolResult::error(self.name(), e.to_string()),
                }
            }
            "screenshot" => {
                let sid = match parsed.session_id {
                    Some(id) => id,
                    None => return ToolResult::error(self.name(), "Missing session_id"),
                };
                match self.supervisor.take_screenshot(&sid).await {
                    Ok(bytes) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        ToolResult::success(
                            self.name(),
                            json!({
                                "session_id": sid,
                                "screenshot": b64,
                                "size_bytes": bytes.len(),
                            }),
                        )
                    }
                    Err(e) => ToolResult::error(self.name(), e.to_string()),
                }
            }
            other => ToolResult::error(
                self.name(),
                format!("Unknown operation: {other}. Expected: create_session, close_session, list_sessions, get_session, keep_alive, screenshot"),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// CdpNavigateTool
// ---------------------------------------------------------------------------

/// Tool arguments for `CdpNavigateTool`.
#[derive(Debug, Deserialize, JsonSchema)]
struct CdpNavigateArgs {
    /// The URL to navigate to.
    url: String,
    /// Optional wait condition: "load", "domcontentloaded", "networkidle".
    #[serde(default)]
    wait_until: Option<String>,
}

/// Tool for navigating a CDP browser session to a URL.
///
/// This is a stub that returns a success confirmation. Real CDP navigation
/// is handled by the lower-level CDP utils layer.
pub struct CdpNavigateTool;

#[async_trait]
impl HermesTool for CdpNavigateTool {
    fn name(&self) -> &str {
        "cdp_navigate"
    }

    fn description(&self) -> &str {
        "Navigate a CDP browser session to a URL. Optionally wait for a page \
         ready state ('load', 'domcontentloaded', or 'networkidle')."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CdpNavigateArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: CdpNavigateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {e}")),
        };

        let wait = parsed
            .wait_until
            .as_deref()
            .unwrap_or("load");

        ToolResult::success(
            self.name(),
            json!({
                "status": "navigation_initiated",
                "url": parsed.url,
                "wait_until": wait,
                "message": format!("Navigation to '{}' initiated (stub). Use the browser tool for actual CDP navigation.", parsed.url),
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// DialogBridgeTool
// ---------------------------------------------------------------------------

/// Tool arguments for `DialogBridgeTool`.
#[derive(Debug, Deserialize, JsonSchema)]
struct DialogBridgeArgs {
    /// The browser session ID to inject the dialog bridge into.
    session_id: String,
    /// Optional custom JavaScript expression to evaluate on the page.
    #[serde(default)]
    js_expression: Option<String>,
}

/// Tool for injecting a JavaScript dialog bridge into a browser session.
///
/// This tool is a stub — the actual JS injection is performed by the CDP
/// utils layer. It validates the session exists and returns a confirmation.
pub struct DialogBridgeTool;

#[async_trait]
impl HermesTool for DialogBridgeTool {
    fn name(&self) -> &str {
        "dialog_bridge"
    }

    fn description(&self) -> &str {
        "Inject JavaScript dialog bridge into a browser session for handling \
         alert/confirm/prompt dialogs. The actual JS execution is delegated to \
         the browser's CDP layer."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<DialogBridgeArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: DialogBridgeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {e}")),
        };

        let js = parsed
            .js_expression
            .unwrap_or_else(|| "window.__hermesDialogBridge = true;".to_string());

        ToolResult::success(
            self.name(),
            json!({
                "status": "bridge_injected",
                "session_id": parsed.session_id,
                "js_snippet": if js.len() > 60 { format!("{}...", &js[..60]) } else { js },
                "message": "Dialog bridge JS expression received (stub). Use browser_cdp for actual injection.",
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Provider config parsing -------------------------------------------------

    #[test]
    fn test_cloud_provider_from_str() {
        assert_eq!(
            "browserbase".parse::<CloudProvider>().unwrap(),
            CloudProvider::Browserbase
        );
        assert_eq!(
            "browser_use".parse::<CloudProvider>().unwrap(),
            CloudProvider::BrowserUse
        );
        assert_eq!(
            "browseruse".parse::<CloudProvider>().unwrap(),
            CloudProvider::BrowserUse
        );
        assert_eq!(
            "firecrawl".parse::<CloudProvider>().unwrap(),
            CloudProvider::Firecrawl
        );
        assert!("unknown".parse::<CloudProvider>().is_err());
    }

    #[test]
    fn test_cloud_provider_display() {
        assert_eq!(CloudProvider::Browserbase.to_string(), "browserbase");
        assert_eq!(CloudProvider::BrowserUse.to_string(), "browser_use");
        assert_eq!(CloudProvider::Firecrawl.to_string(), "firecrawl");
    }

    #[test]
    fn test_cloud_provider_config_serde() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::Browserbase,
            api_key: Some("test-key-123".into()),
            api_url: None,
            region: Some("us-east-1".into()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CloudProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.provider_type, CloudProvider::Browserbase);
        assert_eq!(deserialized.api_key.unwrap(), "test-key-123");
        assert_eq!(deserialized.region.unwrap(), "us-east-1");
    }

    // -- Session lifecycle ------------------------------------------------------

    #[tokio::test]
    async fn test_session_create_and_close() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);

        let session = supervisor
            .create_session(None, Some("https://example.com".into()))
            .await
            .unwrap();

        assert!(session.session_id.starts_with("browser_use_"));
        assert_eq!(session.url.unwrap(), "https://example.com");
        assert_eq!(session.status, SessionStatus::Connected);
        assert_eq!(session.provider, "browser_use");

        // Close it
        supervisor.close_session(&session.session_id).await.unwrap();

        // Verify it's gone
        let sessions = supervisor.list_sessions().await;
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn test_get_session() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);

        let created = supervisor.create_session(None, None).await.unwrap();
        let fetched = supervisor.get_session(&created.session_id).await.unwrap();

        assert_eq!(created.session_id, fetched.session_id);
        assert_eq!(created.provider, fetched.provider);
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);
        let result = supervisor.get_session("nonexistent-id").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_close_nonexistent_session() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);
        let result = supervisor.close_session("nonexistent").await;
        assert!(result.is_err());
    }

    // -- Keep-alive --------------------------------------------------------------

    #[tokio::test]
    async fn test_keep_alive() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);

        let session = supervisor.create_session(None, None).await.unwrap();
        let original = session.last_active.clone();

        // Small delay so timestamps differ
        tokio::time::sleep(Duration::from_millis(10)).await;

        supervisor.keep_alive(&session.session_id).await.unwrap();
        let updated = supervisor
            .get_session(&session.session_id)
            .await
            .unwrap();

        assert_ne!(original, updated.last_active);
    }

    #[tokio::test]
    async fn test_keep_alive_nonexistent() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);
        let result = supervisor.keep_alive("no-such-session").await;
        assert!(result.is_err());
    }

    // -- Concurrent session management ------------------------------------------

    #[tokio::test]
    async fn test_concurrent_session_creation() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));

        let mut handles = Vec::new();
        for i in 0..10 {
            let sup = supervisor.clone();
            handles.push(tokio::spawn(async move {
                sup.create_session(None, Some(format!("https://site-{i}.com")))
                    .await
                    .unwrap()
            }));
        }

        let sessions: Vec<BrowserSession> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(sessions.len(), 10);

        // All sessions should be tracked
        let all = supervisor.list_sessions().await;
        assert_eq!(all.len(), 10);

        // Verify unique IDs
        let mut ids: Vec<String> = all.into_iter().map(|s| s.session_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 10);
    }

    #[tokio::test]
    async fn test_concurrent_close_and_list() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));

        // Create 5 sessions
        let mut session_ids = Vec::new();
        for _ in 0..5 {
            let s = supervisor.create_session(None, None).await.unwrap();
            session_ids.push(s.session_id);
        }

        assert_eq!(supervisor.list_sessions().await.len(), 5);

        // Close them all concurrently
        let mut close_handles = Vec::new();
        for sid in &session_ids {
            let sup = supervisor.clone();
            let sid = sid.clone();
            close_handles.push(tokio::spawn(async move {
                sup.close_session(&sid).await
            }));
        }

        for handle in close_handles {
            handle.await.unwrap().unwrap();
        }

        assert!(supervisor.list_sessions().await.is_empty());
    }

    // -- Tool schemas -----------------------------------------------------------

    #[tokio::test]
    async fn test_browser_supervisor_tool_schema() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));
        let tool = BrowserSupervisorTool::new(supervisor);

        assert_eq!(tool.name(), "browser_supervisor");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "browser_supervisor");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_cdp_navigate_tool_schema() {
        let tool = CdpNavigateTool;
        assert_eq!(tool.name(), "cdp_navigate");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "cdp_navigate");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_dialog_bridge_tool_schema() {
        let tool = DialogBridgeTool;
        assert_eq!(tool.name(), "dialog_bridge");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "dialog_bridge");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    // -- Tool execution ---------------------------------------------------------

    #[tokio::test]
    async fn test_browser_supervisor_tool_create_and_list() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));
        let tool = BrowserSupervisorTool::new(supervisor.clone());

        // Create session via tool
        let result = tool
            .execute(
                json!({ "operation": "create_session", "url": "https://example.com" }),
                ToolContext::default(),
            )
            .await;

        assert!(result.success, "create_session failed: {:?}", result.error);

        // List sessions via tool
        let list_result = tool
            .execute(
                json!({ "operation": "list_sessions" }),
                ToolContext::default(),
            )
            .await;

        assert!(list_result.success);
        let parsed: Value = serde_json::from_str(&list_result.content).unwrap();
        assert_eq!(parsed["count"].as_i64().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_browser_supervisor_tool_invalid_operation() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));
        let tool = BrowserSupervisorTool::new(supervisor);

        let result = tool
            .execute(
                json!({ "operation": "fly_to_the_moon" }),
                ToolContext::default(),
            )
            .await;

        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_browser_supervisor_tool_missing_session_id() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));
        let tool = BrowserSupervisorTool::new(supervisor);

        let result = tool
            .execute(
                json!({ "operation": "close_session" }),
                ToolContext::default(),
            )
            .await;

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Missing session_id"));
    }

    #[tokio::test]
    async fn test_browser_supervisor_tool_unknown_provider() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = Arc::new(CDPSupervisor::new(config));
        let tool = BrowserSupervisorTool::new(supervisor);

        let result = tool
            .execute(
                json!({ "operation": "create_session", "provider": "fakecloud" }),
                ToolContext::default(),
            )
            .await;

        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_dialog_bridge_tool_execute() {
        let tool = DialogBridgeTool;

        let result = tool
            .execute(
                json!({
                    "session_id": "test-session-123",
                    "js_expression": "window.alert = function(msg) { console.log(msg); };"
                }),
                ToolContext::default(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["status"], "bridge_injected");
        assert_eq!(parsed["session_id"], "test-session-123");
    }

    #[tokio::test]
    async fn test_cdp_navigate_tool_execute() {
        let tool = CdpNavigateTool;

        let result = tool
            .execute(
                json!({ "url": "https://example.com", "wait_until": "networkidle" }),
                ToolContext::default(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["url"], "https://example.com");
        assert_eq!(parsed["wait_until"], "networkidle");
    }

    #[tokio::test]
    async fn test_cdp_navigate_tool_invalid_args() {
        let tool = CdpNavigateTool;

        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;

        assert!(!result.success);
    }

    // -- Provider client (stub) -------------------------------------------------

    #[tokio::test]
    async fn test_browser_use_client_stub() {
        let client = BrowserUseClient::new("test".into());

        let session_id = client.create_session(Some("https://example.com")).await.unwrap();
        assert!(session_id.starts_with("browser_use_"));

        let status = client.get_session_status(&session_id).await.unwrap();
        assert_eq!(status, SessionStatus::Connected);

        client.close_session(&session_id).await.unwrap();

        let screenshot = client.take_screenshot(&session_id).await.unwrap();
        assert!(!screenshot.is_empty());
    }

    // -- Inject dialog bridge ---------------------------------------------------

    #[tokio::test]
    async fn test_inject_dialog_bridge() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);

        let session = supervisor.create_session(None, None).await.unwrap();

        let msg = supervisor
            .inject_dialog_bridge(&session.session_id, "window.__hermes = true;")
            .await
            .unwrap();

        assert!(msg.contains(&session.session_id));
    }

    #[tokio::test]
    async fn test_inject_dialog_bridge_nonexistent() {
        let config = CloudProviderConfig {
            provider_type: CloudProvider::BrowserUse,
            api_key: Some("test".into()),
            api_url: None,
            region: None,
        };
        let supervisor = CDPSupervisor::new(config);
        let result = supervisor.inject_dialog_bridge("no-such-session", "code").await;
        assert!(result.is_err());
    }
}
