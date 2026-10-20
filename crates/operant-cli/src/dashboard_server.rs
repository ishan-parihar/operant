//! Dashboard HTTP server.
//! Serves the embedded web dashboard for Operant status monitoring.
//!
//! SCOPE. This is a four-route status view, not the full operant web
//! console: `/api/status`, `/api/boards`, `/api/health`, `/api/config`.
//! The page (`dashboard/index.html`) is first-party and one file, and it
//! calls ONLY those routes. It replaced a 1.9 MB bundle from a larger
//! harness that called 79 routes and opened a `/api/ws` WebSocket this
//! server never had, so every panel but four rendered blank (BUGS.md
//! RT-44). The contract is testable now: `the_page_calls_only_routes_the_
//! server_serves` fails if the page names a route the router does not
//! mount, which is the exact defect the bundle caused.
//!
//! Security: the API endpoints (except /api/health) require a session
//! token that is generated at startup and printed to the console. The
//! frontend needs the token, so the server injects it into index.html as a
//! global variable — but ONLY for a request that already carries the
//! token, so the token is never handed to a caller who does not have it.
//!
//! Two defences make that safe, and both are needed:
//!
//! 1. **The index route requires the token.** The CLI opens the browser
//!    at `/?token=...`, which the server exchanges for an HttpOnly cookie
//!    and then redirects to a clean URL. A request without the token
//!    gets a 401 and no token, so a hostile page that can reach the port
//!    learns nothing.
//! 2. **The Host header is validated** (see [`host_guard`]). The
//!    dashboard binds to loopback by default, and loopback is reachable
//!    from every web page the operator visits. Without the Host check, a
//!    page can point a name it controls at 127.0.0.1 (DNS rebinding) and
//!    the browser would send a Host header that passes any naive check
//!    on "is it localhost".
//!
//! Without both, the auth was theatre: the token was served on an
//! UNAUTHENTICATED route, so `GET /` returned it and every "protected"
//! route was reachable with a scraped token. Verified before the fix:
//! `GET /` returned the token in the body, and that token opened
//! `/api/status`, `/api/boards` and `/api/config`.
//!
//! The token is accepted from three places, and all three matter:
//! `Authorization: Bearer` (a human or a script), `X-Hermes-Session-Token`
//! (what the page sends, because a header survives even when the cookie is
//! withheld), and the dashboard cookie (what the browser sends on a
//! same-origin fetch).

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
};
use operant_core::config::AppConfig;
use operant_core::kanban::KanbanManager;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Embedded frontend HTML (compiled into binary).
const INDEX_HTML: &str = include_str!("dashboard/index.html");

/// Server state shared across all handlers.
#[derive(Clone)]
pub struct DashboardState {
    pub start_time: Instant,
    pub app_config: Arc<AppConfig>,
    pub kanban_dir: Option<std::path::PathBuf>,
    /// Session token for API auth. If None, auth is disabled (--insecure mode).
    pub session_token: Option<String>,
    /// The `Host` values this server answers to.
    ///
    /// A request whose `Host` is not on this list is refused. The dashboard
    /// binds to loopback by default, and loopback is reachable from every
    /// web page the operator visits, so without this a page can point a
    /// name it controls at 127.0.0.1 (DNS rebinding) and the browser will
    /// send a `Host` header the page chose.
    pub allowed_hosts: Vec<String>,
}

/// Build the dashboard's router.
///
/// Split out of the serve loop so the caller can BIND first and open the
/// browser with the token only once the listener is up: a browser that
/// launches before the server is listening gets a connection-refused on
/// its first request, and the operator sees a dead dashboard.
pub fn build_app(
    config: &AppConfig,
    host: &str,
    port: u16,
    session_token: Option<String>,
) -> Router {
    let kanban_dir = config
        .database_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let state = DashboardState {
        start_time: Instant::now(),
        app_config: Arc::new(config.clone()),
        kanban_dir: Some(kanban_dir),
        session_token,
        allowed_hosts: allowed_hosts(host, port),
    };

    Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/boards", get(handle_boards))
        .route("/api/health", get(handle_health))
        .route("/api/config", get(handle_config))
        .route("/", get(handle_index))
        .with_state(state.clone())
        // The middleware reads the allowed hosts from the state, and it is
        // added AFTER `with_state` — so the state is handed to it
        // explicitly rather than extracted.
        .layer(axum::middleware::from_fn_with_state(state, host_guard))
}

/// Bind the dashboard's listener.
///
/// The caller binds BEFORE opening the browser, so the browser never
/// races the server.
pub async fn bind_listener(host: &str, port: u16) -> Result<tokio::net::TcpListener> {
    let addr = format!("{host}:{port}");
    tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {addr}"))
}

/// Serve a bound dashboard listener until shutdown.
pub async fn serve(listener: tokio::net::TcpListener, app: Router) -> Result<()> {
    axum::serve(listener, app).await?;
    Ok(())
}

/// The URL the operator's browser should open.
///
/// The token rides in the query, and the server exchanges it for an
/// HttpOnly cookie on the first request. Without the token in the URL the
/// index route would refuse to serve the page at all.
pub fn dashboard_url(host: &str, port: u16, token: Option<&str>) -> String {
    let base = format!("http://{host}:{port}");
    match token {
        Some(t) => format!("{base}/?token={t}"),
        None => base,
    }
}

/// Generate a session token for the dashboard's API auth.
///
/// 32 bytes from `/dev/urandom`, hex-encoded. Public, because the caller
/// needs it to build the browser URL.
pub fn new_session_token() -> String {
    generate_session_token()
}

/// Generate a cryptographically random session token.
/// Uses /dev/urandom on Unix for unpredictability.
fn generate_session_token() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    // ponytail: stdlib /dev/urandom — cryptographically secure, no extra deps.
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    }
    // hex-encode the 32 random bytes → 64-char token.
    // ponytail: byte→hex via lookup instead of format! per byte.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for &b in &bytes {
        hex.push(HEX[(b >> 4) as usize] as char);
        hex.push(HEX[(b & 0x0f) as usize] as char);
    }
    format!("operant_{}", hex)
}

/// Whether the request's `Host` header is one this server answers to.
///
/// The rules, in order:
///
/// 1. An ABSENT `Host` is refused. HTTP/1.1 requires it, every browser
///    sends it, and a request that omits it is not one a browser made —
///    which is exactly the shape a hostile request wants.
/// 2. A host that is an IP LITERAL is allowed. A server bound to
///    `0.0.0.0` is reachable on every interface, and the operator reaches
///    it by the address their machine has on the LAN — which is not the
///    bind address, and is not in the allowlist. Refusing it would 403
///    the operator's own browser. (A DNS-rebinding attack uses a NAME it
///    controls, not an IP literal, so this does not open the hole.)
/// 3. Otherwise the host must be on the allowlist. The comparison is
///    case-insensitive, because a host name is.
pub fn host_is_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    let Some(host) = headers.get(header::HOST) else {
        return false;
    };
    let Ok(host) = host.to_str() else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    // Strip the port: "example.com:3000" -> "example.com", and an IPv6
    // literal keeps its brackets ("[::1]:3000" -> "[::1]").
    let bare = match host.rsplit_once(':') {
        Some((before, after)) if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) => {
            before
        }
        _ => host.as_str(),
    };
    if is_ip_literal(bare) {
        return true;
    }
    allowed.iter().any(|a| {
        let a = a.trim().to_ascii_lowercase();
        // The allowlist holds both "host" and "host:port" spellings, so a
        // bare host matches the portless entry and a host:port matches
        // either.
        a == host || a == bare
    })
}

/// Whether a host is an IP literal rather than a name.
///
/// A name is what a DNS-rebinding attack controls, so the two are treated
/// differently. This is a lexical check, not a resolver: it asks "is this
/// an address or a name", and it does not care whether the address is
/// reachable.
fn is_ip_literal(host: &str) -> bool {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if unbracketed.contains(':') {
        // IPv6: hex digits, colons, and a dotted-quad tail.
        return unbracketed
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
    }
    // IPv4: four dotted decimal octets.
    let parts: Vec<&str> = unbracketed.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u8>().is_ok()
        })
}

/// The `Host` values this server answers to.
///
/// The address the server was told to bind to, plus the loopback spellings
/// of it. A server bound to `0.0.0.0` is reachable on every interface, and
/// the operator reaches it by the name they typed, so the bound address
/// alone would refuse their own browser.
fn allowed_hosts(host: &str, port: u16) -> Vec<String> {
    let mut hosts = vec![format!("{host}:{port}"), host.to_string()];
    // The loopback spellings of the same port. A browser that resolved
    // `localhost` to `::1` sends `Host: localhost:PORT`, not the address
    // the server was told to bind to.
    for loopback in ["127.0.0.1", "localhost", "[::1]", "::1"] {
        hosts.push(format!("{loopback}:{port}"));
        hosts.push(loopback.to_string());
    }
    hosts
}

/// Refuse a request whose `Host` header is not one this server answers to.
///
/// The dashboard binds to loopback by default, and loopback is reachable
/// from every web page the operator visits. Without this check, a page can
/// point a name it controls at 127.0.0.1 (DNS rebinding) and the browser
/// sends a `Host` header the page chose — which is the one header that
/// tells a loopback service "this request came from a stranger".
async fn host_guard(
    axum::extract::State(state): axum::extract::State<DashboardState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if !host_is_allowed(request.headers(), &state.allowed_hosts) {
        return (
            StatusCode::FORBIDDEN,
            "the Host header does not match this server's address",
        )
            .into_response();
    }
    next.run(request).await
}

/// Check the Authorization header against the session token.
/// Returns Ok(()) if authorized, Err(response) if not.
///
/// The error is boxed: `axum::http::Response<Body>` is ~128+ bytes, which
/// trips clippy::result_large_err on the hot path. Boxing the error variant
/// keeps the Result small (callers deref with `return *e;`).
fn check_auth(state: &DashboardState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let token = match &state.session_token {
        None => return Ok(()), // Insecure mode — no auth required
        Some(t) => t,
    };

    // The token may arrive in any of three places, and ALL THREE matter:
    //
    // - `X-Hermes-Session-Token`: what the frontend bundle ACTUALLY
    //   sends. Its fetch wrapper sets this header on every request. The
    //   server used to read only `Authorization: Bearer`, so every API
    //   call the real dashboard made returned 401 — the UI was
    //   non-functional. (The bundle never sends `Authorization` at all:
    //   the string does not appear in it.)
    // - `Authorization: Bearer`: what a human or a script uses.
    // - The `operant_dashboard_token` cookie: what the browser sends on
    //   a same-origin fetch. This is what lets the UI work WITHOUT the
    //   token being readable by JavaScript.
    let presented = header_token(headers, header::AUTHORIZATION.as_str())
        .or_else(|| header_token(headers, "x-hermes-session-token"))
        .or_else(|| cookie_token(headers))
        .is_some_and(|t| t == *token);

    if presented {
        Ok(())
    } else {
        Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unauthenticated" })),
            )
                .into_response(),
        ))
    }
}

/// The token in a header, with an optional `Bearer ` prefix stripped.
fn header_token(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name).and_then(|v| v.to_str().ok())?;
    let value = value.trim();
    let value = value.strip_prefix("Bearer ").unwrap_or(value);
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The token in the dashboard's cookie.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
    cookies.split(';').find_map(|c| {
        let (name, value) = c.trim().split_once('=')?;
        (name == TOKEN_COOKIE).then(|| value.to_string())
    })
}

// ── API Handlers ──

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    version: String,
    uptime_seconds: u64,
    gateway_running: bool,
    agent_model: String,
    kanban_tasks: usize,
    database_path: String,
    /// Nested, not flattened: the page reads `host.cpu_count`, and the old
    /// flat shape (served for a bundle that is gone) made the panel render
    /// "n/a" for every metric while the API returned real values.
    host: HostMetrics,
}

/// Host metrics for the page's host panel (RT-117).
///
/// `memory.used` / `disk.used` are **bytes** (the page formats them with its
/// own B/KB/MB/GB helper), `load_avg` is the 1/5/15 **array**, and every
/// field is `Option` because a read can fail on a platform without
/// `/proc` — `null` renders as "unavailable", never as a zero.
#[derive(Serialize)]
struct HostMetrics {
    cpu_count: Option<usize>,
    cpu_percent: Option<f64>,
    load_avg: Option<[f64; 3]>,
    memory: Option<MemoryStats>,
    disk: Option<DiskStats>,
    hostname: Option<String>,
    os_release: Option<String>,
    arch: &'static str,
}

#[derive(Debug, Serialize)]
struct MemoryStats {
    used: u64,
    total: u64,
    percent: f64,
}

#[derive(Debug, Serialize)]
struct DiskStats {
    used: u64,
    total: u64,
    percent: f64,
}

impl HostMetrics {
    fn collect() -> Self {
        let memory = read_memory();
        Self {
            cpu_count: std::thread::available_parallelism().ok().map(|n| n.get()),
            cpu_percent: read_cpu_percent(),
            load_avg: read_load_avg(),
            memory,
            disk: read_disk(),
            hostname: read_hostname(),
            os_release: read_os_release(),
            arch: std::env::consts::ARCH,
        }
    }
}

/// A percentage rounded to one decimal — the panel prints the value raw.
fn percent_of(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (used as f64 / total as f64 * 1000.0).round() / 10.0
}

/// CPU counters from the PREVIOUS status poll, so `cpu_percent` is the
/// utilisation since that poll. An average since boot (the total jiffies)
/// barely moves on a long-running host.
static LAST_CPU_SAMPLE: std::sync::Mutex<Option<(u64, u64)>> = std::sync::Mutex::new(None);

#[cfg(target_os = "linux")]
fn read_cpu_percent() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let values: Vec<u64> = stat
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    let total: u64 = values.iter().sum();
    let mut last = LAST_CPU_SAMPLE.lock().ok()?;
    let percent = last.and_then(|(prev_total, prev_idle)| {
        let total_delta = total.saturating_sub(prev_total);
        let idle_delta = idle.saturating_sub(prev_idle);
        (total_delta > 0).then(|| {
            ((total_delta - idle_delta) as f64 / total_delta as f64 * 1000.0).round() / 10.0
        })
    });
    *last = Some((total, idle));
    percent
}

#[cfg(not(target_os = "linux"))]
fn read_cpu_percent() -> Option<f64> {
    // No /proc: the first poll would have no previous sample to subtract, and
    // this server has no background sampler. The panel shows nothing rather
    // than a made-up number.
    None
}

#[cfg(unix)]
fn read_load_avg() -> Option<[f64; 3]> {
    let mut avg = [0f64; 3];
    // SAFETY: getloadavg writes at most the count passed (3) into the array.
    let written = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    (written == 3).then_some(avg)
}

#[cfg(not(unix))]
fn read_load_avg() -> Option<[f64; 3]> {
    None
}

#[cfg(target_os = "linux")]
fn read_memory() -> Option<MemoryStats> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |key: &str| -> Option<u64> {
        text.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u64>().ok())
    };
    let total = field("MemTotal:")? * 1024;
    let available = field("MemAvailable:")? * 1024;
    let used = total.saturating_sub(available);
    Some(MemoryStats {
        used,
        total,
        percent: percent_of(used, total),
    })
}

#[cfg(not(target_os = "linux"))]
fn read_memory() -> Option<MemoryStats> {
    None
}

#[cfg(unix)]
fn read_disk() -> Option<DiskStats> {
    let path = std::ffi::CString::new("/").ok()?;
    // SAFETY: statvfs only writes into the struct we hand it, which is fully
    // zeroed first and freed by its own Drop.
    let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid NUL-terminated C string for the call's life.
    if unsafe { libc::statvfs(path.as_ptr(), &mut vfs) } != 0 {
        return None;
    }
    let unit = if vfs.f_frsize > 0 {
        vfs.f_frsize as u64
    } else {
        vfs.f_bsize as u64
    };
    let total = (vfs.f_blocks as u64).saturating_mul(unit);
    let free = (vfs.f_bfree as u64).saturating_mul(unit);
    let used = total.saturating_sub(free);
    Some(DiskStats {
        used,
        total,
        percent: percent_of(used, total),
    })
}

#[cfg(not(unix))]
fn read_disk() -> Option<DiskStats> {
    None
}

#[cfg(unix)]
fn read_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is 256 bytes and its length is passed alongside.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

#[cfg(not(unix))]
fn read_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

fn read_os_release() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim_matches('"').to_string())
}

async fn handle_status(State(state): State<DashboardState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return *e;
    }

    let kanban_task_count = count_all_kanban_tasks(&state.kanban_dir).unwrap_or(0);

    let version = env!("CARGO_PKG_VERSION").to_string();

    Json(StatusResponse {
        status: "running".to_string(),
        version,
        uptime_seconds: state.start_time.elapsed().as_secs(),
        gateway_running: crate::gateway_runner::is_running().await,
        agent_model: state.app_config.agent.model.clone(),
        kanban_tasks: kanban_task_count,
        database_path: state.app_config.database_path.display().to_string(),
        host: HostMetrics::collect(),
    })
    .into_response()
}

#[derive(Serialize)]
struct BoardResponse {
    slug: String,
    name: String,
    task_counts: TaskCounts,
    total: usize,
}

#[derive(Serialize, Default)]
struct TaskCounts {
    triage: usize,
    todo: usize,
    ready: usize,
    running: usize,
    blocked: usize,
    done: usize,
    archived: usize,
}

async fn handle_boards(State(state): State<DashboardState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return *e;
    }

    let kanban_dir = match &state.kanban_dir {
        Some(d) => d.clone(),
        None => return Json(Vec::<BoardResponse>::new()).into_response(),
    };

    if !kanban_dir.exists() {
        return Json(Vec::<BoardResponse>::new()).into_response();
    }

    let mgr = KanbanManager::new(kanban_dir);
    let boards = match mgr.list_boards() {
        Ok(b) => b,
        Err(_) => return Json(Vec::<BoardResponse>::new()).into_response(),
    };

    let mut result: Vec<BoardResponse> = Vec::new();
    for b in &boards {
        let counts = match build_board_counts(&mgr, &b.slug) {
            Ok(c) => c,
            Err(_) => continue,
        };
        result.push(BoardResponse {
            slug: b.slug.clone(),
            name: format!("{} Board", b.slug),
            task_counts: counts,
            total: b.task_count,
        });
    }

    Json(result).into_response()
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    uptime_seconds: u64,
    database_ok: bool,
    kanban_ok: bool,
    timestamp: i64,
}

async fn handle_health(State(state): State<DashboardState>) -> Json<HealthResponse> {
    // Health endpoint does NOT require auth — needed for health checks.
    let kanban_ok = state
        .kanban_dir
        .as_ref()
        .map(|d| KanbanManager::new(d.clone()).open_board("default").is_ok())
        .unwrap_or(false);

    Json(HealthResponse {
        status: "ok".to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        database_ok: true,
        kanban_ok,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

#[derive(Serialize)]
struct ConfigResponse {
    model: String,
    base_url: String,
    platforms_enabled: Vec<String>,
    skills_dir: String,
    database_path: String,
    gateway_running: bool,
}

async fn handle_config(State(state): State<DashboardState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return *e;
    }

    let cfg = &state.app_config;
    let mut platforms = Vec::new();
    if cfg.gateway.telegram_enabled {
        platforms.push("telegram".into());
    }
    if cfg.gateway.discord_enabled {
        platforms.push("discord".into());
    }
    if cfg.gateway.slack_enabled {
        platforms.push("slack".into());
    }
    if cfg.gateway.webhooks_enabled {
        platforms.push("webhooks".into());
    }

    Json(ConfigResponse {
        model: cfg.agent.model.clone(),
        base_url: cfg.client.base_url.clone(),
        platforms_enabled: platforms,
        skills_dir: cfg.skills.root_dir.display().to_string(),
        database_path: cfg.database_path.display().to_string(),
        gateway_running: crate::gateway_runner::is_running().await,
    })
    .into_response()
}

/// The query the CLI opens the browser with.
#[derive(Debug, Default, serde::Deserialize)]
struct IndexQuery {
    /// The session token, presented by the CLI's own browser launch.
    token: Option<String>,
}

/// The cookie the dashboard hands the operator's browser.
const TOKEN_COOKIE: &str = "operant_dashboard_token";

/// Serve the dashboard page.
///
/// The session token is injected into the HTML so the frontend can call the
/// API — but ONLY for a request that already holds the token, so the token
/// is never handed to a caller who does not have it.
///
/// The exchange:
///
/// 1. The CLI opens the browser at `/?token=...`.
/// 2. That request presents the token in the query. The server sets an
///    **HttpOnly** cookie carrying it and redirects to `/`, so the token
///    does not linger in the address bar or in the browser's history.
/// 3. `GET /` presents the cookie, and the server serves the page with the
///    token injected.
/// 4. A request with neither gets a 401 and the page WITHOUT the token:
///    the page renders, but it cannot reach a single API route, and the
///    caller learns nothing.
///
/// The cookie is HttpOnly (JavaScript cannot read it) and SameSite=Strict
/// (a cross-site request never carries it). The Host header is validated
/// by the [`host_guard`] middleware, which is what stops a hostile page
/// from reaching the dashboard at all.
async fn handle_index(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    Query(query): Query<IndexQuery>,
) -> Response {
    // Insecure mode: no token exists, so there is nothing to gate.
    let Some(token) = state.session_token.as_deref() else {
        return Html(INDEX_HTML.to_string()).into_response();
    };

    // The token in the query: the CLI's own browser launch. Exchange it for
    // the cookie and redirect to the clean URL.
    //
    // The REDIRECT carries the cookie, because the cookie is set on THIS
    // response and the browser sends it on the follow-up request. A
    // redirect that dropped the cookie would land on a page with no token
    // and the dashboard would boot empty.
    if query.token.as_deref().is_some_and(|t| t == token) {
        let cookie = format!("{TOKEN_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/");
        return (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, "/".to_string()),
                (header::SET_COOKIE, cookie),
            ],
            Html(String::new()),
        )
            .into_response();
    }

    // The token in the cookie: the operator's own browser, on a reload.
    let from_cookie = cookie_token(&headers).is_some_and(|t| t == token);

    if !from_cookie {
        // 401 with the page but WITHOUT the token: the operator sees why,
        // and a hostile caller learns nothing. The body carries
        // `error: unauthenticated`, which is what the frontend's 401 path
        // looks for. There is deliberately NO `login_url`: the frontend
        // would assign() to it, and the only URL that carries the token is
        // the one the CLI prints — pointing at it from a 401 body would
        // hand the token to every caller that can reach the port.
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthenticated" })),
        )
            .into_response();
    }

    // The holder of the cookie gets the page with the token injected.
    let html = INDEX_HTML.replace(
        "<div id=\"root\"></div>",
        &format!(
            "<script>window.__HERMES_SESSION_TOKEN__=\"{}\";</script>\n<div id=\"root\"></div>",
            token
        ),
    );
    Html(html).into_response()
}

// ── Helpers ──

fn count_all_kanban_tasks(kanban_dir: &Option<std::path::PathBuf>) -> Result<usize> {
    let dir = match kanban_dir {
        Some(d) => d,
        None => return Ok(0),
    };
    if !dir.exists() {
        return Ok(0);
    }
    let mgr = KanbanManager::new(dir.clone());
    let boards = mgr.list_boards()?;
    let total: usize = boards.iter().map(|b| b.task_count).sum();
    Ok(total)
}

fn build_board_counts(mgr: &KanbanManager, slug: &str) -> Result<TaskCounts> {
    let db = mgr.open_board(slug)?;
    let tasks = db.list_tasks()?;

    let mut counts = TaskCounts::default();
    for task in &tasks {
        match task.status.as_str() {
            "triage" => counts.triage += 1,
            "todo" => counts.todo += 1,
            "ready" => counts.ready += 1,
            "running" => counts.running += 1,
            "blocked" => counts.blocked += 1,
            "done" => counts.done += 1,
            "archived" => counts.archived += 1,
            _ => {}
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    /// The page the server serves — the frontend contract is read from
    /// this, not from a doc comment.
    const PAGE: &str = include_str!("dashboard/index.html");

    /// A status payload with every field filled, for shape assertions.
    fn status_sample() -> StatusResponse {
        StatusResponse {
            status: "running".to_string(),
            version: "0.0.0-test".to_string(),
            uptime_seconds: 42,
            gateway_running: false,
            agent_model: "test-model".to_string(),
            kanban_tasks: 0,
            database_path: "/tmp/database.db".to_string(),
            host: HostMetrics {
                cpu_count: Some(8),
                cpu_percent: Some(12.5),
                load_avg: Some([0.5, 0.4, 0.3]),
                memory: Some(MemoryStats {
                    used: 4 * 1024 * 1024 * 1024,
                    total: 16 * 1024 * 1024 * 1024,
                    percent: 25.0,
                }),
                disk: Some(DiskStats {
                    used: 250 * 1024 * 1024 * 1024,
                    total: 1000 * 1024 * 1024 * 1024,
                    percent: 25.0,
                }),
                hostname: Some("host".to_string()),
                os_release: Some("Test OS".to_string()),
                arch: "x86_64",
            },
        }
    }

    /// RT-117: the status payload and the system panel must agree on the
    /// field names, in BOTH directions — it reads the page's own source and
    /// the server's own serializer, so dropping a field on EITHER side
    /// fails here instead of rendering a blank panel in production.
    #[test]
    fn the_status_payload_carries_every_field_the_system_panel_reads() {
        let payload = serde_json::to_value(status_sample()).expect("the status is serializable");

        // Each entry is (json key on the payload, the JS expression the page
        // uses to read it). Encoding the ACCESS PATH is what catches the
        // shape defect this test missed once: the payload carried the host
        // fields at top level (serde flatten) while the page read
        // `host.cpu_count`, so every host panel rendered "n/a" with a green
        // API. Name-only checks pass through that.
        for (field, access) in [
            ("version", "s.version"),
            ("uptime_seconds", "s.uptime_seconds"),
            ("gateway_running", "s.gateway_running"),
            ("agent_model", "s.agent_model"),
            ("kanban_tasks", "s.kanban_tasks"),
            ("database_path", "s.database_path"),
            ("cpu_count", "host.cpu_count"),
            ("cpu_percent", "host.cpu_percent"),
            ("load_avg", "host.load_avg"),
            ("memory", "host.memory"),
            ("disk", "host.disk"),
            ("hostname", "host.hostname"),
            ("os_release", "host.os_release"),
            ("arch", "host.arch"),
        ] {
            assert!(
                PAGE.contains(access),
                "the page no longer reads `{access}` — the panel changed shape, \
                 so this list (and the response) must be re-derived from it"
            );
            assert!(
                PAGE.contains(field),
                "the page no longer reads `{field}` — the panel changed shape, \
                 so this list (and the response) must be re-derived from it"
            );
            let value = if access.starts_with("host.") {
                payload["host"].get(field)
            } else {
                payload.get(field)
            };
            assert!(
                value.is_some(),
                "the system panel reads `{field}` and /api/status does not serve it: {payload}"
            );
        }

        // The page formats these three numbers as BYTES; a value in MB or
        // KB would render as "0.0 KB" and look like a broken host.
        let memory = &payload["host"]["memory"];
        assert!(
            memory["total"].as_u64().unwrap_or(0) > 1_000_000,
            "memory.total is bytes in the page's formatter, got {memory}"
        );
        assert_eq!(
            payload["host"]["load_avg"].as_array().map(|a| a.len()),
            Some(3),
            "the page requires the 1/5/15 array, not a scalar"
        );
    }

    /// On Linux the metrics are read for real, not stubbed to `None`: a
    /// panel that says "unavailable" on a host that has the numbers is a
    /// defect in its own right.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_host_metrics_are_real_on_linux() {
        let host = HostMetrics::collect();

        let memory = host.memory.expect("/proc/meminfo is readable");
        assert!(memory.total > 0, "memory.total must be real bytes");
        assert!(memory.used <= memory.total, "{memory:?}");
        assert!((0.0..=100.0).contains(&memory.percent), "{memory:?}");

        let disk = host.disk.expect("statvfs on / succeeds");
        assert!(disk.total > 0, "disk.total must be real bytes");
        assert!(disk.used <= disk.total, "{disk:?}");

        assert!(host.cpu_count.unwrap_or(0) > 0);
        assert!(
            host.hostname.as_deref().is_some_and(|h| !h.is_empty()),
            "the panel prints the hostname"
        );
    }

    /// A running dashboard on an ephemeral port.
    struct TestServer {
        port: u16,
    }

    /// Start the REAL server on an ephemeral port and return its address.
    ///
    /// The database path points at a TEMP file. `AppConfig::default()`
    /// points at the real `~/.operant/database.db`, which the running
    /// gateway holds locked: the handlers that read it then BLOCK on the
    /// lock, and the test hangs. That is not a test artifact — it is the
    /// dashboard's own behaviour on a locked database, and it is worth
    /// knowing.
    async fn start(token: &str) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the bound address").port();
        let mut config = AppConfig::default();
        let dir = std::env::temp_dir().join(format!(
            "operant_dashboard_test_{}_{}",
            std::process::id(),
            port
        ));
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        config.database_path = dir.join("database.db");
        let app = build_app(&config, "127.0.0.1", port, Some(token.to_string()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        TestServer { port }
    }

    /// RT-44: the page may only call routes this server mounts. The bundle
    /// it replaced called 79 `/api/...` paths against a four-route server,
    /// so every panel but four 404'd — a defect no unit test caught because
    /// nothing compared the two sides. This compares them: every path the
    /// page fetches is requested from the REAL server and must not 404.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_calls_only_routes_the_server_serves() {
        let mut paths: Vec<&str> = Vec::new();
        let mut rest = PAGE;
        while let Some(at) = rest.find("getJSON(\"") {
            rest = &rest[at + "getJSON(\"".len()..];
            let end = rest.find('"').expect("the path literal is closed");
            paths.push(&rest[..end]);
        }
        assert!(
            !paths.is_empty(),
            "the page must fetch something; parsing found no getJSON paths"
        );
        // A path in the HTML but not in this list is the bundle defect
        // returning: assert the page's set is exactly the four routes.
        for path in &paths {
            assert!(
                ["/api/status", "/api/config", "/api/boards", "/api/health"].contains(path),
                "the page fetches `{path}`, which this list does not name — if the \
                 server now mounts it, add it here; if not, remove the fetch"
            );
        }
        assert!(
            !PAGE.contains("new WebSocket") && !PAGE.contains("/api/ws"),
            "the page must not open a WebSocket: this server has none (RT-44)"
        );

        let server = start("secret-token-value").await;
        for path in &paths {
            let (status, _, _) = get(
                server.port,
                path,
                &[("X-Hermes-Session-Token", "secret-token-value")],
            );
            assert!(
                !status.contains("404"),
                "the page calls `{path}` and the server answers 404: {status}"
            );
        }
    }

    /// A minimal HTTP/1.1 GET, returning (status line, headers, body).
    fn get(
        port: u16,
        path: &str,
        headers: &[(&str, &str)],
    ) -> (String, Vec<(String, String)>, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let mut request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .expect("write the request");
        stream.flush().expect("flush");

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).expect("read the status line");
        let mut headers_out = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read a header");
            let trimmed = line.trim_end().to_string();
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                headers_out.push((name.trim().to_lowercase(), value.trim().to_string()));
            }
        }
        let mut body = String::new();
        reader.read_to_string(&mut body).expect("read the body");
        (status, headers_out, body)
    }

    /// The index route must NOT hand the token to a caller that does not
    /// have it.
    ///
    /// This is the defect: the token was injected into the HTML on an
    /// UNAUTHENTICATED route, so `GET /` returned it and every
    /// "protected" route was reachable with a scraped token. The auth was
    /// theatre.
    // The multi-thread flavor is required, not stylistic: these tests do
    // BLOCKING socket I/O on the test's own thread, and the default
    // current-thread runtime would then have no thread left to run the
    // server on — a deadlock, not a slow test.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_index_route_does_not_hand_the_token_to_an_unauthenticated_caller() {
        let server = start("secret-token-value").await;

        let (status, _, body) = get(server.port, "/", &[]);
        assert!(
            status.contains("401"),
            "an unauthenticated GET / must be refused, got: {status}"
        );
        assert!(
            !body.contains("secret-token-value"),
            "the token must NOT appear in the body of an unauthenticated request"
        );
    }

    /// The token in the query is exchanged for an HttpOnly cookie.
    ///
    /// The cookie is what lets the operator's own browser reach the page
    /// without the token lingering in the address bar, and HttpOnly is
    /// what stops a cross-site script from reading it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_token_in_the_query_becomes_an_httponly_cookie() {
        let server = start("secret-token-value").await;

        let (status, headers, body) = get(server.port, "/?token=secret-token-value", &[]);
        assert!(
            status.contains("303"),
            "the token exchange redirects, got: {status}"
        );
        let cookie = headers
            .iter()
            .find(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.clone());
        let cookie = cookie.expect("the exchange must set a cookie");
        assert!(
            cookie.contains("HttpOnly"),
            "the cookie must be HttpOnly, got: {cookie}"
        );
        assert!(
            cookie.contains("SameSite=Strict"),
            "the cookie must be SameSite=Strict, got: {cookie}"
        );
        assert!(
            !body.contains("secret-token-value"),
            "the redirect body must not carry the token"
        );
    }

    /// The holder of the cookie gets the page with the token injected —
    /// the frontend reads it from the page.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_cookie_holder_gets_the_page_with_the_token() {
        let server = start("secret-token-value").await;

        let (_, headers, _) = get(server.port, "/?token=secret-token-value", &[]);
        let cookie = headers
            .iter()
            .find(|(name, _)| name == "set-cookie")
            .map(|(_, value)| value.clone())
            .expect("the exchange must set a cookie");
        let cookie_pair = cookie.split(';').next().unwrap_or("").to_string();

        let (status, _, body) = get(server.port, "/", &[("Cookie", &cookie_pair)]);
        assert!(status.contains("200"), "the cookie holder gets the page");
        assert!(
            body.contains("secret-token-value"),
            "the page must carry the token for the frontend"
        );
    }

    /// A request with a FOREIGN Host is refused.
    ///
    /// The dashboard binds to loopback, and loopback is reachable from
    /// every web page the operator visits. Without the Host check, a page
    /// can point a name it controls at 127.0.0.1 (DNS rebinding) and the
    /// browser sends a Host the page chose.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_with_a_foreign_host_is_refused() {
        let server = start("secret-token-value").await;

        // A Host the page controls, pointing at loopback via DNS rebinding.
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        let request = "GET /?token=secret-token-value HTTP/1.1\r\nHost: attacker.example\r\nConnection: close\r\n\r\n";
        stream
            .write_all(request.as_bytes())
            .expect("write the request");
        stream.flush().expect("flush");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read the response");
        assert!(
            response.contains("403"),
            "a foreign Host must be refused, got: {}",
            response.lines().next().unwrap_or("")
        );
        assert!(
            !response.contains("secret-token-value"),
            "a refused request must not carry the token"
        );
    }

    /// The Host check accepts the addresses the operator actually uses.
    #[test]
    fn the_host_check_accepts_the_operators_own_addresses() {
        let allowed = allowed_hosts("127.0.0.1", 3000);
        for host in [
            "127.0.0.1:3000",
            "localhost:3000",
            "127.0.0.1",
            "localhost",
            "[::1]:3000",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().expect("a valid header value"));
            assert!(
                host_is_allowed(&headers, &allowed),
                "the operator's own address {host} must be allowed"
            );
        }
        // And refuses one the page chose.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            "attacker.example".parse().expect("a valid header value"),
        );
        assert!(
            !host_is_allowed(&headers, &allowed),
            "a foreign Host must be refused"
        );
    }

    /// A server bound to `0.0.0.0` is reachable on every interface, and the
    /// operator's browser reaches it by the address their machine has on
    /// the LAN. That address is neither the bind address nor on the
    /// allowlist, so an IP literal has to be allowed or the operator gets
    /// a 403 on their own dashboard.
    #[test]
    fn an_ip_literal_host_is_allowed_even_when_it_is_not_the_bind_address() {
        let allowed = allowed_hosts("0.0.0.0", 3000);
        for host in [
            "192.168.1.5:3000",
            "10.0.0.7",
            "203.0.113.9:3000",
            "[2001:db8::1]:3000",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().expect("a valid header value"));
            assert!(
                host_is_allowed(&headers, &allowed),
                "the operator's own LAN address {host} must be allowed"
            );
        }
        // A NAME the page chose is still refused — that is the DNS
        // rebinding vector, and it is a name, not an address.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            "attacker.example:3000"
                .parse()
                .expect("a valid header value"),
        );
        assert!(
            !host_is_allowed(&headers, &allowed),
            "a foreign NAME must be refused even on a wildcard bind"
        );
    }

    /// A request with NO Host header is refused.
    ///
    /// HTTP/1.1 requires the header and every browser sends it, so a
    /// request without one is not one a browser made.
    #[test]
    fn a_request_without_a_host_header_is_refused() {
        let allowed = allowed_hosts("127.0.0.1", 3000);
        let headers = HeaderMap::new();
        assert!(
            !host_is_allowed(&headers, &allowed),
            "a request with no Host header must be refused"
        );
    }

    /// The API must accept the header the page ACTUALLY sends.
    ///
    /// The page's fetch wrapper sets `X-Hermes-Session-Token` on every
    /// request; the string `Authorization` does not appear in it at all.
    /// The server used to read only `Authorization: Bearer`, so every API
    /// call the real dashboard made returned 401 — the UI was
    /// non-functional, and the "the dashboard works" claim was wrong.
    ///
    /// This test reads the header name out of the PAGE, so a change to
    /// the page's contract is caught here.
    ///
    /// `check_auth` is exercised directly rather than through the server:
    /// the 200-path handlers read the SQLite database SYNCHRONOUSLY, and
    /// driving them from a test that also does blocking socket I/O
    /// exhausts the runtime's worker pool and deadlocks. The auth check
    /// is a pure function over the headers, and that is what is under
    /// test.
    #[test]
    fn the_api_accepts_the_header_the_frontend_bundle_sends() {
        // The page's own header name, read from the page itself.
        let bundle = PAGE;
        assert!(
            bundle.contains("X-Hermes-Session-Token"),
            "the page's auth header name changed — update the server to match"
        );
        assert!(
            !bundle.contains("Authorization"),
            "the page does not send an Authorization header; if it now does, \
             the server must still accept it"
        );

        let state = DashboardState {
            start_time: Instant::now(),
            app_config: Arc::new(AppConfig::default()),
            kanban_dir: None,
            session_token: Some("secret-token-value".to_string()),
            allowed_hosts: allowed_hosts("127.0.0.1", 3000),
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hermes-session-token",
            "secret-token-value".parse().expect("a valid header value"),
        );
        assert!(
            check_auth(&state, &headers).is_ok(),
            "the header the bundle sends must open the API"
        );
    }

    /// The API must accept the cookie the browser sends on a same-origin
    /// fetch — that is what lets the UI work without the token being
    /// readable by JavaScript.
    #[test]
    fn the_api_accepts_the_dashboard_cookie() {
        let state = DashboardState {
            start_time: Instant::now(),
            app_config: Arc::new(AppConfig::default()),
            kanban_dir: None,
            session_token: Some("secret-token-value".to_string()),
            allowed_hosts: allowed_hosts("127.0.0.1", 3000),
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "operant_dashboard_token=secret-token-value"
                .parse()
                .expect("a valid header value"),
        );
        assert!(
            check_auth(&state, &headers).is_ok(),
            "the cookie must open the API"
        );
    }

    /// A caller with NO token is refused, whatever it presents.
    #[test]
    fn the_api_refuses_a_caller_with_no_token() {
        let state = DashboardState {
            start_time: Instant::now(),
            app_config: Arc::new(AppConfig::default()),
            kanban_dir: None,
            session_token: Some("secret-token-value".to_string()),
            allowed_hosts: allowed_hosts("127.0.0.1", 3000),
        };
        let headers = HeaderMap::new();
        assert!(
            check_auth(&state, &headers).is_err(),
            "a caller with no token must be refused"
        );
    }

    /// The API routes refuse an unauthenticated caller.
    ///
    /// Only the REFUSAL is driven through the real server: the 200-path
    /// handlers read the SQLite database synchronously, and a test that
    /// does blocking socket I/O on the same runtime exhausts the worker
    /// pool and deadlocks. The accepting side of the auth check is
    /// covered by `the_api_accepts_the_header_the_frontend_bundle_sends`
    /// and `the_api_accepts_the_dashboard_cookie`, which exercise
    /// `check_auth` directly.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_api_routes_refuse_an_unauthenticated_caller() {
        let server = start("secret-token-value").await;

        let (status, _, _) = get(server.port, "/api/status", &[]);
        assert!(
            status.contains("401"),
            "the API must refuse an unauthenticated caller, got: {status}"
        );
    }
}
