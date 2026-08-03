#![allow(dead_code)] // Gateway module: webhook handlers, admin functions, and structs prepared for future use

//! Axum-based HTTP gateway with proper HTTP/1.1 compliance, body limits, and timeouts.
//!
//! This module replaces the raw TCP implementation with axum for:
//! - Proper HTTP/1.1 parsing and compliance
//! - Content-Length validation (handled by hyper)
//! - Request body size limits (64KB max)
//! - Request timeouts (30s) to prevent slow-loris attacks
//! - Header sanitization (handled by axum/hyper)

pub mod acp;
pub mod api;
pub mod error;
pub mod api_config;
pub mod api_onboard;
pub mod api_pairing;
pub mod api_personality;
#[cfg(feature = "plugins-wasm")]
pub mod api_plugins;
#[cfg(feature = "webauthn")]
pub mod api_webauthn;
pub mod auth_rate_limit;
pub mod canvas;
pub mod hardware_context;
pub mod node_tool;
pub mod nodes;
pub mod openapi;
pub mod session_queue;
pub mod sse;
pub mod static_files;
pub mod tls;
#[cfg(feature = "gateway-voice-duplex")]
pub mod voice_duplex;
pub mod ws;
pub mod ws_approval;

use crate::error::{Error, GatewayContextExt as _, Result};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json},
    routing::get,
};
use axum_server::tls_rustls::RustlsConfig;
use operant_api::channel::{Channel, SendMessage};
use operant_api::tool::ToolSpec;
use operant_channels::gmail_push::GmailPushChannel;
use operant_channels::linq::LinqChannel;
use operant_channels::nextcloud_talk::NextcloudTalkChannel;
use operant_channels::wati::WatiChannel;
use operant_channels::whatsapp::WhatsAppChannel;
use operant_config::pairing::PairingGuard;
use operant_config::schema::Config;
use operant_infra::session_backend::SessionBackend;
use operant_memory::{self, Memory, MemoryCategory, MemoryResult};
use operant_providers::{self, Provider};
use operant_runtime::cost::CostTracker;
use operant_runtime::observability::{Observer, set_scoped_broadcast_hook};
use operant_runtime::security::pairing::constant_time_eq;
use operant_runtime::tools;
use operant_runtime::tools::CanvasStore;
use operant_runtime::util::truncate_with_ellipsis;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

/// Maximum request body size (64KB) — prevents memory exhaustion
pub const MAX_BODY_SIZE: usize = 65_536;
/// Default request timeout (30s) — prevents slow-loris attacks.
pub const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Default request timeout for `POST /api/cron/{id}/run` (10 minutes).
///
/// Manually-triggered cron jobs run synchronously inside the request handler
/// and frequently exceed the 30s gateway-wide default — agent jobs in
/// particular can take minutes to complete a full reasoning loop. Capping at
/// 10 minutes keeps the route from hanging indefinitely while still allowing
/// realistic workloads to finish.
pub const LONG_RUNNING_REQUEST_TIMEOUT_SECS: u64 = 600;

/// Read gateway request timeout from `OPERANT_GATEWAY_TIMEOUT_SECS` env var
/// at runtime, falling back to [`REQUEST_TIMEOUT_SECS`].
///
/// Agentic workloads with tool use (web search, MCP tools, sub-agent
/// delegation) regularly exceed 30 seconds. This allows operators to
/// increase the timeout without recompiling.
pub fn gateway_request_timeout_secs() -> u64 {
    std::env::var("OPERANT_GATEWAY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(REQUEST_TIMEOUT_SECS)
}

/// Read manual cron-run request timeout from
/// `OPERANT_GATEWAY_LONG_RUNNING_REQUEST_TIMEOUT_SECS` at runtime, falling back to
/// [`LONG_RUNNING_REQUEST_TIMEOUT_SECS`]. Long-running jobs (e.g. agent prompts that
/// invoke tools) can comfortably exceed the 30s gateway-wide default, so the
/// `/api/cron/{id}/run` route gets its own timeout layer.
pub fn gateway_long_running_request_timeout_secs() -> u64 {
    std::env::var("OPERANT_GATEWAY_LONG_RUNNING_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LONG_RUNNING_REQUEST_TIMEOUT_SECS)
}
/// Sliding window used by gateway rate limiting.
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;
/// Fallback max distinct client keys tracked in gateway rate limiter.
pub const RATE_LIMIT_MAX_KEYS_DEFAULT: usize = 10_000;
/// Fallback max distinct idempotency keys retained in gateway memory.
pub const IDEMPOTENCY_MAX_KEYS_DEFAULT: usize = 10_000;

fn webhook_memory_key() -> String {
    format!("webhook_msg_{}", Uuid::new_v4())
}

fn whatsapp_memory_key(msg: &operant_api::channel::ChannelMessage) -> String {
    format!("whatsapp_{}_{}", msg.sender, msg.id)
}

fn linq_memory_key(msg: &operant_api::channel::ChannelMessage) -> String {
    format!("linq_{}_{}", msg.sender, msg.id)
}

fn wati_memory_key(msg: &operant_api::channel::ChannelMessage) -> String {
    format!("wati_{}_{}", msg.sender, msg.id)
}

fn nextcloud_talk_memory_key(msg: &operant_api::channel::ChannelMessage) -> String {
    format!("nextcloud_talk_{}_{}", msg.sender, msg.id)
}

fn sender_session_id(channel: &str, msg: &operant_api::channel::ChannelMessage) -> String {
    #[allow(dead_code)]
    match &msg.thread_ts {
        Some(thread_id) => format!("{channel}_{thread_id}_{}", msg.sender),
        None => format!("{channel}_{}", msg.sender),
    }
}

fn webhook_session_id(headers: &HeaderMap) -> Option<String> {
    #[allow(dead_code)]
    const MAX_SESSION_ID_LEN: usize = 128;
    headers
        .get("X-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| value.len() <= MAX_SESSION_ID_LEN)
        .filter(|value| {
            value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        })
        .map(str::to_owned)
}

fn hash_webhook_secret(value: &str) -> String {
    #[allow(dead_code)]
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)
}

/// How often the rate limiter sweeps stale IP entries from its map.
const RATE_LIMITER_SWEEP_INTERVAL_SECS: u64 = 300; // 5 minutes

#[derive(Debug)]
struct SlidingWindowRateLimiter {
    limit_per_window: u32,
    window: Duration,
    max_keys: usize,
    requests: Mutex<(HashMap<String, Vec<Instant>>, Instant)>,
}

impl SlidingWindowRateLimiter {
    fn new(limit_per_window: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            limit_per_window,
            window,
            max_keys: max_keys.max(1),
            requests: Mutex::new((HashMap::new(), Instant::now())),
        }
    }

    fn prune_stale(requests: &mut HashMap<String, Vec<Instant>>, cutoff: Instant) {
        requests.retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
    }

    fn allow(&self, key: &str) -> bool {
        if self.limit_per_window == 0 {
            return true;
        }

        let now = Instant::now();
        let cutoff = now.checked_sub(self.window).unwrap_or_else(Instant::now);

        let mut guard = self.requests.lock();
        let (requests, last_sweep) = &mut *guard;

        // Periodic sweep: remove keys with no recent requests
        if last_sweep.elapsed() >= Duration::from_secs(RATE_LIMITER_SWEEP_INTERVAL_SECS) {
            Self::prune_stale(requests, cutoff);
            *last_sweep = now;
        }

        if !requests.contains_key(key) && requests.len() >= self.max_keys {
            // Opportunistic stale cleanup before eviction under cardinality pressure.
            Self::prune_stale(requests, cutoff);
            *last_sweep = now;

            if requests.len() >= self.max_keys {
                let evict_key = requests
                    .iter()
                    .min_by_key(|(_, timestamps)| timestamps.last().copied().unwrap_or(cutoff))
                    .map(|(k, _)| k.clone());
                if let Some(evict_key) = evict_key {
                    requests.remove(&evict_key);
                }
            }
        }

        let entry = requests.entry(key.to_owned()).or_default();
        entry.retain(|instant| *instant > cutoff);

        if entry.len() >= self.limit_per_window as usize {
            return false;
        }

        entry.push(now);
        true
    }
}

#[derive(Debug)]
pub struct GatewayRateLimiter {
    pair: SlidingWindowRateLimiter,
    webhook: SlidingWindowRateLimiter,
}

impl GatewayRateLimiter {
    pub fn new(pair_per_minute: u32, webhook_per_minute: u32, max_keys: usize) -> Self {
        let window = Duration::from_secs(RATE_LIMIT_WINDOW_SECS);
        Self {
            pair: SlidingWindowRateLimiter::new(pair_per_minute, window, max_keys),
            webhook: SlidingWindowRateLimiter::new(webhook_per_minute, window, max_keys),
        }
    }

    fn allow_pair(&self, key: &str) -> bool {
        self.pair.allow(key)
    }

    fn allow_webhook(&self, key: &str) -> bool {
        self.webhook.allow(key)
    }
}

#[derive(Debug)]
pub struct IdempotencyStore {
    ttl: Duration,
    max_keys: usize,
    keys: Mutex<HashMap<String, Instant>>,
}

impl IdempotencyStore {
    pub fn new(ttl: Duration, max_keys: usize) -> Self {
        Self {
            ttl,
            max_keys: max_keys.max(1),
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if this key is new and is now recorded.
    fn record_if_new(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut keys = self.keys.lock();

        keys.retain(|_, seen_at| now.duration_since(*seen_at) < self.ttl);

        if keys.contains_key(key) {
            return false;
        }

        if keys.len() >= self.max_keys {
            let evict_key = keys
                .iter()
                .min_by_key(|(_, seen_at)| *seen_at)
                .map(|(k, _)| k.clone());
            if let Some(evict_key) = evict_key {
                keys.remove(&evict_key);
            }
        }

        keys.insert(key.to_owned(), now);
        true
    }
}

fn parse_client_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }

    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some(addr.ip());
    }

    let value = value.trim_matches(['[', ']']);
    value.parse::<IpAddr>().ok()
}

fn dirs_data_local() -> Option<std::path::PathBuf> {
    #[allow(dead_code)]
    directories::BaseDirs::new().map(|d| d.data_local_dir().to_path_buf())
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for candidate in xff.split(',') {
            if let Some(ip) = parse_client_ip(candidate) {
                return Some(ip);
            }
        }
    }

    headers
        .get("X-Real-IP")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_client_ip)
}

fn client_key_from_request(
    peer_addr: Option<SocketAddr>,
    headers: &HeaderMap,
    trust_forwarded_headers: bool,
) -> String {
    if trust_forwarded_headers && let Some(ip) = forwarded_client_ip(headers) {
        return ip.to_string();
    }

    peer_addr
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn normalize_max_keys(configured: usize, fallback: usize) -> usize {
    #[allow(dead_code)]
    if configured == 0 {
        fallback.max(1)
    } else {
        configured
    }
}

/// Shared state for all axum handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub temperature: f64,
    pub mem: Arc<dyn Memory>,
    pub auto_save: bool,
    /// SHA-256 hash of `X-Webhook-Secret` (hex-encoded), never plaintext.
    pub webhook_secret_hash: Option<Arc<str>>,
    pub pairing: Arc<PairingGuard>,
    pub trust_forwarded_headers: bool,
    pub rate_limiter: Arc<GatewayRateLimiter>,
    pub auth_limiter: Arc<auth_rate_limit::AuthRateLimiter>,
    pub idempotency_store: Arc<IdempotencyStore>,
    pub whatsapp: Option<Arc<WhatsAppChannel>>,
    /// `WhatsApp` app secret for webhook signature verification (`X-Hub-Signature-256`)
    pub whatsapp_app_secret: Option<Arc<str>>,
    pub linq: Option<Arc<LinqChannel>>,
    /// Linq webhook signing secret for signature verification
    pub linq_signing_secret: Option<Arc<str>>,
    pub nextcloud_talk: Option<Arc<NextcloudTalkChannel>>,
    /// Nextcloud Talk webhook secret for signature verification
    pub nextcloud_talk_webhook_secret: Option<Arc<str>>,
    pub wati: Option<Arc<WatiChannel>>,
    /// Gmail Pub/Sub push notification channel
    pub gmail_push: Option<Arc<GmailPushChannel>>,
    /// Observability backend for metrics scraping
    pub observer: Arc<dyn operant_runtime::observability::Observer>,
    /// Registered tool specs (for web dashboard tools page)
    pub tools_registry: Arc<Vec<ToolSpec>>,
    /// Cost tracker (optional, for web dashboard cost page)
    pub cost_tracker: Option<Arc<CostTracker>>,
    /// SSE broadcast channel for real-time events
    pub event_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// Ring buffer of recent events for history replay
    pub event_buffer: Arc<sse::EventBuffer>,
    /// Shutdown signal sender for graceful shutdown
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Reload signal sender owned by the daemon. /admin/reload writes `true`
    /// here; the daemon's wait loop reacts and re-instantiates every
    /// subsystem in place. `None` when running standalone (`operant gateway start`)
    /// — reload then degrades to a 503 with a clear message.
    pub reload_tx: Option<tokio::sync::watch::Sender<bool>>,
    /// Registry of dynamically connected nodes
    pub node_registry: Arc<nodes::NodeRegistry>,
    /// Path prefix for reverse-proxy deployments (empty string = no prefix)
    pub path_prefix: String,
    /// Filesystem path to `web/dist/` for serving the dashboard (None = API-only)
    pub web_dist_dir: Option<std::path::PathBuf>,
    /// Session backend for persisting gateway WS chat sessions
    pub session_backend: Option<Arc<dyn SessionBackend>>,
    /// Per-session actor queue for serializing concurrent turns
    pub session_queue: Arc<session_queue::SessionActorQueue>,
    /// Device registry for paired device management
    pub device_registry: Option<Arc<api_pairing::DeviceRegistry>>,
    /// Pending pairing request store
    pub pending_pairings: Option<Arc<api_pairing::PairingStore>>,
    /// Shared canvas store for Live Canvas (A2UI) system
    pub canvas_store: CanvasStore,
    /// WebAuthn state for hardware key authentication (optional, requires `webauthn` feature)
    #[cfg(feature = "webauthn")]
    pub webauthn: Option<Arc<api_webauthn::WebAuthnState>>,
    /// Per-session cancellation tokens for aborting in-flight agent responses.
    /// Key is session_key (e.g. `gw_<session_id>`), value is the token for the
    /// current turn. Entries are inserted before each turn and removed after
    /// completion (normal or cancelled).
    pub cancel_tokens: Arc<
        std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>,
    >,
}

impl AppState {
    /// Create a new AppState with all required fields initialized.
    pub fn new(
        config: Config,
        pairing: Arc<PairingGuard>,
        _shutdown_rx: tokio::sync::watch::Receiver<bool>,
        _event_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
        _external_event_tx: Option<tokio::sync::broadcast::Sender<serde_json::Value>>,
        _reload_tx: Option<tokio::sync::watch::Sender<bool>>,
        _canvas_store: Option<CanvasStore>,
    ) -> Self {
        let config = Arc::new(Mutex::new(config));

        // Mock provider and memory for gateway - use test implementations
        struct MockProvider;
        #[async_trait::async_trait]
        impl operant_providers::Provider for MockProvider {
            async fn chat_with_system(
                &self,
                _system_prompt: Option<&str>,
                _message: &str,
                _model: &str,
                _temperature: Option<f64>,
            ) -> anyhow::Result<String> {
                Ok("ok".to_string())
            }
        }

        #[derive(Debug, Default, Clone, Copy)]
        struct MockMemory;
        #[async_trait::async_trait]
        impl operant_memory::Memory for MockMemory {
            fn name(&self) -> &str {
                "mock"
            }
            async fn store(
                &self,
                _key: &str,
                _content: &str,
                _category: operant_memory::MemoryCategory,
                _session_id: Option<&str>,
            ) -> MemoryResult<()> {
                Ok(())
            }
            async fn recall(
                &self,
                _query: &str,
                _limit: usize,
                _session_id: Option<&str>,
                _since: Option<&str>,
                _until: Option<&str>,
            ) -> MemoryResult<Vec<operant_memory::MemoryEntry>> {
                Ok(Vec::new())
            }
            async fn get(&self, _key: &str) -> MemoryResult<Option<operant_memory::MemoryEntry>> {
                Ok(None)
            }
            async fn list(
                &self,
                _category: Option<&operant_memory::MemoryCategory>,
                _session_id: Option<&str>,
            ) -> MemoryResult<Vec<operant_memory::MemoryEntry>> {
                Ok(Vec::new())
            }
            async fn forget(&self, _key: &str) -> MemoryResult<bool> {
                Ok(false)
            }
            async fn count(&self) -> MemoryResult<usize> {
                Ok(0)
            }
            async fn health_check(&self) -> bool {
                true
            }
        }

        Self {
            config,
            provider: Arc::new(MockProvider),
            model: String::new(),
            temperature: 0.0,
            mem: Arc::new(MockMemory),
            auto_save: false,
            webhook_secret_hash: None,
            pairing: pairing.clone(),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(crate::auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(crate::sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(crate::nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: Arc::new(crate::session_queue::SessionActorQueue::new(8, 30, 600)),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        }
    }
}

/// Create a test AppState for unit tests.
pub fn test_state(config: operant_config::schema::Config) -> AppState {
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel(16);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let pairing = Arc::new(operant_config::pairing::PairingGuard::new(false, &[]));

    AppState::new(config, pairing, shutdown_rx, event_tx, None, None, None)
}

/// Run the HTTP gateway using axum with proper HTTP/1.1 compliance.
#[allow(clippy::too_many_lines)]
pub async fn run_gateway(
    host: &str,
    port: u16,
    config: Config,
    external_event_tx: Option<tokio::sync::broadcast::Sender<serde_json::Value>>,
    // Reload sender owned by the daemon. /admin/reload writes `true` here;
    // the daemon's wait loop reacts via `subscribe()` and tears down to
    // re-init. Cross-platform replacement for the SIGUSR1 hack.
    reload_tx: Option<tokio::sync::watch::Sender<bool>>,
    canvas_store: Option<CanvasStore>,
) -> Result<()> {
    // ── Security: paircode verification ────────────────────────────
    let paired_tokens = config.gateway.paired_tokens.clone();
    let require_pairing = config.gateway.require_pairing;
    let pairing = Arc::new(PairingGuard::new(require_pairing, &paired_tokens));

    if require_pairing && !pairing.is_paired() {
        // Generate and display paircode if not paired
        let paircode = pairing
            .generate_new_pairing_code()
            .ok_or_else(|| Error::message("Failed to generate pairing code"))?;
        tracing::info!("🔐 Gateway requires pairing. Paircode: {}", paircode);
    }

    // ── TLS / mTLS setup ───────────────────────────────────────────
    let tls_config = match &config.gateway.tls {
        Some(tls_cfg) if tls_cfg.enabled => {
            let has_mtls = tls_cfg.client_auth.as_ref().is_some_and(|ca| ca.enabled);
            if has_mtls {
                tracing::info!("TLS enabled with mutual TLS (mTLS) client verification");
            } else {
                tracing::info!("TLS enabled (no client certificate requirement)");
            }
            Some(tls::build_rustls_config(tls_cfg)?)
        }
        _ => None,
    };

    // ── Build application router ──────────────────────────────────
    let (_shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel(128);

    // Create a broadcast observer that forwards events to the SSE channel
    struct BroadcastObserver {
        tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    }
    impl operant_runtime::observability::Observer for BroadcastObserver {
        fn record_event(&self, event: &operant_runtime::observability::ObserverEvent) {
            let _ = self.tx.send(serde_json::json!({
                "event": format!("{:?}", event)
            }));
        }
        fn record_metric(&self, _metric: &operant_runtime::observability::traits::ObserverMetric) {}
        fn name(&self) -> &str {
            "gateway-broadcast"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    let broadcast_observer = Arc::new(BroadcastObserver {
        tx: event_tx.clone(),
    });
    let broadcast_hook_guard = set_scoped_broadcast_hook(broadcast_observer);

    // Build the main router with all routes
    let inner = crate::api::router(
        config.clone(),
        pairing.clone(),
        shutdown_rx.clone(),
        event_tx.clone(),
        external_event_tx.clone(),
        reload_tx.clone(),
        canvas_store.clone(),
    );

    // Nest under path prefix when configured (axum strips prefix before routing).
    // nest() at "/prefix" handles both "/prefix" and "/prefix/*" but not "/prefix/"
    // with a trailing slash, so we add a fallback redirect for that case.
    let path_prefix = config
        .gateway
        .path_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let app = if let Some(prefix) = path_prefix {
        let redirect_target = prefix.to_string();
        Router::new().nest(prefix, inner).route(
            &format!("{prefix}/"),
            get(|| async move { axum::response::Redirect::permanent(&redirect_target) }),
        )
    } else {
        inner
    };

    // Add middleware layers
    let app = app
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(Duration::from_secs(
            gateway_long_running_request_timeout_secs(),
        )));

    // Add the state to create Router<AppState>
    let app = app.with_state(AppState::new(
        config.clone(),
        pairing.clone(),
        shutdown_rx.clone(),
        event_tx.clone(),
        external_event_tx,
        reload_tx,
        canvas_store,
    ));

    // ── Start server ──────────────────────────────────────────────
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}"))
        .await
        .context("failed to bind TCP listener")?;

    if let Some(tls_config) = tls_config {
        // TLS path — use axum_server which supports stateful routers directly
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();

        tokio::spawn(async move {
            let _ = shutdown_rx.changed().await;
            tracing::info!("🦀 Operant Gateway shutting down...");
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });

        let rustls_config = RustlsConfig::from_config(Arc::new(tls_config));

        axum_server::bind_rustls(format!("{host}:{port}").parse()?, rustls_config)
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        // Plain TCP — use axum's built-in serve (properly handles stateful routers in axum 0.8+)
        // Convert Router<AppState> to Router<()> by calling with_state(())
        let mut shutdown_rx = shutdown_rx.clone();
        let make_service = app
            .with_state(())
            .into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, make_service)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
                tracing::info!("🦀 Operant Gateway shutting down...");
            })
            .await?;
    }

    drop(broadcast_hook_guard);

    Ok(())
}

fn format_paircode_recovery_command(host: &str, port: u16) -> String {
    #[allow(dead_code)]
    let mut cmd = format!("operant gateway get-paircode --new --port {port}");
    if let Some(host_arg) = paircode_recovery_host_arg(host) {
        cmd.push_str(" --host ");
        cmd.push_str(host_arg);
    }
    cmd
}

fn paircode_recovery_host_arg(host: &str) -> Option<&str> {
    #[allow(dead_code)]
    match host {
        "127.0.0.1" | "localhost" | "::1" | "0.0.0.0" | "::" => None,
        _ => Some(host),
    }
}

fn format_paircode_recovery_curl(host: &str, port: u16, path_prefix: &str) -> String {
    format!("curl -s -X POST http://{host}:{port}{path_prefix}/admin/paircode/new")
}

// ══════════════════════════════════════════════════════════════════════════════
// AXUM HANDLERS
// ══════════════════════════════════════════════════════════════════════════════

/// GET /health — always public (no secrets leaked)
async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    #[allow(dead_code)]
    let body = serde_json::json!({
        "status": "ok",
        "paired": state.pairing.is_paired(),
        "require_pairing": state.pairing.require_pairing(),
        "runtime": operant_runtime::health::snapshot_json(),
    });
    Json(body)
}

/// Prometheus content type for text exposition format.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
#[allow(dead_code)]
fn prometheus_disabled_hint() -> String {
    #[allow(dead_code)]
    String::from(
        "# Prometheus backend not enabled. Set [observability] backend = \"prometheus\" in config.\n",
    )
}

#[cfg(feature = "observability-prometheus")]
fn prometheus_observer_from_state(
    observer: &dyn operant_runtime::observability::Observer,
) -> Option<&operant_runtime::observability::PrometheusObserver> {
    // `TeeObserver::as_any` returns the primary observer, so a single direct
    // downcast finds the PrometheusObserver whether the state observer is the
    // raw backend or wrapped by the factory tee.
    observer
        .as_any()
        .downcast_ref::<operant_runtime::observability::PrometheusObserver>()
}

/// GET /metrics — Prometheus text exposition format
async fn handle_metrics(State(state): State<AppState>) -> impl IntoResponse {
    #[allow(dead_code)]
    let body = {
        #[cfg(feature = "observability-prometheus")]
        {
            if let Some(prom) = prometheus_observer_from_state(state.observer.as_ref()) {
                prom.encode()
            } else {
                prometheus_disabled_hint()
            }
        }
        #[cfg(not(feature = "observability-prometheus"))]
        {
            let _ = &state;
            prometheus_disabled_hint()
        }
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
}

/// POST /pair — exchange one-time code for bearer token
#[axum::debug_handler]
async fn handle_pair(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let rate_key =
        client_key_from_request(Some(peer_addr), &headers, state.trust_forwarded_headers);
    if !state.rate_limiter.allow_pair(&rate_key) {
        tracing::warn!("/pair rate limit exceeded");
        let err = serde_json::json!({
            "error": "Too many pairing requests. Please retry later.",
            "retry_after": RATE_LIMIT_WINDOW_SECS,
        });
        return (StatusCode::TOO_MANY_REQUESTS, Json(err));
    }

    // ── Auth rate limiting (brute-force protection) ──
    if let Err(e) = state.auth_limiter.check_rate_limit(&rate_key) {
        tracing::warn!("🔐 Pairing auth rate limit exceeded for {rate_key}");
        let err = serde_json::json!({
            "error": format!("Too many auth attempts. Try again in {}s.", e.retry_after_secs),
            "retry_after": e.retry_after_secs,
        });
        return (StatusCode::TOO_MANY_REQUESTS, Json(err));
    }

    let code = headers
        .get("X-Pairing-Code")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    match state.pairing.try_pair(code, &rate_key).await {
        Ok(Some(token)) => {
            tracing::info!("🔐 New client paired successfully");
            if let Err(err) =
                Box::pin(persist_pairing_tokens(state.config.clone(), &state.pairing)).await
            {
                tracing::error!("🔐 Pairing succeeded but token persistence failed: {err:#}");
                let body = serde_json::json!({
                    "paired": true,
                    "persisted": false,
                    "token": token,
                    "message": "Paired for this process, but failed to persist token to config.toml. Check config path and write permissions.",
                });
                return (StatusCode::OK, Json(body));
            }

            let body = serde_json::json!({
                "paired": true,
                "persisted": true,
                "token": token,
                "message": "Save this token — use it as Authorization: Bearer <token>"
            });
            (StatusCode::OK, Json(body))
        }
        Ok(None) => {
            state.auth_limiter.record_attempt(&rate_key);
            tracing::warn!("🔐 Pairing attempt with invalid code");
            let err = serde_json::json!({"error": "Invalid pairing code"});
            (StatusCode::FORBIDDEN, Json(err))
        }
        Err(lockout_secs) => {
            tracing::warn!(
                "🔐 Pairing locked out — too many failed attempts ({lockout_secs}s remaining)"
            );
            let err = serde_json::json!({
                "error": format!("Too many failed attempts. Try again in {lockout_secs}s."),
                "retry_after": lockout_secs
            });
            (StatusCode::TOO_MANY_REQUESTS, Json(err))
        }
    }
}

async fn persist_pairing_tokens(config: Arc<Mutex<Config>>, pairing: &PairingGuard) -> Result<()> {
    let paired_tokens = pairing.tokens();
    // This is needed because parking_lot's guard is not Send so we clone the inner
    // this should be removed once async mutexes are used everywhere
    let mut updated_cfg = { config.lock().clone() };
    updated_cfg.gateway.paired_tokens = paired_tokens;
    updated_cfg
        .save()
        .await
        // `Config::save` returns `anyhow::Result` (operant-config boundary),
        // which the generic GatewayContextExt blanket impl cannot cover —
        // `anyhow::Error` deliberately does not implement `std::error::Error`.
        // Convert explicitly, preserving the context on the Backend seam.
        .map_err(|e| Error::Backend(e.context("Failed to persist paired tokens to config.toml")))?;

    // Keep shared runtime config in sync with persisted tokens.
    *config.lock() = updated_cfg;
    Ok(())
}

/// Result of a gateway chat turn. Carries the response text plus per-turn
/// token / cost totals captured from the cost-tracking scope (when present)
/// so callers can populate observer-event annotations without racing
/// concurrent webhook traffic that shares the same `CostTracker`.
struct GatewayChatOutcome {
    #[allow(dead_code)]
    response: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

/// Returns a structured `needs_onboarding` error when `model` is empty
/// or whitespace-only, otherwise `None`. Empty model means the gateway
/// booted with nothing configured (fresh install). Callers refuse the
/// dispatch with this marker instead of calling the provider with an
/// empty model id. Mirrors `agent::Agent::from_config` (#6099) at
/// request-time so `/onboard` stays reachable.
fn needs_onboarding_for(model: &str) -> Option<Error> {
    if model.trim().is_empty() {
        Some(Error::NeedsOnboarding)
    } else {
        None
    }
}

/// True when `e` is the typed [`Error::NeedsOnboarding`] variant produced
/// by [`needs_onboarding_for`]. Used by chat-dispatch error paths to map
/// the marker to a 503 `needs_onboarding` HTTP response or a more accurate
/// channel-side reply, instead of the generic 500 / "sorry" catch-all.
///
/// Typed-variant matching (instead of a substring probe) means unrelated
/// errors can never be misclassified, and a wrapped onboarding marker is
/// caught by the compiler instead of silently degrading to a 500.
fn is_needs_onboarding_err(e: &Error) -> bool {
    matches!(e, Error::NeedsOnboarding)
}

/// Reply text sent over a channel SDK when chat dispatch refuses
/// because the gateway has no model configured.
fn needs_onboarding_channel_reply() -> String {
    let reply = operant_runtime::i18n::get_cli_string("channel-needs-onboarding-reply");
    match reply {
        Some(text) if !text.starts_with('{') => text,
        _ => {
            tracing::warn!(
                "Fluent i18n key 'channel-needs-onboarding-reply' not resolved; using hardcoded fallback. Ensure i18n::init() is called at startup."
            );
            "This agent isn't fully set up yet. The operator needs to complete onboarding before I can reply.".to_string()
        }
    }
}

/// Full-featured chat with tools for channel and webhook handlers.
async fn run_gateway_chat_with_tools(
    #[allow(dead_code)] state: &AppState,
    message: &str,
    session_id: Option<&str>,
) -> Result<GatewayChatOutcome> {
    if let Some(err) = needs_onboarding_for(&state.model) {
        return Err(err);
    }

    // Tests exercise webhook infrastructure (idempotency, auth, autosave)
    // through handle_webhook, so dispatch to the mock provider directly
    // instead of bootstrapping the full agent runtime. The mock path
    // doesn't go through the cost-tracking scope, so usage stays None.
    #[cfg(test)]
    {
        let _ = session_id;
        let response = state
            .provider
            .chat_with_system(None, message, &state.model, Some(state.temperature))
            .await?;
        Ok(GatewayChatOutcome {
            response,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
        })
    }

    #[cfg(not(test))]
    {
        let config = state.config.lock().clone();
        // Scope the cost tracking context so per-LLM-call usage flows into the
        // gateway's cost tracker and costs.jsonl. Without this scope, the
        // tracker exists on AppState but never receives any records from the
        // runtime tool loop. The context's per-scope `turn_usage` accumulator
        // also lets us read out this turn's tokens / cost after the scope
        // exits without racing concurrent webhook traffic that shares the
        // same tracker.
        let cost_tracking_context = state.cost_tracker.as_ref().map(|tracker| {
            operant_runtime::agent::loop_::ToolLoopCostTrackingContext::new(
                tracker.clone(),
                std::sync::Arc::new(state.config.lock().cost.prices.clone()),
            )
        });
        let captured_usage = cost_tracking_context
            .as_ref()
            .map(|ctx| ctx.turn_usage.clone());
        let response = Box::pin(
            operant_runtime::agent::loop_::TOOL_LOOP_COST_TRACKING_CONTEXT.scope(
                cost_tracking_context,
                operant_runtime::agent::process_message(
                    config,
                    message,
                    session_id,
                    Some(state.observer.clone()),
                ),
            ),
        )
        .await?;
        let usage = captured_usage
            .map(|cell| *cell.lock())
            .filter(|u| u.input_tokens > 0 || u.output_tokens > 0);
        let (input_tokens, output_tokens, cost_usd) = match usage {
            Some(u) => (
                Some(u.input_tokens),
                Some(u.output_tokens),
                Some(u.cost_usd),
            ),
            None => (None, None, None),
        };
        Ok(GatewayChatOutcome {
            response,
            input_tokens,
            output_tokens,
            cost_usd,
        })
    }
}

/// Webhook request body
#[derive(serde::Deserialize)]
pub struct WebhookBody {
    pub message: String,
}

/// POST /webhook — main webhook endpoint
async fn handle_webhook(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<WebhookBody>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let rate_key =
        client_key_from_request(Some(peer_addr), &headers, state.trust_forwarded_headers);
    if !state.rate_limiter.allow_webhook(&rate_key) {
        tracing::warn!("/webhook rate limit exceeded");
        let err = serde_json::json!({
            "error": "Too many webhook requests. Please retry later.",
            "retry_after": RATE_LIMIT_WINDOW_SECS,
        });
        return (StatusCode::TOO_MANY_REQUESTS, Json(err));
    }

    // ── Bearer token auth (pairing) with auth rate limiting ──
    if state.pairing.require_pairing() {
        if let Err(e) = state.auth_limiter.check_rate_limit(&rate_key) {
            tracing::warn!("Webhook: auth rate limit exceeded for {rate_key}");
            let err = serde_json::json!({
                "error": format!("Too many auth attempts. Try again in {}s.", e.retry_after_secs),
                "retry_after": e.retry_after_secs,
            });
            return (StatusCode::TOO_MANY_REQUESTS, Json(err));
        }
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("");
        if !state.pairing.is_authenticated(token) {
            state.auth_limiter.record_attempt(&rate_key);
            tracing::warn!("Webhook: rejected — not paired / invalid bearer token");
            let err = serde_json::json!({
                "error": "Unauthorized — pair first via POST /pair, then send Authorization: Bearer <token>"
            });
            return (StatusCode::UNAUTHORIZED, Json(err));
        }
    }

    // ── Webhook secret auth (optional, additional layer) ──
    if let Some(ref secret_hash) = state.webhook_secret_hash {
        let header_hash = headers
            .get("X-Webhook-Secret")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(hash_webhook_secret);
        match header_hash {
            Some(val) if constant_time_eq(&val, secret_hash.as_ref()) => {}
            _ => {
                tracing::warn!("Webhook: rejected request — invalid or missing X-Webhook-Secret");
                let err = serde_json::json!({"error": "Unauthorized — invalid or missing X-Webhook-Secret header"});
                return (StatusCode::UNAUTHORIZED, Json(err));
            }
        }
    }

    // ── Parse body ──
    let Json(webhook_body) = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Webhook JSON parse error: {e}");
            let err = serde_json::json!({
                "error": "Invalid JSON body. Expected: {\"message\": \"...\"}"
            });
            return (StatusCode::BAD_REQUEST, Json(err));
        }
    };

    // ── Idempotency (optional) ──
    if let Some(idempotency_key) = headers
        .get("X-Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && !state.idempotency_store.record_if_new(idempotency_key)
    {
        tracing::info!("Webhook duplicate ignored (idempotency key: {idempotency_key})");
        let body = serde_json::json!({
            "status": "duplicate",
            "idempotent": true,
            "message": "Request already processed for this idempotency key"
        });
        return (StatusCode::OK, Json(body));
    }

    let message = &webhook_body.message;
    let session_id = webhook_session_id(&headers);

    if state.auto_save && !operant_memory::should_skip_autosave_content(message) {
        let key = webhook_memory_key();
        let _ = state
            .mem
            .store(
                &key,
                message,
                MemoryCategory::Conversation,
                session_id.as_deref(),
            )
            .await;
    }

    let provider_label = state
        .config
        .lock()
        .providers
        .fallback
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let model_label = state.model.clone();
    let started_at = Instant::now();

    state
        .observer
        .record_event(&operant_runtime::observability::ObserverEvent::AgentStart {
            provider: provider_label.clone(),
            model: model_label.clone(),
        });
    state
        .observer
        .record_event(&operant_runtime::observability::ObserverEvent::LlmRequest {
            provider: provider_label.clone(),
            model: model_label.clone(),
            messages_count: 1,
        });

    match run_gateway_chat_with_tools(&state, message, session_id.as_deref()).await {
        Ok(GatewayChatOutcome {
            response,
            input_tokens,
            output_tokens,
            cost_usd,
        }) => {
            let duration = started_at.elapsed();
            // Per-turn token / cost annotation captured from the cost-tracking
            // scope inside `run_gateway_chat_with_tools` (None outside of test
            // / when no LLM call recorded). Cost is also persisted to
            // /api/cost and costs.jsonl via the same scope.
            let tokens_used = input_tokens
                .zip(output_tokens)
                .map(|(i, o)| i + o)
                .or(input_tokens)
                .or(output_tokens);
            state.observer.record_event(
                &operant_runtime::observability::ObserverEvent::LlmResponse {
                    provider: provider_label.clone(),
                    model: model_label.clone(),
                    duration,
                    success: true,
                    error_message: None,
                    input_tokens: None,
                    output_tokens: None,
                },
            );
            state.observer.record_metric(
                &operant_runtime::observability::traits::ObserverMetric::RequestLatency(duration),
            );
            state
                .observer
                .record_event(&operant_runtime::observability::ObserverEvent::AgentEnd {
                    provider: provider_label,
                    model: model_label,
                    duration,
                    tokens_used,
                    cost_usd,
                });

            let body = serde_json::json!({"response": response, "model": state.model});
            (StatusCode::OK, Json(body))
        }
        Err(e) => {
            let duration = started_at.elapsed();
            let sanitized = operant_providers::sanitize_api_error(&e.to_string());

            state.observer.record_event(
                &operant_runtime::observability::ObserverEvent::LlmResponse {
                    provider: provider_label.clone(),
                    model: model_label.clone(),
                    duration,
                    success: false,
                    error_message: Some(sanitized.clone()),
                    input_tokens: None,
                    output_tokens: None,
                },
            );
            state.observer.record_metric(
                &operant_runtime::observability::traits::ObserverMetric::RequestLatency(duration),
            );
            state
                .observer
                .record_event(&operant_runtime::observability::ObserverEvent::Error {
                    component: "gateway".to_string(),
                    message: sanitized.clone(),
                });
            state
                .observer
                .record_event(&operant_runtime::observability::ObserverEvent::AgentEnd {
                    provider: provider_label,
                    model: model_label,
                    duration,
                    tokens_used: None,
                    cost_usd: None,
                });

            if is_needs_onboarding_err(&e) {
                tracing::warn!(
                    "Webhook chat refused: gateway has no model configured; \
                     visit /onboard"
                );
                let body = serde_json::json!({
                    "error": "needs_onboarding",
                    "url": "/onboard"
                });
                (StatusCode::SERVICE_UNAVAILABLE, Json(body))
            } else {
                tracing::error!("Webhook provider error: {}", sanitized);
                let err = serde_json::json!({"error": "LLM request failed"});
                (StatusCode::INTERNAL_SERVER_ERROR, Json(err))
            }
        }
    }
}

/// `WhatsApp` verification query params
#[derive(serde::Deserialize)]
pub struct WhatsAppVerifyQuery {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}

/// GET /whatsapp — Meta webhook verification
async fn handle_whatsapp_verify(
    #[allow(dead_code)] State(state): State<AppState>,
    Query(params): Query<WhatsAppVerifyQuery>,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp else {
        return (StatusCode::NOT_FOUND, "WhatsApp not configured".to_string());
    };

    // Verify the token matches (constant-time comparison to prevent timing attacks)
    let token_matches = params
        .verify_token
        .as_deref()
        .is_some_and(|t| constant_time_eq(t, wa.verify_token()));
    if params.mode.as_deref() == Some("subscribe") && token_matches {
        if let Some(ch) = params.challenge {
            tracing::info!("WhatsApp webhook verified successfully");
            return (StatusCode::OK, ch);
        }
        return (StatusCode::BAD_REQUEST, "Missing hub.challenge".to_string());
    }

    tracing::warn!("WhatsApp webhook verification failed — token mismatch");
    (StatusCode::FORBIDDEN, "Forbidden".to_string())
}

/// Verify `WhatsApp` webhook signature (`X-Hub-Signature-256`).
/// Returns true if the signature is valid, false otherwise.
/// See: <https://developers.facebook.com/docs/graph-api/webhooks/getting-started#verification-requests>
pub fn verify_whatsapp_signature(app_secret: &str, body: &[u8], signature_header: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // Signature format: "sha256=<hex_signature>"
    let Some(hex_sig) = signature_header.strip_prefix("sha256=") else {
        return false;
    };

    // Decode hex signature
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };

    // Compute HMAC-SHA256
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);

    // Constant-time comparison
    mac.verify_slice(&expected).is_ok()
}

/// POST /whatsapp — incoming message webhook
async fn handle_whatsapp_message(
    #[allow(dead_code)] State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "WhatsApp not configured"})),
        );
    };

    // ── Security: Verify X-Hub-Signature-256 if app_secret is configured ──
    if let Some(ref app_secret) = state.whatsapp_app_secret {
        let signature = headers
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !verify_whatsapp_signature(app_secret, &body, signature) {
            tracing::warn!(
                "WhatsApp webhook signature verification failed (signature: {})",
                if signature.is_empty() {
                    "missing"
                } else {
                    "invalid"
                }
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid signature"})),
            );
        }
    }

    // Parse JSON body
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid JSON payload"})),
        );
    };

    // Parse messages from the webhook payload
    let messages = wa.parse_webhook_payload(&payload);

    if messages.is_empty() {
        // Acknowledge the webhook even if no messages (could be status updates)
        return (StatusCode::OK, Json(serde_json::json!({"status": "ok"})));
    }

    // Process each message
    for msg in &messages {
        tracing::info!(
            "WhatsApp message from {}: {}",
            msg.sender,
            truncate_with_ellipsis(&msg.content, 50)
        );

        // Route approval replies to pending approval requests before dispatching to agent
        if let Some((token, response)) = operant_channels::util::parse_approval_reply(&msg.content)
        {
            let mut map = wa.pending_approvals().lock().await;
            if let Some(sender) = map.remove(&token) {
                let _ = sender.send(response);
                continue;
            }
        }

        let session_id = sender_session_id("whatsapp", msg);

        // Auto-save to memory
        if state.auto_save && !operant_memory::should_skip_autosave_content(&msg.content) {
            let key = whatsapp_memory_key(msg);
            let _ = state
                .mem
                .store(
                    &key,
                    &msg.content,
                    MemoryCategory::Conversation,
                    Some(&session_id),
                )
                .await;
        }

        match Box::pin(run_gateway_chat_with_tools(
            &state,
            &msg.content,
            Some(&session_id),
        ))
        .await
        {
            Ok(GatewayChatOutcome { response, .. }) => {
                // Send reply via WhatsApp
                if let Err(e) = wa
                    .send(&SendMessage::new(response, &msg.reply_target))
                    .await
                {
                    tracing::error!("Failed to send WhatsApp reply: {e}");
                }
            }
            Err(e) => {
                let reply = if is_needs_onboarding_err(&e) {
                    tracing::warn!(
                        "WhatsApp chat refused: gateway has no model configured; \
                         visit /onboard"
                    );
                    needs_onboarding_channel_reply()
                } else {
                    tracing::error!("LLM error for WhatsApp message: {e:#}");
                    "Sorry, I couldn't process your message right now.".to_string()
                };
                let _ = wa.send(&SendMessage::new(reply, &msg.reply_target)).await;
            }
        }
    }

    // Acknowledge the webhook
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// POST /linq — incoming message webhook (iMessage/RCS/SMS via Linq)
async fn handle_linq_webhook(
    #[allow(dead_code)] State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref linq) = state.linq else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Linq not configured"})),
        );
    };

    let body_str = String::from_utf8_lossy(&body);

    // ── Security: Verify X-Webhook-Signature if signing_secret is configured ──
    if let Some(ref signing_secret) = state.linq_signing_secret {
        let timestamp = headers
            .get("X-Webhook-Timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let signature = headers
            .get("X-Webhook-Signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !operant_channels::linq::verify_linq_signature(
            signing_secret,
            &body_str,
            timestamp,
            signature,
        ) {
            tracing::warn!(
                "Linq webhook signature verification failed (signature: {})",
                if signature.is_empty() {
                    "missing"
                } else {
                    "invalid"
                }
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid signature"})),
            );
        }
    }

    // Parse JSON body
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid JSON payload"})),
        );
    };

    // Parse messages from the webhook payload
    let messages = linq.parse_webhook_payload(&payload);

    if messages.is_empty() {
        // Acknowledge the webhook even if no messages (could be status/delivery events)
        return (StatusCode::OK, Json(serde_json::json!({"status": "ok"})));
    }

    // Process each message
    for msg in &messages {
        tracing::info!(
            "Linq message from {}: {}",
            msg.sender,
            truncate_with_ellipsis(&msg.content, 50)
        );
        let session_id = sender_session_id("linq", msg);

        // Auto-save to memory
        if state.auto_save && !operant_memory::should_skip_autosave_content(&msg.content) {
            let key = linq_memory_key(msg);
            let _ = state
                .mem
                .store(
                    &key,
                    &msg.content,
                    MemoryCategory::Conversation,
                    Some(&session_id),
                )
                .await;
        }

        // Call the LLM
        match Box::pin(run_gateway_chat_with_tools(
            &state,
            &msg.content,
            Some(&session_id),
        ))
        .await
        {
            Ok(GatewayChatOutcome { response, .. }) => {
                // Send reply via Linq
                if let Err(e) = linq
                    .send(&SendMessage::new(response, &msg.reply_target))
                    .await
                {
                    tracing::error!("Failed to send Linq reply: {e}");
                }
            }
            Err(e) => {
                let reply = if is_needs_onboarding_err(&e) {
                    tracing::warn!(
                        "Linq chat refused: gateway has no model configured; \
                         visit /onboard"
                    );
                    needs_onboarding_channel_reply()
                } else {
                    tracing::error!("LLM error for Linq message: {e:#}");
                    "Sorry, I couldn't process your message right now.".to_string()
                };
                let _ = linq.send(&SendMessage::new(reply, &msg.reply_target)).await;
            }
        }
    }

    // Acknowledge the webhook
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// GET /wati — WATI webhook verification (echoes hub.challenge)
async fn handle_wati_verify(
    #[allow(dead_code)] State(state): State<AppState>,
    Query(params): Query<WatiVerifyQuery>,
) -> impl IntoResponse {
    if state.wati.is_none() {
        return (StatusCode::NOT_FOUND, "WATI not configured".to_string());
    }

    // WATI may use Meta-style webhook verification; echo the challenge
    if let Some(challenge) = params.challenge {
        tracing::info!("WATI webhook verified successfully");
        return (StatusCode::OK, challenge);
    }

    (StatusCode::BAD_REQUEST, "Missing hub.challenge".to_string())
}

#[derive(Debug, serde::Deserialize)]
pub struct WatiVerifyQuery {
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}

/// POST /wati — incoming WATI WhatsApp message webhook
async fn handle_wati_webhook(State(state): State<AppState>, body: Bytes) -> impl IntoResponse {
    #[allow(dead_code)]
    let Some(ref wati) = state.wati else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "WATI not configured"})),
        );
    };

    // Parse JSON body
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid JSON payload"})),
        );
    };

    // Detect audio before the synchronous parse
    let msg_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let messages = if matches!(msg_type, "audio" | "voice") {
        // Build a synthetic ChannelMessage from the audio transcript
        if let Some(transcript) = wati.try_transcribe_audio(&payload).await {
            wati.parse_audio_as_message(&payload, transcript)
        } else {
            vec![]
        }
    } else {
        wati.parse_webhook_payload(&payload)
    };

    if messages.is_empty() {
        return (StatusCode::OK, Json(serde_json::json!({"status": "ok"})));
    }

    // Process each message
    for msg in &messages {
        tracing::info!(
            "WATI message from {}: {}",
            msg.sender,
            truncate_with_ellipsis(&msg.content, 50)
        );
        let session_id = sender_session_id("wati", msg);

        // Auto-save to memory
        if state.auto_save && !operant_memory::should_skip_autosave_content(&msg.content) {
            let key = wati_memory_key(msg);
            let _ = state
                .mem
                .store(
                    &key,
                    &msg.content,
                    MemoryCategory::Conversation,
                    Some(&session_id),
                )
                .await;
        }

        // Call the LLM
        match Box::pin(run_gateway_chat_with_tools(
            &state,
            &msg.content,
            Some(&session_id),
        ))
        .await
        {
            Ok(GatewayChatOutcome { response, .. }) => {
                // Send reply via WATI
                if let Err(e) = wati
                    .send(&SendMessage::new(response, &msg.reply_target))
                    .await
                {
                    tracing::error!("Failed to send WATI reply: {e}");
                }
            }
            Err(e) => {
                let reply = if is_needs_onboarding_err(&e) {
                    tracing::warn!(
                        "WATI chat refused: gateway has no model configured; \
                         visit /onboard"
                    );
                    needs_onboarding_channel_reply()
                } else {
                    tracing::error!("LLM error for WATI message: {e:#}");
                    "Sorry, I couldn't process your message right now.".to_string()
                };
                let _ = wati.send(&SendMessage::new(reply, &msg.reply_target)).await;
            }
        }
    }

    // Acknowledge the webhook
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// POST /nextcloud-talk — incoming message webhook (Nextcloud Talk bot API)
async fn handle_nextcloud_talk_webhook(
    #[allow(dead_code)] State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref nextcloud_talk) = state.nextcloud_talk else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Nextcloud Talk not configured"})),
        );
    };

    let body_str = String::from_utf8_lossy(&body);

    // ── Security: Verify Nextcloud Talk HMAC signature if secret is configured ──
    if let Some(ref webhook_secret) = state.nextcloud_talk_webhook_secret {
        let random = headers
            .get("X-Nextcloud-Talk-Random")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let signature = headers
            .get("X-Nextcloud-Talk-Signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !operant_channels::nextcloud_talk::verify_nextcloud_talk_signature(
            webhook_secret,
            random,
            &body_str,
            signature,
        ) {
            tracing::warn!(
                "Nextcloud Talk webhook signature verification failed (signature: {})",
                if signature.is_empty() {
                    "missing"
                } else {
                    "invalid"
                }
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid signature"})),
            );
        }
    }

    // Parse JSON body
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid JSON payload"})),
        );
    };

    // Parse messages from webhook payload
    let messages = nextcloud_talk.parse_webhook_payload(&payload);
    if messages.is_empty() {
        // Acknowledge webhook even if payload does not contain actionable user messages.
        return (StatusCode::OK, Json(serde_json::json!({"status": "ok"})));
    }

    // Spawn per-message processing so the webhook returns 200 quickly.
    // Nextcloud Talk cancels webhook requests that don't complete within ~5s
    // (see #6156); slow local models routinely exceed that. Each message gets
    // its own task — the LLM call and reply are independent of the ack.
    for msg in messages {
        let state = state.clone();
        let nextcloud_talk = Arc::clone(nextcloud_talk);
        tokio::spawn(async move {
            tracing::info!(
                "Nextcloud Talk message from {}: {}",
                msg.sender,
                truncate_with_ellipsis(&msg.content, 50)
            );
            let session_id = sender_session_id("nextcloud_talk", &msg);

            if state.auto_save && !operant_memory::should_skip_autosave_content(&msg.content) {
                let key = nextcloud_talk_memory_key(&msg);
                let _ = state
                    .mem
                    .store(
                        &key,
                        &msg.content,
                        MemoryCategory::Conversation,
                        Some(&session_id),
                    )
                    .await;
            }

            match Box::pin(run_gateway_chat_with_tools(
                &state,
                &msg.content,
                Some(&session_id),
            ))
            .await
            {
                Ok(GatewayChatOutcome { response, .. }) => {
                    if let Err(e) = nextcloud_talk
                        .send(&SendMessage::new(response, &msg.reply_target))
                        .await
                    {
                        tracing::error!("Failed to send Nextcloud Talk reply: {e}");
                    }
                }
                Err(e) => {
                    let reply = if is_needs_onboarding_err(&e) {
                        tracing::warn!(
                            "Nextcloud Talk chat refused: gateway has no model configured; \
                             visit /onboard"
                        );
                        needs_onboarding_channel_reply()
                    } else {
                        tracing::error!("LLM error for Nextcloud Talk message: {e:#}");
                        "Sorry, I couldn't process your message right now.".to_string()
                    };
                    let _ = nextcloud_talk
                        .send(&SendMessage::new(reply, &msg.reply_target))
                        .await;
                }
            }
        });
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// Maximum request body size for the Gmail webhook endpoint (1 MB).
/// Google Pub/Sub messages are typically under 10 KB.
const GMAIL_WEBHOOK_MAX_BODY: usize = 1024 * 1024;
#[allow(dead_code)]
/// POST /webhook/gmail — incoming Gmail Pub/Sub push notification
async fn handle_gmail_push_webhook(
    #[allow(dead_code)] State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref gmail_push) = state.gmail_push else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Gmail push not configured"})),
        );
    };

    // Enforce body size limit.
    if body.len() > GMAIL_WEBHOOK_MAX_BODY {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Request body too large"})),
        );
    }

    // Authenticate the webhook request using a shared secret.
    let secret = gmail_push.resolve_webhook_secret();
    if !secret.is_empty() {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|auth| auth.strip_prefix("Bearer "))
            .unwrap_or("");

        if provided != secret {
            tracing::warn!("Gmail push webhook: unauthorized request");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Unauthorized"})),
            );
        }
    }

    let body_str = String::from_utf8_lossy(&body);
    let envelope: operant_channels::gmail_push::PubSubEnvelope =
        match serde_json::from_str(&body_str) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Gmail push webhook: invalid payload: {e}");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid Pub/Sub envelope"})),
                );
            }
        };

    // Process the notification asynchronously (non-blocking for the webhook response)
    let channel = Arc::clone(gmail_push);
    tokio::spawn(async move {
        if let Err(e) = channel.handle_notification(&envelope).await {
            tracing::error!("Gmail push notification processing failed: {e:#}");
        }
    });

    // Acknowledge immediately — Google Pub/Sub requires a 2xx within ~10s
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

// ══════════════════════════════════════════════════════════════════════════════
// ADMIN HANDLERS (for CLI management)
// ══════════════════════════════════════════════════════════════════════════════

/// Response for admin endpoints
#[derive(serde::Serialize)]
struct AdminResponse {
    #[allow(dead_code)]
    success: bool,
    message: String,
}

/// Reject requests that do not originate from a loopback address.
fn require_localhost(peer: &SocketAddr) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    #[allow(dead_code)]
    if peer.ip().is_loopback() {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Admin endpoints are restricted to localhost"
            })),
        ))
    }
}

/// POST /admin/shutdown — graceful shutdown from CLI (localhost only)
async fn handle_admin_shutdown(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_localhost(&peer)?;
    tracing::info!("🔌 Admin shutdown request received — initiating graceful shutdown");

    let body = AdminResponse {
        success: true,
        message: "Gateway shutdown initiated".to_string(),
    };

    let _ = state.shutdown_tx.send(true);

    Ok((StatusCode::OK, Json(body)))
}

/// POST /admin/reload — reload the daemon in place (localhost only).
///
/// Sends `true` on the reload channel the daemon owns. The daemon's main
/// wait loop sees the change, returns `DaemonExit::Reload`, and the outer
/// loop in `src/main.rs` re-reads config from disk and re-runs
/// `daemon::run` — re-instantiating every subsystem (gateway / channels /
/// heartbeat / scheduler / mqtt) with the fresh config.
///
/// Same PID throughout. Brief HTTP downtime while the gateway listener
/// rebinds — typically sub-second. Clients should poll `/health` to detect
/// when the new instance is ready.
///
/// Cross-platform — works identically on Linux, macOS, and Windows because
/// the channel is in-process tokio, not an OS signal. The gateway-only
/// `operant gateway start` (no daemon supervisor) returns 503 with a
/// clear message because there's nothing to signal.
async fn handle_admin_reload(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_localhost(&peer)?;

    let Some(reload_tx) = state.reload_tx.clone() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "no daemon supervisor — running as standalone gateway. \
                          Restart the process to pick up config changes."
            })),
        ));
    };

    tracing::info!("🔄 Admin reload request received");
    // Trigger graceful shutdown of THIS gateway instance's axum::serve so
    // its TcpListener releases the port before the daemon supervisor
    // spawns the new instance. Without this, daemon::run aborts the
    // gateway tokio task at the next await point — but the OLD listener
    // can stay bound briefly, racing the NEW gateway's bind. The new
    // bind then fails and spawn_component_supervisor backs off; in the
    // meantime the OLD gateway keeps serving requests with stale
    // in-memory config, and `/api/config/drift` reports drift against
    // disk because in-memory hasn't been replaced yet. Cold restart
    // (process exit + start) hits this path differently because the OS
    // fully releases the listener — that's why the user observes "shut
    // down + bring up = correct" but "/admin/reload = stale".
    let shutdown_tx = state.shutdown_tx.clone();
    // Brief delay so the HTTP response flushes before tear-down begins.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // Drain axum first so the listener releases.
        let _ = shutdown_tx.send(true);
        // Then signal the daemon to re-read disk and re-spawn subsystems.
        let _ = reload_tx.send(true);
    });

    Ok((
        StatusCode::OK,
        Json(AdminResponse {
            success: true,
            message: "Daemon reload initiated".to_string(),
        }),
    ))
}

/// GET /admin/paircode — fetch current pairing code (localhost only)
async fn handle_admin_paircode(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_localhost(&peer)?;
    let code = state.pairing.pairing_code();

    let body = if let Some(c) = code {
        serde_json::json!({
            "success": true,
            "pairing_required": state.pairing.require_pairing(),
            "pairing_code": c,
            "message": "Use this one-time code to pair"
        })
    } else {
        serde_json::json!({
            "success": true,
            "pairing_required": state.pairing.require_pairing(),
            "pairing_code": null,
            "message": if state.pairing.require_pairing() {
                "Pairing is active but no new code available (already paired or code expired)"
            } else {
                "Pairing is disabled for this gateway"
            }
        })
    };

    Ok((StatusCode::OK, Json(body)))
}

/// POST /admin/paircode/new — generate a new pairing code (localhost only)
async fn handle_admin_paircode_new(
    #[allow(dead_code)] State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_localhost(&peer)?;
    match state.pairing.generate_new_pairing_code() {
        Some(code) => {
            tracing::info!("🔐 New pairing code generated via admin endpoint");
            let body = serde_json::json!({
                "success": true,
                "pairing_required": state.pairing.require_pairing(),
                "pairing_code": code,
                "message": "New pairing code generated — use this one-time code to pair"
            });
            Ok((StatusCode::OK, Json(body)))
        }
        None => {
            let body = serde_json::json!({
                "success": false,
                "pairing_required": false,
                "pairing_code": null,
                "message": "Pairing is disabled for this gateway"
            });
            Ok((StatusCode::BAD_REQUEST, Json(body)))
        }
    }
}

/// GET /pair/code — fetch the initial pairing code (no auth, no localhost restriction).
///
/// This endpoint is intentionally public so that Docker and remote users can see
/// the pairing code on the web dashboard without needing terminal access. It only
/// returns a code when the gateway is in its initial un-paired state (no devices
/// paired yet and a pairing code exists). Once the first device pairs, this
/// endpoint stops returning a code.
async fn handle_pair_code(State(state): State<AppState>) -> impl IntoResponse {
    #[allow(dead_code)]
    let require = state.pairing.require_pairing();
    let is_paired = state.pairing.is_paired();

    // Only expose the code during initial setup (before first pairing)
    let code = if require && !is_paired {
        state.pairing.pairing_code()
    } else {
        None
    };

    let body = serde_json::json!({
        "success": true,
        "pairing_required": require,
        "pairing_code": code,
    });

    (StatusCode::OK, Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::http::HeaderValue;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;
    use operant_api::channel::ChannelMessage;
    use operant_memory::{Memory, MemoryCategory, MemoryEntry};
    use operant_providers::Provider;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Generate a random hex secret at runtime to avoid hard-coded cryptographic values.
    fn generate_test_secret() -> String {
        let bytes: [u8; 32] = rand::random();
        hex::encode(bytes)
    }

    #[test]
    fn security_body_limit_is_64kb() {
        assert_eq!(MAX_BODY_SIZE, 65_536);
    }

    #[test]
    fn security_timeout_default_is_30_seconds() {
        assert_eq!(REQUEST_TIMEOUT_SECS, 30);
    }

    #[test]
    fn paircode_recovery_command_includes_alternate_port() {
        assert_eq!(
            format_paircode_recovery_command("127.0.0.1", 42617),
            "operant gateway get-paircode --new --port 42617"
        );
    }

    #[test]
    fn paircode_recovery_command_includes_specific_host_when_needed() {
        assert_eq!(
            format_paircode_recovery_command("192.168.1.20", 42617),
            "operant gateway get-paircode --new --port 42617 --host 192.168.1.20"
        );
    }

    #[test]
    fn paircode_recovery_curl_targets_running_instance() {
        assert_eq!(
            format_paircode_recovery_curl("127.0.0.1", 42617, ""),
            "curl -s -X POST http://127.0.0.1:42617/admin/paircode/new"
        );
    }

    #[test]
    fn paircode_recovery_curl_preserves_path_prefix() {
        assert_eq!(
            format_paircode_recovery_curl("127.0.0.1", 42617, "/gw"),
            "curl -s -X POST http://127.0.0.1:42617/gw/admin/paircode/new"
        );
    }

    #[test]
    fn gateway_timeout_falls_back_to_default() {
        // When env var is not set, should return the default constant
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_TIMEOUT_SECS") };
        assert_eq!(gateway_request_timeout_secs(), 30);
    }

    #[test]
    fn long_running_request_timeout_default_is_ten_minutes() {
        assert_eq!(LONG_RUNNING_REQUEST_TIMEOUT_SECS, 600);
    }

    #[test]
    fn long_running_request_timeout_falls_back_to_default() {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_LONG_RUNNING_REQUEST_TIMEOUT_SECS") };
        assert_eq!(gateway_long_running_request_timeout_secs(), 600);
    }

    #[test]
    fn webhook_body_requires_message_field() {
        let valid = r#"{"message": "hello"}"#;
        let parsed: Result<WebhookBody, _> = serde_json::from_str(valid);
        assert!(parsed.is_ok());
        assert_eq!(parsed.unwrap().message, "hello");

        let missing = r#"{"other": "field"}"#;
        let parsed: Result<WebhookBody, _> = serde_json::from_str(missing);
        assert!(parsed.is_err());
    }

    #[test]
    fn whatsapp_query_fields_are_optional() {
        let q = WhatsAppVerifyQuery {
            mode: None,
            verify_token: None,
            challenge: None,
        };
        assert!(q.mode.is_none());
    }

    #[test]
    fn app_state_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<AppState>();
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_hint_when_prometheus_is_disabled() {
        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider: Arc::new(MockProvider::default()),
            model: "test-model".into(),
            temperature: 0.0,
            mem: Arc::new(MockMemory),
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let response = handle_metrics(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(PROMETHEUS_CONTENT_TYPE)
        );

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("Prometheus backend not enabled"));
    }

    #[cfg(feature = "observability-prometheus")]
    #[tokio::test]
    async fn metrics_endpoint_renders_prometheus_output() {
        let event_tx = tokio::sync::broadcast::channel(16).0;
        let prom = operant_runtime::observability::PrometheusObserver::new();
        operant_runtime::observability::Observer::record_event(
            &prom,
            &operant_runtime::observability::ObserverEvent::HeartbeatTick,
        );

        let observer: Arc<dyn operant_runtime::observability::Observer> = Arc::new(prom);
        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider: Arc::new(MockProvider::default()),
            model: "test-model".into(),
            temperature: 0.0,
            mem: Arc::new(MockMemory),
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer,
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let response = handle_metrics(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("operant_heartbeat_ticks_total 1"));
    }

    #[test]
    fn gateway_rate_limiter_blocks_after_limit() {
        let limiter = GatewayRateLimiter::new(2, 2, 100);
        assert!(limiter.allow_pair("127.0.0.1"));
        assert!(limiter.allow_pair("127.0.0.1"));
        assert!(!limiter.allow_pair("127.0.0.1"));
    }

    #[test]
    fn rate_limiter_sweep_removes_stale_entries() {
        let limiter = SlidingWindowRateLimiter::new(10, Duration::from_secs(60), 100);
        // Add entries for multiple IPs
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-2"));
        assert!(limiter.allow("ip-3"));

        {
            let guard = limiter.requests.lock();
            assert_eq!(guard.0.len(), 3);
        }

        // Force a sweep by backdating last_sweep
        {
            let mut guard = limiter.requests.lock();
            guard.1 = Instant::now()
                .checked_sub(Duration::from_secs(RATE_LIMITER_SWEEP_INTERVAL_SECS + 1))
                .unwrap();
            // Clear timestamps for ip-2 and ip-3 to simulate stale entries
            guard.0.get_mut("ip-2").unwrap().clear();
            guard.0.get_mut("ip-3").unwrap().clear();
        }

        // Next allow() call should trigger sweep and remove stale entries
        assert!(limiter.allow("ip-1"));

        {
            let guard = limiter.requests.lock();
            assert_eq!(guard.0.len(), 1, "Stale entries should have been swept");
            assert!(guard.0.contains_key("ip-1"));
        }
    }

    #[test]
    fn rate_limiter_zero_limit_always_allows() {
        let limiter = SlidingWindowRateLimiter::new(0, Duration::from_secs(60), 10);
        for _ in 0..100 {
            assert!(limiter.allow("any-key"));
        }
    }

    #[test]
    fn idempotency_store_rejects_duplicate_key() {
        let store = IdempotencyStore::new(Duration::from_secs(30), 10);
        assert!(store.record_if_new("req-1"));
        assert!(!store.record_if_new("req-1"));
        assert!(store.record_if_new("req-2"));
    }

    #[test]
    fn rate_limiter_bounded_cardinality_evicts_oldest_key() {
        let limiter = SlidingWindowRateLimiter::new(5, Duration::from_secs(60), 2);
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-2"));
        assert!(limiter.allow("ip-3"));

        let guard = limiter.requests.lock();
        assert_eq!(guard.0.len(), 2);
        assert!(guard.0.contains_key("ip-2"));
        assert!(guard.0.contains_key("ip-3"));
    }

    #[test]
    fn idempotency_store_bounded_cardinality_evicts_oldest_key() {
        let store = IdempotencyStore::new(Duration::from_secs(300), 2);
        assert!(store.record_if_new("k1"));
        std::thread::sleep(Duration::from_millis(2));
        assert!(store.record_if_new("k2"));
        std::thread::sleep(Duration::from_millis(2));
        assert!(store.record_if_new("k3"));

        let keys = store.keys.lock();
        assert_eq!(keys.len(), 2);
        assert!(!keys.contains_key("k1"));
        assert!(keys.contains_key("k2"));
        assert!(keys.contains_key("k3"));
    }

    #[test]
    fn client_key_defaults_to_peer_addr_when_untrusted_proxy_mode() {
        let peer = SocketAddr::from(([10, 0, 0, 5], 42617));
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Forwarded-For",
            HeaderValue::from_static("198.51.100.10, 203.0.113.11"),
        );

        let key = client_key_from_request(Some(peer), &headers, false);
        assert_eq!(key, "10.0.0.5");
    }

    #[test]
    fn client_key_uses_forwarded_ip_only_in_trusted_proxy_mode() {
        let peer = SocketAddr::from(([10, 0, 0, 5], 42617));
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Forwarded-For",
            HeaderValue::from_static("198.51.100.10, 203.0.113.11"),
        );

        let key = client_key_from_request(Some(peer), &headers, true);
        assert_eq!(key, "198.51.100.10");
    }

    #[test]
    fn client_key_falls_back_to_peer_when_forwarded_header_invalid() {
        let peer = SocketAddr::from(([10, 0, 0, 5], 42617));
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", HeaderValue::from_static("garbage-value"));

        let key = client_key_from_request(Some(peer), &headers, true);
        assert_eq!(key, "10.0.0.5");
    }

    #[test]
    fn normalize_max_keys_uses_fallback_for_zero() {
        assert_eq!(normalize_max_keys(0, 10_000), 10_000);
        assert_eq!(normalize_max_keys(0, 0), 1);
    }

    #[test]
    fn normalize_max_keys_preserves_nonzero_values() {
        assert_eq!(normalize_max_keys(2_048, 10_000), 2_048);
        assert_eq!(normalize_max_keys(1, 10_000), 1);
    }

    #[tokio::test]
    async fn persist_pairing_tokens_writes_config_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let workspace_path = temp.path().join("workspace");

        let config = Config {
            config_path: config_path.clone(),
            workspace_dir: workspace_path,
            ..Default::default()
        };
        config.save().await.unwrap();

        let guard = PairingGuard::new(true, &[]);
        let code = guard.pairing_code().unwrap();
        let token = guard.try_pair(&code, "test_client").await.unwrap().unwrap();
        assert!(guard.is_authenticated(&token));

        let shared_config = Arc::new(Mutex::new(config));
        Box::pin(persist_pairing_tokens(shared_config.clone(), &guard))
            .await
            .unwrap();

        // In-memory tokens should remain as plaintext 64-char hex hashes.
        let plaintext = {
            let in_memory = shared_config.lock();
            assert_eq!(in_memory.gateway.paired_tokens.len(), 1);
            in_memory.gateway.paired_tokens[0].clone()
        };
        assert_eq!(plaintext.len(), 64);
        assert!(plaintext.chars().all(|c: char| c.is_ascii_hexdigit()));

        // On disk, the token should be encrypted (secrets.encrypt defaults to true).
        let saved = tokio::fs::read_to_string(config_path).await.unwrap();
        let raw_parsed: Config = toml::from_str(&saved).unwrap();
        assert_eq!(raw_parsed.gateway.paired_tokens.len(), 1);
        let on_disk = &raw_parsed.gateway.paired_tokens[0];
        assert!(
            operant_runtime::security::SecretStore::is_encrypted(on_disk),
            "paired_token should be encrypted on disk"
        );
    }

    #[test]
    fn webhook_memory_key_is_unique() {
        let key1 = webhook_memory_key();
        let key2 = webhook_memory_key();

        assert!(key1.starts_with("webhook_msg_"));
        assert!(key2.starts_with("webhook_msg_"));
        assert_ne!(key1, key2);
    }

    #[test]
    fn webhook_session_id_accepts_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Session-Id", HeaderValue::from_static("abc-DEF_123.foo"));
        assert_eq!(webhook_session_id(&headers), Some("abc-DEF_123.foo".into()));
    }

    #[test]
    fn webhook_session_id_trims_whitespace() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Session-Id", HeaderValue::from_static("  my-session  "));
        assert_eq!(webhook_session_id(&headers), Some("my-session".into()));
    }

    #[test]
    fn webhook_session_id_rejects_empty() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Session-Id", HeaderValue::from_static(""));
        assert_eq!(webhook_session_id(&headers), None);

        headers.insert("X-Session-Id", HeaderValue::from_static("   "));
        assert_eq!(webhook_session_id(&headers), None);
    }

    #[test]
    fn webhook_session_id_rejects_missing() {
        let headers = HeaderMap::new();
        assert_eq!(webhook_session_id(&headers), None);
    }

    #[test]
    fn webhook_session_id_rejects_oversized() {
        let mut headers = HeaderMap::new();
        let long = "a".repeat(129);
        headers.insert("X-Session-Id", HeaderValue::from_str(&long).unwrap());
        assert_eq!(webhook_session_id(&headers), None);

        let at_limit = "b".repeat(128);
        headers.insert("X-Session-Id", HeaderValue::from_str(&at_limit).unwrap());
        assert!(webhook_session_id(&headers).is_some());
    }

    #[test]
    fn webhook_session_id_rejects_invalid_chars() {
        let mut headers = HeaderMap::new();
        for bad in &[
            "has/slash",
            "has:colon",
            "has space",
            "has@at",
            "emoji\u{1f600}",
        ] {
            if let Ok(val) = HeaderValue::from_str(bad) {
                headers.insert("X-Session-Id", val);
                assert_eq!(webhook_session_id(&headers), None, "should reject: {bad}");
            }
        }
    }

    #[test]
    fn whatsapp_memory_key_includes_sender_and_message_id() {
        let msg = ChannelMessage {
            id: "wamid-123".into(),
            sender: "+1234567890".into(),
            reply_target: "+1234567890".into(),
            content: "hello".into(),
            channel: "whatsapp".into(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        };

        let key = whatsapp_memory_key(&msg);
        assert_eq!(key, "whatsapp_+1234567890_wamid-123");
    }

    #[derive(Default)]
    struct MockMemory;

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> MemoryResult<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
            _since: Option<&str>,
            _until: Option<&str>,
        ) -> MemoryResult<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _key: &str) -> MemoryResult<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> MemoryResult<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> MemoryResult<bool> {
            Ok(false)
        }

        async fn count(&self) -> MemoryResult<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct MockProvider {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("ok".into())
        }
    }

    #[derive(Default)]
    struct TrackingMemory {
        keys: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Memory for TrackingMemory {
        fn name(&self) -> &str {
            "tracking"
        }

        async fn store(
            &self,
            key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> MemoryResult<()> {
            self.keys.lock().push(key.to_string());
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
            _since: Option<&str>,
            _until: Option<&str>,
        ) -> MemoryResult<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _key: &str) -> MemoryResult<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> MemoryResult<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> MemoryResult<bool> {
            Ok(false)
        }

        async fn count(&self) -> MemoryResult<usize> {
            let size = self.keys.lock().len();
            Ok(size)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    fn test_connect_info() -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 30_300)))
    }

    #[tokio::test]
    async fn webhook_idempotency_skips_duplicate_provider_calls() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let mut headers = HeaderMap::new();
        headers.insert("X-Idempotency-Key", HeaderValue::from_static("abc-123"));

        let body = Ok(Json(WebhookBody {
            message: "hello".into(),
        }));
        let first = handle_webhook(
            State(state.clone()),
            test_connect_info(),
            headers.clone(),
            body,
        )
        .await
        .into_response();
        assert_eq!(first.status(), StatusCode::OK);

        let body = Ok(Json(WebhookBody {
            message: "hello".into(),
        }));
        let second = handle_webhook(State(state), test_connect_info(), headers, body)
            .await
            .into_response();
        assert_eq!(second.status(), StatusCode::OK);

        let payload = second.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(parsed["status"], "duplicate");
        assert_eq!(parsed["idempotent"], true);
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn webhook_autosave_stores_distinct_keys_per_request() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let tracking_impl = Arc::new(TrackingMemory::default());
        let memory: Arc<dyn Memory> = tracking_impl.clone();

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: true,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let headers = HeaderMap::new();

        let body1 = Ok(Json(WebhookBody {
            message: "hello one".into(),
        }));
        let first = handle_webhook(
            State(state.clone()),
            test_connect_info(),
            headers.clone(),
            body1,
        )
        .await
        .into_response();
        assert_eq!(first.status(), StatusCode::OK);

        let body2 = Ok(Json(WebhookBody {
            message: "hello two".into(),
        }));
        let second = handle_webhook(State(state), test_connect_info(), headers, body2)
            .await
            .into_response();
        assert_eq!(second.status(), StatusCode::OK);

        let keys = tracking_impl.keys.lock().clone();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
        assert!(keys[0].starts_with("webhook_msg_"));
        assert!(keys[1].starts_with("webhook_msg_"));
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn webhook_secret_hash_is_deterministic_and_nonempty() {
        let secret_a = generate_test_secret();
        let secret_b = generate_test_secret();
        let one = hash_webhook_secret(&secret_a);
        let two = hash_webhook_secret(&secret_a);
        let other = hash_webhook_secret(&secret_b);

        assert_eq!(one, two);
        assert_ne!(one, other);
        assert_eq!(one.len(), 64);
    }

    #[tokio::test]
    async fn webhook_secret_hash_rejects_missing_header() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);
        let secret = generate_test_secret();

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&secret))),
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let response = handle_webhook(
            State(state),
            test_connect_info(),
            HeaderMap::new(),
            Ok(Json(WebhookBody {
                message: "hello".into(),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn webhook_secret_hash_rejects_invalid_header() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);
        let valid_secret = generate_test_secret();
        let wrong_secret = generate_test_secret();

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&valid_secret))),
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Webhook-Secret",
            HeaderValue::from_str(&wrong_secret).unwrap(),
        );

        let response = handle_webhook(
            State(state),
            test_connect_info(),
            headers,
            Ok(Json(WebhookBody {
                message: "hello".into(),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn webhook_secret_hash_accepts_valid_header() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);
        let secret = generate_test_secret();

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: Some(Arc::from(hash_webhook_secret(&secret))),
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let mut headers = HeaderMap::new();
        headers.insert("X-Webhook-Secret", HeaderValue::from_str(&secret).unwrap());

        let response = handle_webhook(
            State(state),
            test_connect_info(),
            headers,
            Ok(Json(WebhookBody {
                message: "hello".into(),
            })),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 1);
    }

    fn compute_nextcloud_signature_hex(secret: &str, random: &str, body: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let payload = format!("{random}{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[tokio::test]
    async fn nextcloud_talk_webhook_returns_not_found_when_not_configured() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::default());
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let response = Box::pin(handle_nextcloud_talk_webhook(
            State(state),
            HeaderMap::new(),
            Bytes::from_static(br#"{"type":"message"}"#),
        ))
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn nextcloud_talk_webhook_rejects_invalid_signature() {
        let provider_impl = Arc::new(MockProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);

        let channel = Arc::new(NextcloudTalkChannel::new(
            "https://cloud.example.com".into(),
            "app-token".into(),
            String::new(),
            vec!["*".into()],
        ));

        let secret = "nextcloud-test-secret";
        let random = "seed-value";
        let body = r#"{"type":"message","object":{"token":"room-token"},"message":{"actorType":"users","actorId":"user_a","message":"hello"}}"#;
        let _valid_signature = compute_nextcloud_signature_hex(secret, random, body);
        let invalid_signature = "deadbeef";

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: Some(channel),
            nextcloud_talk_webhook_secret: Some(Arc::from(secret)),
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Nextcloud-Talk-Random",
            HeaderValue::from_str(random).unwrap(),
        );
        headers.insert(
            "X-Nextcloud-Talk-Signature",
            HeaderValue::from_str(invalid_signature).unwrap(),
        );

        let response = Box::pin(handle_nextcloud_talk_webhook(
            State(state),
            headers,
            Bytes::from(body),
        ))
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 0);
    }

    // Regression for #6156: handler must return 200 OK before the (potentially
    // slow) LLM call completes, so Nextcloud Talk doesn't cancel the webhook
    // request at its ~5s timeout.
    #[derive(Default)]
    struct SlowProvider {
        calls: AtomicUsize,
        started_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl Provider for SlowProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(tx) = self.started_tx.lock().take() {
                let _ = tx.send(());
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok("slow ok".into())
        }
    }

    #[tokio::test]
    async fn nextcloud_talk_webhook_returns_before_llm_call_completes() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let provider_impl = Arc::new(SlowProvider {
            calls: AtomicUsize::new(0),
            started_tx: Mutex::new(Some(started_tx)),
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let memory: Arc<dyn Memory> = Arc::new(MockMemory);

        let channel = Arc::new(NextcloudTalkChannel::new(
            "https://cloud.example.com".into(),
            "app-token".into(),
            String::new(),
            vec!["*".into()],
        ));

        let body = r#"{"type":"message","object":{"token":"room-token"},"actor":{"id":"user_a","name":"User A"},"message":{"actorType":"users","actorId":"user_a","message":"hello"}}"#;

        let state = AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider,
            model: "test-model".into(),
            temperature: 0.0,
            mem: memory,
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(PairingGuard::new(false, &[])),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(GatewayRateLimiter::new(100, 100, 100)),
            auth_limiter: Arc::new(auth_rate_limit::AuthRateLimiter::new()),
            idempotency_store: Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000)),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: Some(channel),
            nextcloud_talk_webhook_secret: None,
            wati: None,
            gmail_push: None,
            observer: Arc::new(operant_runtime::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(16).0,
            event_buffer: Arc::new(sse::EventBuffer::new(16)),
            shutdown_tx: tokio::sync::watch::channel(false).0,
            reload_tx: None,
            node_registry: Arc::new(nodes::NodeRegistry::new(16)),
            path_prefix: String::new(),
            web_dist_dir: None,
            session_backend: None,
            session_queue: std::sync::Arc::new(crate::session_queue::SessionActorQueue::new(
                8, 30, 600,
            )),
            device_registry: None,
            pending_pairings: None,
            canvas_store: CanvasStore::new(),
            cancel_tokens: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            #[cfg(feature = "webauthn")]
            webauthn: None,
        };

        let start = std::time::Instant::now();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            Box::pin(handle_nextcloud_talk_webhook(
                State(state),
                HeaderMap::new(),
                Bytes::from(body),
            )),
        )
        .await
        .expect("webhook must return before 2s deadline (regression #6156)")
        .into_response();

        let elapsed = start.elapsed();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            elapsed < Duration::from_secs(2),
            "handler returned after {elapsed:?}; expected fast return for #6156"
        );

        // Confirm the spawned task actually started the LLM call (i.e., the
        // ack didn't just skip processing). The 30s sleep is still in flight.
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .expect("spawned LLM call did not start within 2s")
            .expect("started_tx sender was dropped");
        assert_eq!(provider_impl.calls.load(Ordering::SeqCst), 1);
    }

    // ══════════════════════════════════════════════════════════
    // WhatsApp Signature Verification Tests (CWE-345 Prevention)
    // ══════════════════════════════════════════════════════════

    fn compute_whatsapp_signature_hex(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn compute_whatsapp_signature_header(secret: &str, body: &[u8]) -> String {
        format!("sha256={}", compute_whatsapp_signature_hex(secret, body))
    }

    #[test]
    fn whatsapp_signature_valid() {
        let app_secret = generate_test_secret();
        let body = b"test body content";

        let signature_header = compute_whatsapp_signature_header(&app_secret, body);

        assert!(verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_invalid_wrong_secret() {
        let app_secret = generate_test_secret();
        let wrong_secret = generate_test_secret();
        let body = b"test body content";

        let signature_header = compute_whatsapp_signature_header(&wrong_secret, body);

        assert!(!verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_invalid_wrong_body() {
        let app_secret = generate_test_secret();
        let original_body = b"original body";
        let tampered_body = b"tampered body";

        let signature_header = compute_whatsapp_signature_header(&app_secret, original_body);

        // Verify with tampered body should fail
        assert!(!verify_whatsapp_signature(
            &app_secret,
            tampered_body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_missing_prefix() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        // Signature without "sha256=" prefix
        let signature_header = "abc123def456";

        assert!(!verify_whatsapp_signature(
            &app_secret,
            body,
            signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_empty_header() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        assert!(!verify_whatsapp_signature(&app_secret, body, ""));
    }

    #[test]
    fn whatsapp_signature_invalid_hex() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        // Invalid hex characters
        let signature_header = "sha256=not_valid_hex_zzz";

        assert!(!verify_whatsapp_signature(
            &app_secret,
            body,
            signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_empty_body() {
        let app_secret = generate_test_secret();
        let body = b"";

        let signature_header = compute_whatsapp_signature_header(&app_secret, body);

        assert!(verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_unicode_body() {
        let app_secret = generate_test_secret();
        let body = "Hello 🦀 World".as_bytes();

        let signature_header = compute_whatsapp_signature_header(&app_secret, body);

        assert!(verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_json_payload() {
        let app_secret = generate_test_secret();
        let body = br#"{"entry":[{"changes":[{"value":{"messages":[{"from":"1234567890","text":{"body":"Hello"}}]}}]}]}"#;

        let signature_header = compute_whatsapp_signature_header(&app_secret, body);

        assert!(verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_case_sensitive_prefix() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);

        // Wrong case prefix should fail
        let wrong_prefix = format!("SHA256={hex_sig}");
        assert!(!verify_whatsapp_signature(&app_secret, body, &wrong_prefix));

        // Correct prefix should pass
        let correct_prefix = format!("sha256={hex_sig}");
        assert!(verify_whatsapp_signature(
            &app_secret,
            body,
            &correct_prefix
        ));
    }

    #[test]
    fn whatsapp_signature_truncated_hex() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);
        let truncated = &hex_sig[..32]; // Only half the signature
        let signature_header = format!("sha256={truncated}");

        assert!(!verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    #[test]
    fn whatsapp_signature_extra_bytes() {
        let app_secret = generate_test_secret();
        let body = b"test body";

        let hex_sig = compute_whatsapp_signature_hex(&app_secret, body);
        let extended = format!("{hex_sig}deadbeef");
        let signature_header = format!("sha256={extended}");

        assert!(!verify_whatsapp_signature(
            &app_secret,
            body,
            &signature_header
        ));
    }

    // ══════════════════════════════════════════════════════════
    // IdempotencyStore Edge-Case Tests
    // ══════════════════════════════════════════════════════════

    #[test]
    fn idempotency_store_allows_different_keys() {
        let store = IdempotencyStore::new(Duration::from_secs(60), 100);
        assert!(store.record_if_new("key-a"));
        assert!(store.record_if_new("key-b"));
        assert!(store.record_if_new("key-c"));
        assert!(store.record_if_new("key-d"));
    }

    #[test]
    fn idempotency_store_max_keys_clamped_to_one() {
        let store = IdempotencyStore::new(Duration::from_secs(60), 0);
        assert!(store.record_if_new("only-key"));
        assert!(!store.record_if_new("only-key"));
    }

    #[test]
    fn idempotency_store_rapid_duplicate_rejected() {
        let store = IdempotencyStore::new(Duration::from_secs(300), 100);
        assert!(store.record_if_new("rapid"));
        assert!(!store.record_if_new("rapid"));
    }

    #[test]
    fn idempotency_store_accepts_after_ttl_expires() {
        let store = IdempotencyStore::new(Duration::from_millis(1), 100);
        assert!(store.record_if_new("ttl-key"));
        std::thread::sleep(Duration::from_millis(10));
        assert!(store.record_if_new("ttl-key"));
    }

    #[test]
    fn idempotency_store_eviction_preserves_newest() {
        let store = IdempotencyStore::new(Duration::from_secs(300), 1);
        assert!(store.record_if_new("old-key"));
        std::thread::sleep(Duration::from_millis(2));
        assert!(store.record_if_new("new-key"));

        let keys = store.keys.lock();
        assert_eq!(keys.len(), 1);
        assert!(!keys.contains_key("old-key"));
        assert!(keys.contains_key("new-key"));
    }

    #[test]
    fn rate_limiter_allows_after_window_expires() {
        let window = Duration::from_millis(50);
        let limiter = SlidingWindowRateLimiter::new(2, window, 100);
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-1"));
        assert!(!limiter.allow("ip-1")); // blocked

        // Wait for window to expire
        std::thread::sleep(Duration::from_millis(60));

        // Should be allowed again
        assert!(limiter.allow("ip-1"));
    }

    #[test]
    fn rate_limiter_independent_keys_tracked_separately() {
        let limiter = SlidingWindowRateLimiter::new(2, Duration::from_secs(60), 100);
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-1"));
        assert!(!limiter.allow("ip-1")); // ip-1 blocked

        // ip-2 should still work
        assert!(limiter.allow("ip-2"));
        assert!(limiter.allow("ip-2"));
        assert!(!limiter.allow("ip-2")); // ip-2 now blocked
    }

    #[test]
    fn rate_limiter_exact_boundary_at_max_keys() {
        let limiter = SlidingWindowRateLimiter::new(10, Duration::from_secs(60), 3);
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-2"));
        assert!(limiter.allow("ip-3"));
        // At capacity now
        assert!(limiter.allow("ip-4")); // should evict ip-1

        let guard = limiter.requests.lock();
        assert_eq!(guard.0.len(), 3);
        assert!(
            !guard.0.contains_key("ip-1"),
            "ip-1 should have been evicted"
        );
        assert!(guard.0.contains_key("ip-2"));
        assert!(guard.0.contains_key("ip-3"));
        assert!(guard.0.contains_key("ip-4"));
    }

    #[test]
    fn gateway_rate_limiter_pair_and_webhook_are_independent() {
        let limiter = GatewayRateLimiter::new(2, 3, 100);

        // Exhaust pair limit
        assert!(limiter.allow_pair("ip-1"));
        assert!(limiter.allow_pair("ip-1"));
        assert!(!limiter.allow_pair("ip-1")); // pair blocked

        // Webhook should still work
        assert!(limiter.allow_webhook("ip-1"));
        assert!(limiter.allow_webhook("ip-1"));
        assert!(limiter.allow_webhook("ip-1"));
        assert!(!limiter.allow_webhook("ip-1")); // webhook now blocked
    }

    #[test]
    fn rate_limiter_single_key_max_allows_one_request() {
        let limiter = SlidingWindowRateLimiter::new(5, Duration::from_secs(60), 1);
        assert!(limiter.allow("ip-1"));
        assert!(limiter.allow("ip-2")); // evicts ip-1

        let guard = limiter.requests.lock();
        assert_eq!(guard.0.len(), 1);
        assert!(guard.0.contains_key("ip-2"));
        assert!(!guard.0.contains_key("ip-1"));
    }

    #[test]
    fn rate_limiter_concurrent_access_safe() {
        use std::sync::Arc;

        let limiter = Arc::new(SlidingWindowRateLimiter::new(
            1000,
            Duration::from_secs(60),
            1000,
        ));
        let mut handles = Vec::new();

        for i in 0..10 {
            let limiter = limiter.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    limiter.allow(&format!("thread-{i}-req-{j}"));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Should not panic or deadlock
        let guard = limiter.requests.lock();
        assert!(guard.0.len() <= 1000, "should respect max_keys");
    }

    #[test]
    fn idempotency_store_concurrent_access_safe() {
        use std::sync::Arc;

        let store = Arc::new(IdempotencyStore::new(Duration::from_secs(300), 1000));
        let mut handles = Vec::new();

        for i in 0..10 {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    store.record_if_new(&format!("thread-{i}-key-{j}"));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let keys = store.keys.lock();
        assert!(keys.len() <= 1000, "should respect max_keys");
    }

    #[test]
    fn rate_limiter_rapid_burst_then_cooldown() {
        let limiter = SlidingWindowRateLimiter::new(5, Duration::from_millis(50), 100);

        // Burst: use all 5 requests
        for _ in 0..5 {
            assert!(limiter.allow("burst-ip"));
        }
        assert!(!limiter.allow("burst-ip")); // 6th should fail

        // Cooldown
        std::thread::sleep(Duration::from_millis(60));

        // Should be allowed again
        assert!(limiter.allow("burst-ip"));
    }

    #[test]
    fn require_localhost_accepts_ipv4_loopback() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 12345));
        assert!(require_localhost(&peer).is_ok());
    }

    #[test]
    fn require_localhost_accepts_ipv6_loopback() {
        let peer = SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, 12345));
        assert!(require_localhost(&peer).is_ok());
    }

    #[test]
    fn require_localhost_rejects_non_loopback_ipv4() {
        let peer = SocketAddr::from(([192, 168, 1, 100], 12345));
        let err = require_localhost(&peer).unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn require_localhost_rejects_non_loopback_ipv6() {
        let peer = SocketAddr::from((
            std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
            12345,
        ));
        let err = require_localhost(&peer).unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn needs_onboarding_for_flags_empty_model() {
        let err =
            needs_onboarding_for("").expect("empty model must produce a needs_onboarding error");
        let msg = err.to_string();
        assert!(
            msg.contains("needs_onboarding"),
            "error must carry the needs_onboarding marker for callers to map to 503; got: {msg}"
        );
        assert!(
            msg.contains("/onboard"),
            "error must point the user at /onboard; got: {msg}"
        );
    }

    #[test]
    fn needs_onboarding_for_flags_whitespace_only_model() {
        assert!(
            needs_onboarding_for("   ").is_some(),
            "whitespace-only model must be treated as empty"
        );
        assert!(
            needs_onboarding_for("\n\t ").is_some(),
            "tabs and newlines count as empty too"
        );
    }

    #[test]
    fn needs_onboarding_for_passes_real_model() {
        assert!(
            needs_onboarding_for("anthropic/claude-sonnet-4").is_none(),
            "a real model id must not be flagged"
        );
        assert!(
            needs_onboarding_for("  gpt-4  ").is_none(),
            "leading/trailing whitespace around a real model id must not be flagged"
        );
    }

    #[test]
    fn is_needs_onboarding_err_detects_marker_from_helper() {
        let err = needs_onboarding_for("").expect("empty model produces marker");
        assert!(
            is_needs_onboarding_err(&err),
            "the marker emitted by needs_onboarding_for must be detected"
        );
    }

    #[test]
    fn is_needs_onboarding_err_ignores_unrelated_errors() {
        let err = Error::message("upstream timeout: provider returned 504");
        assert!(
            !is_needs_onboarding_err(&err),
            "unrelated errors must not be misclassified as needs_onboarding"
        );
        let err = Error::message("invalid api key");
        assert!(!is_needs_onboarding_err(&err));
    }

    #[test]
    fn is_needs_onboarding_err_detects_only_typed_variant() {
        // Typed-variant matching replaced the substring heuristic: an
        // error whose text merely contains "needs_onboarding" (e.g. a
        // wrapped backend failure) must NOT be classified — only the
        // exact typed variant produced by needs_onboarding_for is.
        let wrapped = Error::Backend(anyhow::anyhow!("provider call failed"));
        assert!(
            !is_needs_onboarding_err(&wrapped),
            "backend-wrapped errors must not be misclassified via text"
        );
        let marker = needs_onboarding_for("").expect("empty model produces marker");
        assert!(
            is_needs_onboarding_err(&marker),
            "the typed NeedsOnboarding variant must be detected"
        );
    }

    #[test]
    fn needs_onboarding_channel_reply_resolves_via_fluent() {
        // The channel-needs-onboarding-reply must resolve to real text.
        let reply = needs_onboarding_channel_reply();
        assert!(
            !reply.starts_with('{') && !reply.ends_with('}'),
            "missing-key fallback leaked into channel reply: {reply:?}"
        );
        assert!(
            reply.to_lowercase().contains("onboarding"),
            "channel reply must mention onboarding so users know what's missing: {reply:?}"
        );
    }
}
