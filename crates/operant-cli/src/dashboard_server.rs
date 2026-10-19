//! Dashboard HTTP server.
//! Serves the embedded web dashboard for Operant status monitoring.
//!
//! Security: API endpoints (except /api/health) require a bearer token
//! that is generated at startup and printed to the console. The frontend
//! needs the token, so the server injects it into index.html as a global
//! variable — but ONLY for a request that already carries the token, so
//! the token is never handed to a caller who does not have it.
//!
//! Two defences make that safe, and both are needed:
//!
//! 1. **The index route requires the token.** The CLI opens the browser
//!    at `/?token=...`, which the server exchanges for the token-bearing
//!    page and then redirects to a clean URL. A request without the token
//!    gets a 401 and the page WITHOUT the token, so a hostile page that
//!    can reach the port learns nothing.
//! 2. **The Host header is validated** (see [`host_is_allowed`]). The
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

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{Path, Query, State},
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

/// Embedded static assets (JS, CSS, fonts, images).
/// These are served at `/assets/<filename>` so the index.html script/link tags resolve.
static ASSETS: &[(&str, &[u8], &str)] = &[
    (
        "index-BB4BRelo.js",
        include_bytes!("dashboard/assets/index-BB4BRelo.js"),
        "text/javascript",
    ),
    (
        "index-DJxmcHRv.css",
        include_bytes!("dashboard/assets/index-DJxmcHRv.css"),
        "text/css",
    ),
    (
        "Collapse-Bold-mgICk9-_.woff2",
        include_bytes!("dashboard/assets/Collapse-Bold-mgICk9-_.woff2"),
        "font/woff2",
    ),
    (
        "Collapse-Regular-DysayoTY.woff2",
        include_bytes!("dashboard/assets/Collapse-Regular-DysayoTY.woff2"),
        "font/woff2",
    ),
    (
        "Mondwest-Regular-CWscgue7.woff2",
        include_bytes!("dashboard/assets/Mondwest-Regular-CWscgue7.woff2"),
        "font/woff2",
    ),
    (
        "RulesCompressed-Medium-CA76_CrB.woff2",
        include_bytes!("dashboard/assets/RulesCompressed-Medium-CA76_CrB.woff2"),
        "font/woff2",
    ),
    (
        "RulesCompressed-Regular-BSXFyF4x.woff2",
        include_bytes!("dashboard/assets/RulesCompressed-Regular-BSXFyF4x.woff2"),
        "font/woff2",
    ),
    (
        "RulesExpanded-Bold-DZA7s8Pa.woff2",
        include_bytes!("dashboard/assets/RulesExpanded-Bold-DZA7s8Pa.woff2"),
        "font/woff2",
    ),
    (
        "RulesExpanded-Regular-l8uVympt.woff2",
        include_bytes!("dashboard/assets/RulesExpanded-Regular-l8uVympt.woff2"),
        "font/woff2",
    ),
    (
        "filler-bg0-DxMaWJpb.webp",
        include_bytes!("dashboard/assets/filler-bg0-DxMaWJpb.webp"),
        "image/webp",
    ),
];

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
        .route("/assets/{filename}", get(handle_asset))
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

    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth_header == format!("Bearer {}", token) {
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
}

async fn handle_status(State(state): State<DashboardState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return *e;
    }

    let kanban_task_count = count_all_kanban_tasks(&state.kanban_dir).unwrap_or(0);

    Json(StatusResponse {
        status: "running".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        gateway_running: crate::gateway_runner::is_running().await,
        agent_model: state.app_config.agent.model.clone(),
        kanban_tasks: kanban_task_count,
        database_path: state.app_config.database_path.display().to_string(),
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
    let from_cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let (name, value) = c.trim().split_once('=')?;
                (name == TOKEN_COOKIE).then_some(value)
            })
        })
        .is_some_and(|t| t == token);

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

/// Serve an embedded static asset (JS, CSS, font, image).
async fn handle_asset(
    State(_state): State<DashboardState>,
    Path(filename): Path<String>,
) -> Response {
    // Look up the asset by filename.
    for (name, bytes, content_type) in ASSETS {
        if *name == filename {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, *content_type)
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .body(axum::body::Body::from(*bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    StatusCode::NOT_FOUND.into_response()
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

    /// A running dashboard on an ephemeral port.
    struct TestServer {
        port: u16,
    }

    /// Start the REAL server on an ephemeral port and return its address.
    async fn start(token: &str) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the bound address").port();
        let config = AppConfig::default();
        let app = build_app(&config, "127.0.0.1", port, Some(token.to_string()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        TestServer { port }
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

    /// The API routes still require the bearer token.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_api_routes_still_require_the_bearer_token() {
        let server = start("secret-token-value").await;

        let (status, _, _) = get(server.port, "/api/status", &[]);
        assert!(
            status.contains("401"),
            "the API must refuse an unauthenticated caller, got: {status}"
        );
        let (status, _, body) = get(
            server.port,
            "/api/status",
            &[("Authorization", "Bearer secret-token-value")],
        );
        assert!(status.contains("200"), "the bearer token opens the API");
        assert!(body.contains("\"status\""), "the body is the status");
    }
}
