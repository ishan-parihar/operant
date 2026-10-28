//! The gateway daemon's read-only session API (plan 2026-10-28 iter-E, G8).
//!
//! `operant gateway run` already holds the machine's single-daemon flock; this
//! module gives that same process one axum route group so the OTHER surfaces
//! (dashboard page, scripts, a future second TUI) can read the unified session
//! record over HTTP instead of opening the database:
//!
//! - `GET /api/sessions` — `Database::list_sessions_rich`, the SAME rows the
//!   TUI hub shows (plan goal: one record, every surface).
//! - `GET /api/sessions/:id/messages?cap=N` — capped `get_session_messages`.
//! - `GET /api/subagents` — the live delegation registry snapshot
//!   (`records_snapshot`), the hub's subagent peer rows.
//!
//! Auth is the dashboard's token discipline (AD-040) verbatim: a token minted
//! at startup, accepted as `Authorization: Bearer`, `X-Hermes-Session-Token`,
//! or the dashboard cookie. 127.0.0.1 binding is defense-in-depth — the token
//! is the gate. Read-only: no route writes, so the flock already held by the
//! daemon is the single-writer guarantee and this listener never needs one.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use serde::Deserialize;
use std::sync::Arc;

use operant_core::database::Database;
use operant_core::tools::async_delegation;

/// Shared, immutable state for the route group: the DB handle the routes read
/// through (one connection each request — SQLite serializes them anyway) and
/// the startup token.
#[derive(Clone)]
pub struct GatewayApiState {
    db_path: Arc<std::path::PathBuf>,
    token: Arc<String>,
}

impl GatewayApiState {
    pub fn new(db_path: std::path::PathBuf, token: String) -> Self {
        Self {
            db_path: Arc::new(db_path),
            token: Arc::new(token),
        }
    }
}

/// Token gate (dashboard_server's three-way acceptance, same order):
/// `Authorization: Bearer`, the `X-Hermes-Session-Token` header, or the
/// `operant_dashboard_session` cookie. A missing/foreign token is 401.
fn token_ok(state: &GatewayApiState, headers: &HeaderMap) -> bool {
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION)
        && let Ok(v) = auth.to_str()
        && let Some(bearer) = v.strip_prefix("Bearer ")
    {
        return bearer == state.token.as_str();
    }
    if let Some(t) = headers.get("x-hermes-session-token")
        && let Ok(v) = t.to_str()
    {
        return v == state.token.as_str();
    }
    if let Some(cookie) = headers.get(axum::http::header::COOKIE)
        && let Ok(v) = cookie.to_str()
        && v.split(';').any(|c| {
            let c = c.trim();
            c.strip_prefix("operant_dashboard_session=")
                .is_some_and(|t| t == state.token.as_str())
        })
    {
        return true;
    }
    false
}

/// Serialize one row to the hub's shape. Hand-mapped (not a derive):
/// `RowKind::Subagent` carries nested fields and `updated_at` is a
/// `DateTime<Utc>` whose serde form would need a chrono serde feature —
/// the explicit map is the API contract, visible here.
fn row_to_json(row: &operant_core::session::listing::SessionEntryV2) -> serde_json::Value {
    use operant_core::session::listing::RowKind;
    let mut v = serde_json::json!({
        "id": row.id,
        "parent_id": row.parent_id,
        "title": row.title,
        "updated_at": row.updated_at.to_rfc3339(),
        "employee_id": row.employee_id,
        "source": row.source,
        "message_count": row.message_count,
        "cost_usd": row.cost_usd,
        "tokens": row.tokens,
        "cwd": row.cwd,
        "status": row.status.api_label(),
    });
    let kind = match &row.kind {
        RowKind::Session => serde_json::json!("session"),
        RowKind::Branch => serde_json::json!("branch"),
        RowKind::Subagent {
            goal,
            model,
            dispatch_time,
            status,
        } => serde_json::json!({
            "type": "subagent",
            "goal": goal,
            "model": model,
            "dispatch_time": dispatch_time,
            "status": status,
        }),
    };
    v["kind"] = kind;
    v
}

async fn list_sessions(
    State(state): State<GatewayApiState>,
    headers: HeaderMap,
) -> StatusCodeResult {
    if !token_ok(&state, &headers) {
        return auth_denied();
    }
    // The rows the TUI hub loads (500 there); one number, one shape.
    match tokio::task::spawn_blocking(move || Database::init(state.db_path.as_ref().clone())).await
    {
        Ok(Ok(db)) => match db.list_sessions_rich(500) {
            Ok(rows) => {
                let rows: Vec<_> = rows.iter().map(row_to_json).collect();
                ok_json(rows)
            }
            Err(e) => err_json(e),
        },
        Ok(Err(e)) => err_json(e),
        Err(e) => err_json(e),
    }
}

type StatusCodeResult = Result<(StatusCode, axum::Json<serde_json::Value>), StatusCode>;

fn ok_json(rows: impl serde::Serialize) -> StatusCodeResult {
    match serde_json::to_value(rows) {
        Ok(v) => Ok((StatusCode::OK, axum::Json(v))),
        Err(e) => Err(internal_error(e)),
    }
}

fn err_json(e: impl std::fmt::Display) -> StatusCodeResult {
    Ok((
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({ "error": e.to_string() })),
    ))
}

fn auth_denied() -> StatusCodeResult {
    Ok((
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "unauthorized" })),
    ))
}

fn internal_error(e: impl std::fmt::Display) -> StatusCode {
    tracing::warn!(error = %e, "gateway API handler failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(Deserialize, Default)]
struct MessagesQuery {
    #[serde(default)]
    cap: Option<usize>,
}

async fn session_messages(
    State(state): State<GatewayApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> StatusCodeResult {
    if !token_ok(&state, &headers) {
        return auth_denied();
    }
    // Cap the transcript a single GET can pull (the plan's `?cap=N`; 100
    // default, 1000 ceiling) so a curl cannot stream the whole history.
    let cap = q.cap.unwrap_or(100).min(1000);
    match tokio::task::spawn_blocking(move || Database::init(state.db_path.as_ref().clone())).await
    {
        Ok(Ok(db)) => match db.get_session_messages(&id) {
            Ok(mut messages) => {
                if messages.len() > cap {
                    messages.truncate(cap);
                }
                // The legacy Message shape: role/content/timestamp, as-is.
                let rows: Vec<_> = messages
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "role": m.role,
                            "content": m.content,
                            "timestamp": m.timestamp,
                        })
                    })
                    .collect();
                ok_json(rows)
            }
            Err(e) => err_json(e),
        },
        Ok(Err(e)) => err_json(e),
        Err(e) => err_json(e),
    }
}

async fn subagents(State(state): State<GatewayApiState>, headers: HeaderMap) -> StatusCodeResult {
    if !token_ok(&state, &headers) {
        return auth_denied();
    }
    let rows = async_delegation::records_snapshot();
    let rows: Vec<_> = rows.iter().map(row_to_json).collect();
    ok_json(rows)
}

/// The route group. Bind it loopback in the caller (`[gateway] api_bind`
/// default "127.0.0.1:8731") — the token, not the address, is the gate.
pub fn build_router(state: GatewayApiState) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}/messages", get(session_messages))
        .route("/api/subagents", get(subagents))
        .with_state(state)
}

/// Spawn the API listener inside the gateway daemon's process. The task is
/// detached (dies with the daemon); a bind failure logs and the gateway runs
/// without the API rather than refusing to start — the messaging surface is
/// the product, the API is a read-only convenience.
pub fn spawn_if_enabled(
    gateway: &operant_core::config::GatewaySettings,
    db_path: &std::path::Path,
) {
    if !gateway.api_enabled {
        return;
    }
    let bind = gateway.api_bind.clone();
    // The dashboard's token generator (/dev/urandom-backed) — same
    // discipline, a fresh token per daemon start.
    let state = GatewayApiState::new(
        db_path.to_path_buf(),
        crate::dashboard_server::new_session_token(),
    );
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&bind).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, bind = %bind, "gateway API listener not started");
                return;
            }
        };
        tracing::info!(bind = %bind, "gateway session API listening");
        let app = build_router(state);
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!(error = %e, "gateway session API stopped");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    /// A running API on an ephemeral port (the dashboard_server test
    /// pattern: real listener, raw HTTP — no tower dependency).
    struct TestApi {
        port: u16,
    }

    impl TestApi {
        fn start() -> Self {
            use futures::FutureExt;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .now_or_never()
                .expect("bind is synchronous")
                .expect("an ephemeral port is free");
            let port = listener
                .local_addr()
                .expect("the ephemeral listener has an address")
                .port();
            let dir = std::env::temp_dir().join(format!("operant_gateway_api_test_{port}"));
            std::fs::create_dir_all(&dir).expect("create the temp dir");
            let db_path = dir.join("api_test.db");
            // Init the schema through the real path so the routes read a
            // real (empty) database, not a stub.
            if let Ok(db) = Database::init(db_path.clone()) {
                let _ = db.list_sessions_rich(10);
            }
            let state = GatewayApiState::new(db_path, "test-token".to_string());
            let app = build_router(state);
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            TestApi { port }
        }

        /// Minimal HTTP/1.1 GET → (status line, body).
        fn get(&self, path: &str, headers: &[(&str, &str)]) -> (String, String) {
            let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
            let mut request = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
                self.port
            );
            for (name, value) in headers {
                request.push_str(&format!("{name}: {value}\r\n"));
            }
            request.push_str("\r\n");
            stream.write_all(request.as_bytes()).expect("write");
            stream.flush().expect("flush");
            let mut reader = BufReader::new(stream);
            let mut status = String::new();
            reader.read_line(&mut status).expect("status line");
            let mut rest = String::new();
            reader.read_to_string(&mut rest).expect("body");
            // The status line is followed by headers, a blank line, then the
            // body — split on the first blank line.
            let body = rest
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default();
            (status, body)
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessions_rejects_missing_and_foreign_tokens() {
        let api = TestApi::start();
        let (s, _) = api.get("/api/sessions", &[]);
        assert!(s.contains("401"), "anon: {s}");
        let (s, _) = api.get("/api/sessions", &[("Authorization", "Bearer wrong-token")]);
        assert!(s.contains("401"), "foreign: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessions_serves_the_unified_rows() {
        let api = TestApi::start();
        let (s, body) = api.get("/api/sessions", &[("Authorization", "Bearer test-token")]);
        assert!(s.contains("200"), "status: {s}");
        let v: serde_json::Value = serde_json::from_str(&body).expect("the body is JSON");
        assert!(v.is_array(), "the hub row shape is a JSON array");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn header_and_cookie_token_variants_are_accepted() {
        let api = TestApi::start();
        let (s, _) = api.get(
            "/api/subagents",
            &[("X-Hermes-Session-Token", "test-token")],
        );
        assert!(s.contains("200"), "header variant: {s}");
        let (s, _) = api.get(
            "/api/subagents",
            &[("Cookie", "other=1; operant_dashboard_session=test-token")],
        );
        assert!(s.contains("200"), "cookie variant: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn messages_route_answers_a_missing_id_with_an_empty_array() {
        let api = TestApi::start();
        // Read-only: an unknown session id is a miss, not a 500.
        let (s, body) = api.get(
            "/api/sessions/no-such-id/messages?cap=5",
            &[("Authorization", "Bearer test-token")],
        );
        assert!(s.contains("200"), "status: {s}");
        let v: serde_json::Value = serde_json::from_str(&body).expect("the body is JSON");
        assert!(v.is_array(), "an empty transcript is []");
    }
}
