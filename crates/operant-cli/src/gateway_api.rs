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
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use std::sync::Arc;

use operant_core::tools::async_delegation;

/// Shared, immutable state for the route group: the DB handle the routes read
/// through (one connection each request — SQLite serializes them anyway) and
/// the startup token.
#[derive(Clone)]
pub struct GatewayApiState {
    /// The ONE database handle the API serves from (RT-519).
    ///
    /// Every request used to call `Database::init`, which opens the
    /// connection, sets pragmas, tightens permissions, reconciles
    /// columns, runs the migration runner and creates the tables — so a
    /// client polling `/api/sessions` every second took the SQLite WRITE
    /// lock against the daemon's own turn writes and could get back
    /// `500 {"error":"database is locked"}`.
    db: Arc<operant_core::database::Database>,
    token: Arc<String>,
    /// The `Host` values this API answers to (A3-gateway H-4).
    ///
    /// Derived from the address it was told to bind, the same way
    /// `dashboard_server` derives its own -- see `allowed_hosts`.
    allowed_hosts: Vec<String>,
}

impl GatewayApiState {
    /// `bind` is the address the caller is about to serve on
    /// (`[gateway] api_bind`); its loopback spellings become the Host
    /// allowlist. A bind that is not parseable as `host:port` yields an EMPTY
    /// allowlist, which fails closed -- every request is refused rather than
    /// the guard silently passing everything (the failure mode a `unwrap_or_default`
    /// here would create).
    pub fn new(db_path: std::path::PathBuf, token: String, bind: &str) -> std::io::Result<Self> {
        let db = operant_core::database::Database::init(db_path.clone())
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let allowed_hosts = match bind.rsplit_once(':') {
            Some((host, port)) => match port.parse::<u16>() {
                Ok(port) => crate::dashboard_server::allowed_hosts(host, port),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        Ok(Self {
            db: Arc::new(db),
            token: Arc::new(token),
            allowed_hosts,
        })
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
        // Do NOT log the token or the headers here. This fires on every 401, so
        // the daemon's stderr/journal accumulated the VALID bearer token on every
        // failed request -- anyone who can read the unit's logs (or trigger a
        // request and then read them) recovers the credential. `token_ok` is the
        // only thing worth recording: that the check ran and refused.
        tracing::debug!("gateway_api: token check failed");
        return auth_denied();
    }
    // The rows the TUI hub loads (500 there); one number, one shape.
    let db = state.db.clone();
    match tokio::task::spawn_blocking(move || db.list_sessions_rich(500)).await {
        Ok(Ok(rows)) => {
            let rows: Vec<_> = rows.iter().map(row_to_json).collect();
            ok_json(rows)
        }
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
    //
    // (RT-519) The cap is pushed into SQL: the old path selected the WHOLE
    // transcript and truncated it in Rust, so a 50,000-message session
    // materialised 50,000 structs per request and returned the FIRST 100
    // — the opposite of what "cap" means. The query now selects the NEWEST
    // `cap` rows (newest-first with a LIMIT, then reversed back into
    // transcript order).
    let cap = q.cap.unwrap_or(100).min(1000);
    let db = state.db.clone();
    match tokio::task::spawn_blocking(move || db.get_session_messages_capped(&id, cap)).await {
        Ok(Ok(messages)) => {
            {
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
        }
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

/// Refuse a request whose `Host` header is not one this server answers to.
///
/// (A3-gateway H-4) The token alone is not enough on a loopback surface: a page
/// the operator visits can point a name it controls at 127.0.0.1:8731 (DNS
/// rebinding) and the browser sends whatever `Host` the page chose, which is
/// the one header that says "this request came from a stranger".
///
/// `dashboard_server::host_is_allowed` already implements the check, with the
/// DNS-rebinding rationale and the port-stripping / IPv6-literal handling that
/// this API needs identically — so this delegates rather than restating it.
/// A second copy is how the dashboard and the API would come to disagree about
/// what "loopback" means.
async fn host_guard(
    State(state): State<GatewayApiState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if !crate::dashboard_server::host_is_allowed(request.headers(), &state.allowed_hosts) {
        return (
            StatusCode::FORBIDDEN,
            "the Host header does not match this server's address",
        )
            .into_response();
    }
    next.run(request).await
}

/// The route group. Bind it loopback in the caller (`[gateway] api_bind`
/// default "127.0.0.1:8731") — the token AND the Host header are the gates.
///
/// (A3-gateway H-4) The token alone was the gate until iter-2525. It is a
/// 32-byte secret compared in the clear, and it is attached by the browser to
/// every request to this origin — including one a page chose the Host of. The
/// Host guard closes the DNS-rebinding read of `/api/sessions` and friends.
pub fn build_router(state: GatewayApiState) -> Router {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}/messages", get(session_messages))
        .route("/api/subagents", get(subagents))
        .layer(middleware::from_fn_with_state(state.clone(), host_guard))
        .with_state(state)
}

/// Spawn the API listener inside the gateway daemon's process. The task is
/// detached (dies with the daemon); a bind failure logs and the gateway runs
/// without the API rather than refusing to start — the messaging surface is
/// the product, the API is a read-only convenience.
pub async fn spawn_if_enabled(
    gateway: &operant_core::config::GatewaySettings,
    db_path: &std::path::Path,
) -> ApiStartOutcome {
    if !gateway.api_enabled {
        return ApiStartOutcome::Disabled;
    }
    let bind = gateway.api_bind.clone();
    // The dashboard's token generator (/dev/urandom-backed) — same
    // discipline, a fresh token per daemon start. PRINTED once to stdout
    // (journal under systemd): a token the operator cannot read is a
    // feature nobody can use, and the dashboard sets the precedent.
    let token = crate::dashboard_server::new_session_token();
    // (RT-523) BIND FIRST, print the token second. The old order printed
    // the token and THEN bound, and a bind failure was only a WARN — so a
    // port conflict printed a token line followed by nothing useful, and
    // the operator believed the API was up.
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(error = %e, bind = %bind, "gateway session API NOT started");
            println!("Gateway session API NOT started — could not bind {bind}: {e}");
            return ApiStartOutcome::NotStarted(format!("could not bind {bind}: {e}"));
        }
    };
    println!("Gateway session API on http://{bind} — Authorization: Bearer {token}");
    let state = match GatewayApiState::new(db_path.to_path_buf(), token, &bind) {
        Ok(state) => state,
        Err(e) => {
            tracing::error!(error = %e, "the gateway session API could not open its database");
            println!("Gateway session API NOT started — could not open the database: {e}");
            return ApiStartOutcome::NotStarted(format!("could not open the database: {e}"));
        }
    };
    tokio::spawn(async move {
        tracing::info!(bind = %bind, "gateway session API listening");
        let app = build_router(state);
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!(error = %e, "gateway session API stopped");
        }
    });
    ApiStartOutcome::Started
}

/// What the API spawn did (RT-523).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiStartOutcome {
    /// The API is disabled in the config — nothing was attempted.
    Disabled,
    /// The listener is bound and the token was printed.
    Started,
    /// The bind failed: NOTHING is listening and no token was printed.
    NotStarted(String),
}

#[cfg(test)]
mod tests {
    use operant_core::database::Database;

    /// RT-523: a failed bind starts NOTHING and says so.
    ///
    /// The old order printed the bearer token and THEN bound, logging a
    /// bind failure as a warning — so a port conflict printed a token line
    /// followed by nothing useful, and the operator believed the API was up.
    #[tokio::test]
    async fn a_failed_bind_starts_nothing() {
        let holder = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("hold a port");
        let addr = holder.local_addr().expect("addr").to_string();
        let settings = operant_core::config::GatewaySettings {
            api_enabled: true,
            api_bind: addr.clone(),
            ..Default::default()
        };

        let outcome = super::spawn_if_enabled(
            &settings,
            std::path::Path::new("/tmp/operant-acp-bind-test.sqlite"),
        )
        .await;
        assert!(
            matches!(outcome, super::ApiStartOutcome::NotStarted(_)),
            "a bind conflict must report NOT started, got {outcome:?}"
        );
        // The port is still the holder's: the API took nothing.
        assert!(
            tokio::net::TcpListener::bind(&addr).await.is_err(),
            "the API must not have bound the port"
        );
    }

    /// RT-519: the API must NOT re-open the database per request.
    ///
    /// Every request used to call `Database::init`, which opens the
    /// connection, sets pragmas, tightens permissions, reconciles columns,
    /// runs the migration runner and creates the tables — so a client
    /// polling `/api/sessions` every second took the SQLite WRITE lock
    /// against the daemon's own turn writes and could get back
    /// `500 {"error":"database is locked"}`.
    ///
    /// A SOURCE contract over the seam: the open is a side effect on the
    /// database, and the handle is private to the state.
    #[test]
    fn the_api_does_not_reopen_the_database_per_request() {
        let source = include_str!("gateway_api.rs");
        // The PRODUCTION part of the file: the test module opens its own
        // database on purpose.
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        // Assembled from two literals so this test's own source cannot
        // satisfy the needle: the CALL, with its parenthesis, so a doc
        // comment that names the function is not an open.
        let opens = ["Database::init", "("].concat();
        let count = production.matches(opens.as_str()).count();
        assert_eq!(
            count, 1,
            "the API must open the database ONCE — per request it re-ran the \
             migrations and took the daemon's write lock"
        );
    }

    /// RT-523: the token is printed only AFTER the bind succeeds.
    ///
    /// The old order printed the bearer token and THEN bound, so a port
    /// conflict left a token line and nothing else — the operator
    /// believed the API was up.
    ///
    /// A SOURCE contract over the seam: the print is a side effect on
    /// stdout, which a test cannot capture portably, but the ORDER is the
    /// invariant.
    #[test]
    fn the_token_is_printed_after_the_bind() {
        let source = include_str!("gateway_api.rs");
        let bind_at = source.find("TcpListener::bind").expect("the bind call");
        let print_at = source
            .find("Gateway session API on http://")
            .expect("the token print");
        assert!(
            bind_at < print_at,
            "the token must be printed only after the bind succeeds"
        );
    }

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
            // The bind string must describe where the listener ACTUALLY is,
            // because `GatewayApiState::new` derives the Host allowlist from
            // it. Binding ephemeral and passing a hardcoded port would make
            // the server refuse its own requests.
            let bind = format!("127.0.0.1:{port}");
            let state = GatewayApiState::new(db_path, "test-token".to_string(), &bind)
                .expect("the API opens its database");
            let app = build_router(state);
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            TestApi { port }
        }

        /// Minimal HTTP/1.1 GET → (status line, body).
        fn get(&self, path: &str, headers: &[(&str, &str)]) -> (String, String) {
            self.get_with_host(path, headers, None)
        }

        /// The same, with a chosen `Host` header.
        ///
        /// `None` sends the server's own `127.0.0.1:<port>`, which the guard
        /// allows. `Some(host)` is how a test plays the part of a page that
        /// pointed a name it controls at this loopback port (A3-gateway H-4:
        /// the browser sends whatever `Host` the page chose).
        fn get_with_host(
            &self,
            path: &str,
            headers: &[(&str, &str)],
            host: Option<&str>,
        ) -> (String, String) {
            let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
            let host = host
                .map(|h| h.to_string())
                .unwrap_or_else(|| format!("127.0.0.1:{}", self.port));
            // Header NAMES only. The values carry `Authorization: Bearer <token>`,
            // so dumping the map wrote the live credential into the logs on every
            // proxied request.
            let header_names: Vec<&str> = headers.keys().map(|n| n.as_str()).collect();
            tracing::debug!(host = %host, header_names = ?header_names, "proxy request");
            let mut request =
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
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

    /// (A3-gateway H-4) A foreign `Host` header is refused even with a VALID
    /// token.
    ///
    /// This is the DNS-rebinding read: a page the operator visits points a name
    /// it controls at 127.0.0.1:8731, the browser connects, and it attaches the
    /// token because the request is to this origin -- but it also sends the
    /// `Host` the page chose. The token alone cannot tell those apart. Before
    /// the guard, this returned 200 and `/api/sessions` (the operator's whole
    /// session record) was readable cross-origin.
    ///
    /// The positive control matters as much as the refusal: the SAME token on
    /// the server's own host must still work, or the guard could be "refuse
    /// everything" and this test would pass.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_foreign_host_is_refused_even_with_a_valid_token() {
        let api = TestApi::start();
        let token = format!("{}{} test-token", "Bea", "rer");
        let auth = [("Authorization", token.as_str())];

        // Control: the server's own Host, valid token -> 200.
        let (ok, _) = api.get_with_host("/api/sessions", &auth, None);
        assert!(
            ok.contains("200"),
            "the server must answer its OWN host with a valid token; got {ok}"
        );

        // The attack: a Host the page chose, same valid token -> 403.
        let (bad, _) = api.get_with_host("/api/sessions", &auth, Some("attacker.example:8731"));
        assert!(
            bad.contains("403"),
            "a foreign Host must be refused even with a valid token (DNS \
             rebinding); got {bad}"
        );
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
