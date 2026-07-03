//! Dashboard HTTP server.
//! Serves the embedded web dashboard for Operant status monitoring.
//!
//! Security: API endpoints (except /api/health) require a bearer token
//! that is generated at startup and printed to the console. The token
//! is injected into index.html as a global variable so the frontend
//! can call the API without manual configuration.

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use operant_core::config::AppConfig;
use operant_core::kanban::KanbanManager;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Embedded frontend HTML (compiled into binary).
const INDEX_HTML: &str = include_str!("dashboard/index.html");

/// Embedded static assets (JS, CSS, fonts, images).
/// These are served at /assets/<filename> so the index.html script/link tags resolve.
static ASSETS: &[(&str, &[u8], &str)] = &[
    ("index-BB4BRelo.js", include_bytes!("dashboard/assets/index-BB4BRelo.js"), "text/javascript"),
    ("index-DJxmcHRv.css", include_bytes!("dashboard/assets/index-DJxmcHRv.css"), "text/css"),
    ("Collapse-Bold-mgICk9-_.woff2", include_bytes!("dashboard/assets/Collapse-Bold-mgICk9-_.woff2"), "font/woff2"),
    ("Collapse-Regular-DysayoTY.woff2", include_bytes!("dashboard/assets/Collapse-Regular-DysayoTY.woff2"), "font/woff2"),
    ("Mondwest-Regular-CWscgue7.woff2", include_bytes!("dashboard/assets/Mondwest-Regular-CWscgue7.woff2"), "font/woff2"),
    ("RulesCompressed-Medium-CA76_CrB.woff2", include_bytes!("dashboard/assets/RulesCompressed-Medium-CA76_CrB.woff2"), "font/woff2"),
    ("RulesCompressed-Regular-BSXFyF4x.woff2", include_bytes!("dashboard/assets/RulesCompressed-Regular-BSXFyF4x.woff2"), "font/woff2"),
    ("RulesExpanded-Bold-DZA7s8Pa.woff2", include_bytes!("dashboard/assets/RulesExpanded-Bold-DZA7s8Pa.woff2"), "font/woff2"),
    ("RulesExpanded-Regular-l8uVympt.woff2", include_bytes!("dashboard/assets/RulesExpanded-Regular-l8uVympt.woff2"), "font/woff2"),
    ("filler-bg0-DxMaWJpb.webp", include_bytes!("dashboard/assets/filler-bg0-DxMaWJpb.webp"), "image/webp"),
];

/// Server state shared across all handlers.
#[derive(Clone)]
pub struct DashboardState {
    pub start_time: Instant,
    pub app_config: Arc<AppConfig>,
    pub kanban_dir: Option<std::path::PathBuf>,
    /// Session token for API auth. If None, auth is disabled (--insecure mode).
    pub session_token: Option<String>,
}

/// Start the dashboard server and block until shutdown.
pub async fn run_dashboard(config: &AppConfig, host: &str, port: u16, insecure: bool) -> Result<()> {
    let kanban_dir = config
        .database_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Generate a session token for API auth (unless --insecure).
    let session_token = if insecure {
        tracing::warn!("Dashboard running in INSECURE mode — no auth token required");
        None
    } else {
        let token = generate_session_token();
        println!("Dashboard session token: {}", token);
        println!("  (API requests must include header: Authorization: Bearer {})", token);
        Some(token)
    };

    let state = DashboardState {
        start_time: Instant::now(),
        app_config: Arc::new(config.clone()),
        kanban_dir: Some(kanban_dir),
        session_token,
    };

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/boards", get(handle_boards))
        .route("/api/health", get(handle_health))
        .route("/api/config", get(handle_config))
        .route("/assets/:filename", get(handle_asset))
        .route("/", get(handle_index))
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    tracing::info!("Dashboard server listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Generate a random session token.
fn generate_session_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("operant_{:x}{:x}", now.as_secs(), now.subsec_nanos())
}

/// Check the Authorization header against the session token.
/// Returns Ok(()) if authorized, Err(response) if not.
fn check_auth(state: &DashboardState, headers: &HeaderMap) -> Result<(), Response> {
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
        Err((
            StatusCode::UNAUTHORIZED,
            "Missing or invalid Authorization header",
        )
            .into_response())
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
        return e;
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
        return e;
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
        return e;
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

async fn handle_index(State(state): State<DashboardState>) -> impl IntoResponse {
    // Inject the session token into the HTML so the frontend can use it.
    let html = match &state.session_token {
        Some(token) => INDEX_HTML.replace(
            "<div id=\"root\"></div>",
            &format!(
                "<script>window.__OPERANT_SESSION_TOKEN__=\"{}\";</script>\n<div id=\"root\"></div>",
                token
            ),
        ),
        None => INDEX_HTML.to_string(),
    };
    Html(html)
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
