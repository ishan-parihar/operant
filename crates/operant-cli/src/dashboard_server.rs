//! Dashboard HTTP server.
//! Ported from operant-agent/operant_cli/web_server.py.
//! Serves a minimal web dashboard for Operant status monitoring.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use operant_core::config::AppConfig;
use operant_core::kanban::KanbanManager;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Embedded frontend HTML (compiled into binary).
/// Served at the root route for zero-deployment static assets.
const INDEX_HTML: &str = include_str!("dashboard/index.html");

/// Server state shared across all handlers.
#[derive(Clone)]
pub struct DashboardState {
    pub start_time: Instant,
    pub app_config: Arc<AppConfig>,
    pub kanban_dir: Option<std::path::PathBuf>,
}

/// Start the dashboard server and block until shutdown.
pub async fn run_dashboard(config: &AppConfig, host: &str, port: u16) -> Result<()> {
    let kanban_dir = config
        .database_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let state = DashboardState {
        start_time: Instant::now(),
        app_config: Arc::new(config.clone()),
        kanban_dir: Some(kanban_dir),
    };

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/boards", get(handle_boards))
        .route("/api/health", get(handle_health))
        .route("/api/config", get(handle_config))
        .route("/", get(handle_index))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    tracing::info!("Dashboard server listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
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

async fn handle_status(State(state): State<DashboardState>) -> Json<StatusResponse> {
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

async fn handle_boards(State(state): State<DashboardState>) -> Json<Vec<BoardResponse>> {
    let kanban_dir = match &state.kanban_dir {
        Some(d) => d.clone(),
        None => return Json(Vec::new()),
    };

    if !kanban_dir.exists() {
        return Json(Vec::new());
    }

    let mgr = KanbanManager::new(kanban_dir);
    let boards = match mgr.list_boards() {
        Ok(b) => b,
        Err(_) => return Json(Vec::new()),
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

    Json(result)
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

async fn handle_config(State(state): State<DashboardState>) -> Json<ConfigResponse> {
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
}

async fn handle_index() -> impl IntoResponse {
    Html(INDEX_HTML)
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
