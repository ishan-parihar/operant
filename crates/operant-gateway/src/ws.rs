//! WebSocket agent chat handler.
//!
//! Connect: `ws://host:port/ws/chat?session_id=ID&name=My+Session`
//!
//! Protocol:
//! ```text
//! Server -> Client: {"type":"session_start","session_id":"...","name":"...","resumed":true,"message_count":42}
//! Client -> Server: {"type":"message","content":"Hello"}
//! Server -> Client: {"type":"chunk","content":"Hi! "}
//! Server -> Client: {"type":"tool_call","name":"shell","args":{...}}
//! Server -> Client: {"type":"tool_result","name":"shell","output":"..."}
//! Server -> Client: {"type":"done","full_response":"..."}
//! ```
//!
//! ## Tool approvals
//!
//! When supervised-mode tool calls hit the `ApprovalManager`, the server
//! emits an `approval_request` and pauses the tool loop until the client
//! responds. Mirrors the Telegram inline-keyboard / CLI Y/N/A pattern,
//! over the WS frame transport.
//!
//! ```text
//! Server -> Client: {
//!     "type": "approval_request",
//!     "request_id": "<uuid>",
//!     "tool": "shell",
//!     "arguments_summary": "command: git status",
//!     "timeout_secs": 120
//! }
//! Client -> Server: {
//!     "type": "approval_response",
//!     "request_id": "<uuid>",
//!     "decision": "approve" | "deny" | "always"
//! }
//! ```
//!
//! `approve` runs the tool once, `always` adds the tool to the session
//! allowlist for the rest of the conversation, `deny` returns a structured
//! error to the model. When no client is connected, or the client
//! disconnects mid-prompt, the tool call is auto-denied after `timeout_secs`.
//!
//! ### `arguments_summary` security boundary
//!
//! `arguments_summary` is a human-readable string the runtime synthesises
//! for the operator (e.g. `"command: git status"`, `"path: /etc/hosts"`).
//! It is render-only; the operator's approve/deny choice attaches to the
//! `request_id`, never to the summary string. The runtime must not echo
//! any `#[secret]` or `#[derived_from_secret]` field (auth tokens, API
//! keys, OAuth secrets) into the summary. The agent's tool loop runs
//! tool args through `operant_runtime::approval::summarize_args` before
//! the request reaches this transport; do not stringify raw args here.
//!
//! Query params:
//! - `session_id` — resume or create a session (default: new UUID)
//! - `name` — optional human-readable label for the session
//! - `token` — bearer auth token (alternative to Authorization header)

use super::AppState;
use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, header},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use operant_api::channel::ChannelApprovalResponse;
use operant_runtime::agent::reconciled::{ReconciledAgent, TurnExitReason};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Optional connection parameters sent as the first WebSocket message.
///
/// If the first message after upgrade is `{"type":"connect",...}`, these
/// parameters are extracted and an acknowledgement is sent back. Old clients
/// that send `{"type":"message",...}` as the first frame still work — the
/// message is processed normally (backward-compatible).
#[derive(Debug, Deserialize)]
struct ConnectParams {
    #[serde(rename = "type")]
    msg_type: String,
    /// Client-chosen session ID for memory persistence
    #[serde(default)]
    session_id: Option<String>,
    /// Device name for device registry tracking
    #[serde(default)]
    device_name: Option<String>,
    /// Client capabilities
    #[serde(default)]
    capabilities: Vec<String>,
    /// Project root / working directory for this session.
    #[serde(default, alias = "workspaceDir", alias = "workspace_dir")]
    cwd: Option<String>,
}

/// The sub-protocol we support for the chat WebSocket.
const WS_PROTOCOL: &str = "operant.v1";

/// Prefix used in `Sec-WebSocket-Protocol` to carry a bearer token.
const BEARER_SUBPROTO_PREFIX: &str = "bearer.";

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
    pub session_id: Option<String>,
    /// Optional human-readable name for the session.
    pub name: Option<String>,
    /// Project root / working directory for this session.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, alias = "workspaceDir", alias = "workspace_dir")]
    pub workspace_dir: Option<String>,
}

/// Extract a bearer token from WebSocket-compatible sources.
///
/// Precedence (first non-empty wins):
/// 1. `Authorization: Bearer <token>` header
/// 2. `Sec-WebSocket-Protocol: bearer.<token>` subprotocol
/// 3. `?token=<token>` query parameter
///
/// Browsers cannot set custom headers on `new WebSocket(url)`, so the query
/// parameter and subprotocol paths are required for browser-based clients.
fn extract_ws_token<'a>(headers: &'a HeaderMap, query_token: Option<&'a str>) -> Option<&'a str> {
    // 1. Authorization header
    if let Some(t) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        && !t.is_empty()
    {
        return Some(t);
    }

    // 2. Sec-WebSocket-Protocol: bearer.<token>
    if let Some(t) = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|protos| {
            protos
                .split(',')
                .map(|p| p.trim())
                .find_map(|p| p.strip_prefix(BEARER_SUBPROTO_PREFIX))
        })
        && !t.is_empty()
    {
        return Some(t);
    }

    // 3. ?token= query parameter
    if let Some(t) = query_token
        && !t.is_empty()
    {
        return Some(t);
    }

    None
}

/// GET /ws/chat — WebSocket upgrade for agent chat
pub async fn handle_ws_chat(
    State(state): State<AppState>,
    Query(params): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Auth: check header, subprotocol, then query param (precedence order)
    if state.pairing.require_pairing() {
        let token = extract_ws_token(&headers, params.token.as_deref()).unwrap_or("");
        if !state.pairing.is_authenticated(token) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "Unauthorized — provide Authorization header, Sec-WebSocket-Protocol bearer, or ?token= query param",
            )
                .into_response();
        }
    }

    // Echo Sec-WebSocket-Protocol if the client requests our sub-protocol.
    let ws = if headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|protos| protos.split(',').any(|p| p.trim() == WS_PROTOCOL))
    {
        ws.protocols([WS_PROTOCOL])
    } else {
        ws
    };

    let session_id = params.session_id;
    let session_name = params.name;
    let session_cwd = params.cwd.or(params.workspace_dir);
    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id, session_name, session_cwd))
        .into_response()
}

/// Gateway session key prefix to avoid collisions with channel sessions.
const GW_SESSION_PREFIX: &str = "gw_";

#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    session_id: Option<String>,
    session_name: Option<String>,
    session_cwd: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();

    // Resolve session ID: use provided or generate a new UUID
    let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let session_key = format!("{GW_SESSION_PREFIX}{session_id}");
    // Match the sanitized form persisted by memory backend migrations.
    let mut memory_session_id = operant_api::session_keys::sanitize_session_key(&session_id);

    // Hydrate session metadata from persistence (if available). Agent
    // construction is deferred until after the optional `connect` frame so the
    // client can provide a per-session cwd for the security sandbox root.
    let config = state.config.lock().clone();
    let mut resumed = false;
    let mut message_count: usize = 0;
    let mut effective_name: Option<String> = None;
    let mut stored_messages = Vec::new();
    if let Some(ref backend) = state.session_backend {
        let messages = backend.load(&session_key);
        if !messages.is_empty() {
            message_count = messages.len();
            stored_messages = messages;
            resumed = true;
        }
        // Set session name if provided (non-empty) on connect
        if let Some(ref name) = session_name
            && !name.is_empty()
        {
            let _ = backend.set_session_name(&session_key, name);
            effective_name = Some(name.clone());
        }
        // If no name was provided via query param, load the stored name
        if effective_name.is_none() {
            effective_name = backend.get_session_name(&session_key).unwrap_or(None);
        }
    }

    // Send session_start message to client
    let mut session_start = serde_json::json!({
        "type": "session_start",
        "session_id": session_id,
        "resumed": resumed,
        "message_count": message_count,
    });
    if let Some(ref name) = effective_name {
        session_start["name"] = serde_json::Value::String(name.clone());
    }
    let _ = sender
        .send(Message::Text(session_start.to_string().into()))
        .await;

    // ── Optional connect handshake ──────────────────────────────────
    // The first message may be a `{"type":"connect",...}` frame carrying
    // connection parameters.  If it is, we extract the params, send an
    // ack, and proceed to the normal message loop.  If the first message
    // is a regular `{"type":"message",...}` frame, we fall through and
    // process it immediately (backward-compatible).
    let mut first_msg_fallback: Option<String> = None;
    let mut requested_cwd = session_cwd;

    if let Some(first) = receiver.next().await {
        match first {
            Ok(Message::Text(text)) => {
                if let Ok(cp) = serde_json::from_str::<ConnectParams>(&text) {
                    if cp.msg_type == "connect" {
                        debug!(
                            session_id = ?cp.session_id,
                            device_name = ?cp.device_name,
                            capabilities = ?cp.capabilities,
                            cwd = ?cp.cwd,
                            "WebSocket connect params received"
                        );
                        if let Some(sid) = &cp.session_id {
                            memory_session_id =
                                operant_api::session_keys::sanitize_session_key(sid);
                            debug!(
                                session_id = sid,
                                "WebSocket connect session override received"
                            );
                        }
                        if cp.cwd.is_some() {
                            requested_cwd = cp.cwd;
                        }
                        let ack = serde_json::json!({
                            "type": "connected",
                            "message": "Connection established"
                        });
                        let _ = sender.send(Message::Text(ack.to_string().into())).await;
                    } else {
                        // Not a connect message — fall through to normal processing
                        first_msg_fallback = Some(text.to_string());
                    }
                } else {
                    // Not parseable as ConnectParams — fall through
                    first_msg_fallback = Some(text.to_string());
                }
            }
            Ok(Message::Close(_)) | Err(_) => return,
            _ => {}
        }
    }

    // validated for the INVALID_CWD error path; core tools resolve paths
    // against the process cwd (Loop A semantics) so the value is not
    // consumed by the facade.
    let _session_cwd = match resolve_session_cwd(requested_cwd.as_deref(), &config.workspace_dir) {
        Ok(cwd) => cwd,
        Err(e) => {
            let err = serde_json::json!({
                "type": "error",
                "message": e.to_string(),
                "code": "INVALID_CWD"
            });
            let _ = sender.send(Message::Text(err.to_string().into())).await;
            return;
        }
    };

    if let Some(err) = needs_onboarding_ws_error(&config) {
        let _ = sender.send(Message::Text(err.to_string().into())).await;
        return;
    }

    // Build a per-session reconciled-facade agent (Loop A under the hood) so
    // history is maintained across turns on this connection. The facade owns
    // injection scanning, approvals, cancellation and the TurnEvent stream;
    // wiring parity notes: the SSE observer rides through the facade's
    // core→api bridge, approvals route through `ReconciledApprovals`, and the
    // core session id keys the facade's own transcript continuity.
    //
    // Construction is shared with every other facade consumer
    // (`ReconciledAgent::from_config`); this call site passes the gateway's
    // SSE observer, the memory session id, and eager MCP (iter-628 wiring,
    // lifted verbatim).
    let mut agent = match ReconciledAgent::from_config(
        &config,
        Some(state.observer.clone()),
        Some(&memory_session_id),
        true,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "Agent initialization failed");
            let err = serde_json::json!({
                "type": "error",
                "message": format!("Failed to initialise agent: {e}"),
                "code": "AGENT_INIT_FAILED"
            });
            let _ = sender.send(Message::Text(err.to_string().into())).await;
            let _ = sender
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Agent initialization failed".into(),
                })))
                .await;
            return;
        }
    };
    agent.core_agent().set_session_id(session_key.clone());
    if !stored_messages.is_empty() {
        // Migrate Loop B history: seed only when the core session store has
        // no transcript yet — otherwise the import would duplicate the turns
        // the facade already rehydrates for this session id.
        agent.seed_history_if_empty(&stored_messages).await;
    }

    // Process the first message if it was not a connect frame
    if let Some(ref text) = first_msg_fallback {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
            if parsed["type"].as_str() == Some("message") {
                let content = parsed["content"].as_str().unwrap_or("").to_string();
                if !content.is_empty() {
                    // Persist user message
                    if let Some(ref backend) = state.session_backend {
                        let user_msg = operant_providers::ChatMessage::user(&content);
                        let _ = backend.append(&session_key, &user_msg);
                    }
                    process_chat_message(
                        &state,
                        &mut agent,
                        &mut sender,
                        &mut receiver,
                        &content,
                        &session_key,
                    )
                    .await;
                }
            } else {
                let unknown_type = parsed["type"].as_str().unwrap_or("unknown");
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!(
                        "Unsupported message type \"{unknown_type}\". Send {{\"type\":\"message\",\"content\":\"your text\"}}"
                    )
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;
            }
        } else {
            let err = serde_json::json!({
                "type": "error",
                "message": "Invalid JSON. Send {\"type\":\"message\",\"content\":\"your text\"}"
            });
            let _ = sender.send(Message::Text(err.to_string().into())).await;
        }
    }

    // Subscribe to the shared broadcast channel so cron/heartbeat events
    // are forwarded to this WebSocket client.
    let mut broadcast_rx = state.event_tx.subscribe();

    loop {
        tokio::select! {
            // ── Client message ────────────────────────────────────────
            client_msg = receiver.next() => {
                let Some(msg) = client_msg else { break };
                let msg = match msg {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => continue,
                };

                // Parse incoming message
                let parsed: serde_json::Value = match serde_json::from_str(&msg) {
                    Ok(v) => v,
                    Err(e) => {
                        let err = serde_json::json!({
                            "type": "error",
                            "message": format!("Invalid JSON: {}", e),
                            "code": "INVALID_JSON"
                        });
                        let _ = sender.send(Message::Text(err.to_string().into())).await;
                        continue;
                    }
                };

                let msg_type = parsed["type"].as_str().unwrap_or("");

                // ── Voice duplex event dispatch (gated by feature flag + runtime config) ──
                #[cfg(feature = "gateway-voice-duplex")]
                {
                    let duplex_enabled = state
                        .config
                        .lock()
                        .channels
                        .voice_duplex
                        .as_ref()
                        .is_some_and(|v| v.enabled);
                    // `continue` runs whenever a voice event parsed (even with
                    // no error frame), so only the outer pair collapses.
                    if duplex_enabled
                        && let Some(voice_event) =
                            crate::voice_duplex::try_parse_voice_event(&msg)
                    {
                        if let Some(error_frame) =
                            crate::voice_duplex::handle_voice_event(voice_event)
                        {
                            let _ = sender
                                .send(Message::Text(error_frame.to_string().into()))
                                .await;
                        }
                        continue;
                    }
                }

                // ── approval_response (operator answered a tool prompt) ──
                if msg_type == "approval_response" {
                    let request_id = parsed["request_id"].as_str().unwrap_or("");
                    let decision_str = parsed["decision"].as_str().unwrap_or("");
                    let decision = match decision_str {
                        "approve" => Some(ChannelApprovalResponse::Approve),
                        "always" => Some(ChannelApprovalResponse::AlwaysApprove),
                        "deny" => Some(ChannelApprovalResponse::Deny),
                        _ => None,
                    };
                    if request_id.is_empty() || decision.is_none() {
                        let err = serde_json::json!({
                            "type": "error",
                            "message": "approval_response requires request_id and decision in {approve,deny,always}",
                            "code": "INVALID_APPROVAL_RESPONSE"
                        });
                        let _ = sender.send(Message::Text(err.to_string().into())).await;
                        continue;
                    }
                    // Facade registry: a live turn's parked approvals resolve
                    // here; a stale id (turn ended) logs and is ignored.
                    if let Some(d) = decision
                        && !agent.approvals().resolve(request_id, d)
                    {
                        debug!(%request_id, "approval_response with no matching pending request");
                    }
                    continue;
                }

                if msg_type != "message" {
                    let err = serde_json::json!({
                        "type": "error",
                        "message": format!(
                            "Unsupported message type \"{msg_type}\". Send {{\"type\":\"message\",\"content\":\"your text\"}}"
                        ),
                        "code": "UNKNOWN_MESSAGE_TYPE"
                    });
                    let _ = sender.send(Message::Text(err.to_string().into())).await;
                    continue;
                }

                let content = parsed["content"].as_str().unwrap_or("").to_string();
                if content.is_empty() {
                    let err = serde_json::json!({
                        "type": "error",
                        "message": "Message content cannot be empty",
                        "code": "EMPTY_CONTENT"
                    });
                    let _ = sender.send(Message::Text(err.to_string().into())).await;
                    continue;
                }

                // Acquire session lock to serialize concurrent turns
                let _session_guard = match state.session_queue.acquire(&session_key).await {
                    Ok(guard) => guard,
                    Err(e) => {
                        let err = serde_json::json!({
                            "type": "error",
                            "message": e.to_string(),
                            "code": "SESSION_BUSY"
                        });
                        let _ = sender.send(Message::Text(err.to_string().into())).await;
                        continue;
                    }
                };

                // Persist user message
                if let Some(ref backend) = state.session_backend {
                    let user_msg = operant_providers::ChatMessage::user(&content);
                    let _ = backend.append(&session_key, &user_msg);
                }

                process_chat_message(&state, &mut agent, &mut sender, &mut receiver, &content, &session_key)
                    .await;
            }

            // ── Broadcast event (cron/heartbeat results) ──────────────
            event = broadcast_rx.recv() => {
                if let Ok(event) = event
                    && event_matches_session(&event, &session_id)
                {
                    let _ = sender.send(Message::Text(event.to_string().into())).await;
                }
            }
        }
    }
}

fn resolve_session_cwd(
    requested_cwd: Option<&str>,
    default_workspace: &Path,
) -> crate::error::Result<PathBuf> {
    let cwd = requested_cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| default_workspace.to_path_buf());
    std::fs::canonicalize(&cwd).map_err(|e| {
        crate::error::Error::message(format!(
            "cwd is not a usable directory ({}): {e}",
            cwd.display()
        ))
    })
}

fn needs_onboarding_ws_error(config: &operant_config::schema::Config) -> Option<serde_json::Value> {
    let model = config.providers.resolve_default_model().unwrap_or_default();
    crate::needs_onboarding_for(&model)?;
    Some(serde_json::json!({
        "type": "error",
        "error": "needs_onboarding",
        "code": "NEEDS_ONBOARDING",
        "message": crate::needs_onboarding_channel_reply(),
        "url": "/onboard",
    }))
}

fn event_matches_session(event: &serde_json::Value, session_id: &str) -> bool {
    match event.get("session_id").and_then(|value| value.as_str()) {
        Some(event_session_id) => event_session_id == session_id,
        None => true,
    }
}

#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
/// Process a single chat message through the agent and send the response.
///
/// Runs through the reconciled facade
/// ([`ReconciledAgent::turn_streamed`], Loop A) so that intermediate text
/// chunks, tool calls, and tool results are forwarded to the WebSocket
/// client in real time.
#[allow(clippy::too_many_arguments)]
async fn process_chat_message(
    state: &AppState,
    agent: &mut ReconciledAgent,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    content: &str,
    session_key: &str,
) {
    use futures_util::StreamExt as _;
    use operant_runtime::agent::TurnEvent;

    let provider_label = state
        .config
        .lock()
        .providers
        .fallback
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    // Set session state to running
    let turn_id = uuid::Uuid::new_v4().to_string();
    if let Some(ref backend) = state.session_backend {
        let _ = backend.set_session_state(session_key, "running", Some(&turn_id));
    }

    // ── Cancellation token lifecycle ─────────────────────────────
    // Create a token before the turn starts so the abort endpoint
    // can cancel it. Remove it after the turn completes regardless
    // of outcome (normal, error, or cancelled).
    let cancel_token = tokio_util::sync::CancellationToken::new();
    {
        state
            .cancel_tokens
            .lock()
            .expect("cancel_tokens lock poisoned")
            .insert(session_key.to_string(), cancel_token.clone());
    }

    // Channel for streaming turn events from the agent.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<TurnEvent>(64);

    // Approval routing for this turn: resolve/deny through the facade's
    // registry (clonable, Arc-backed) so `forward_fut` can drive it while
    // `turn_fut` holds the &mut borrow on the agent.
    let approvals = agent.approvals().clone();

    // Run the streamed turn concurrently: the agent produces events
    // while we forward them to the WebSocket below.  We cannot move
    // `agent` into a spawned task (it is `&mut`), so we use a join
    // instead — `turn_streamed` writes to the channel and we drain it
    // from the other branch.
    let content_owned = content.to_string();
    let session_key_owned = session_key.to_string();
    let turn_fut = async {
        operant_runtime::agent::loop_::scope_session_key(
            Some(session_key_owned),
            agent.turn_streamed(&content_owned, event_tx, Some(cancel_token.clone())),
        )
        .await
    };

    // Drive both futures concurrently: the agent turn produces events
    // and we relay them over WebSocket. Track streamed chunks so we
    // can reconstruct partial content on cancellation.
    //
    // WHY incremental persistence: If the process crashes during streaming,
    // the assistant's response is lost — only the user message survives.
    // We append a placeholder assistant message on the first chunk, then
    // update_last periodically (every 500ms) so partial content survives.
    // The final response overwrites this via update_last on completion.
    let mut accumulated_text = String::new();
    let mut partial_saved = false;
    let mut last_partial_save = std::time::Instant::now();
    let partial_save_interval = std::time::Duration::from_millis(500);

    // Aggregate token usage across all LLM calls in this turn.
    // The agent emits TurnEvent::Usage once per LLM call when the provider
    // surfaces usage; we sum to produce a single done-frame total.
    let mut total_input_tokens: Option<u64> = None;
    let mut total_output_tokens: Option<u64> = None;

    // Routes the concurrent streams the running turn cares about:
    //   1. inbound `approval_response` frames from the WebSocket client
    //      (resolved through the facade's `ReconciledApprovals`),
    //   2. `TurnEvent`s from the facade — including `ApprovalRequest`,
    //      bridged from core's permission pump.
    // Without the multiplexed select, draining only `event_rx` would block
    // the approval back-channel for the whole turn, so a pending tool
    // approval could neither be sent to the client nor answered before the
    // 120s deadline fired.
    let forward_fut = async {
        let mut cancel_drained = false;
        loop {
            tokio::select! {
                biased;
                // ── Cancellation arm ─────────────────────────────
                // When `/abort` cancels the token, auto-deny every parked
                // approval so any in-flight permission oneshot unblocks
                // immediately instead of racing the 120s deadline — a
                // hung approval would otherwise stall the turn's
                // observation of the interrupt flag.
                _ = cancel_token.cancelled(), if !cancel_drained => {
                    approvals.deny_all();
                    cancel_drained = true;
                    // Fall through; the agent loop will now wake from the
                    // approval await, see the cancel token, and propagate
                    // a ToolLoopCancelled error which closes event_rx and
                    // breaks this loop on the `event_rx.recv()` arm below.
                }
                client_msg = receiver.next() => {
                    // On client disconnect, `receiver.next()` returns `None`
                    // (stream end) or `Err(_)` repeatedly. A bare `continue`
                    // hot-loops the select; cancel the turn so `turn_fut`
                    // resolves with `ToolLoopCancelled` and `tokio::join!`
                    // below can return. See #6514.
                    let text = match client_msg {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                            cancel_token.cancel();
                            break;
                        }
                        _ => continue,
                    };
                    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    if parsed["type"].as_str() != Some("approval_response") {
                        // Mid-turn `message` / other frames are ignored. The
                        // outer `select!` will not see them either; we drop
                        // them deliberately rather than queueing.
                        continue;
                    }
                    let request_id = parsed["request_id"].as_str().unwrap_or("");
                    let decision = match parsed["decision"].as_str().unwrap_or("") {
                        "approve" => Some(ChannelApprovalResponse::Approve),
                        "always" => Some(ChannelApprovalResponse::AlwaysApprove),
                        "deny" => Some(ChannelApprovalResponse::Deny),
                        _ => None,
                    };
                    if request_id.is_empty() || decision.is_none() {
                        continue;
                    }
                    if let Some(d) = decision
                        && !approvals.resolve(request_id, d)
                    {
                        debug!(%request_id, "approval_response with no matching pending request (mid-turn)");
                    }
                }
                event_opt = event_rx.recv() => {
                    let Some(event) = event_opt else { break };
                    let ws_msg = match event {
                        TurnEvent::Usage {
                            input_tokens,
                            output_tokens,
                            cost_usd: _,
                        } => {
                            if let Some(it) = input_tokens {
                                total_input_tokens = Some(total_input_tokens.unwrap_or(0) + it);
                            }
                            if let Some(ot) = output_tokens {
                                total_output_tokens = Some(total_output_tokens.unwrap_or(0) + ot);
                            }
                            continue;
                        }
                        TurnEvent::Chunk { ref delta } => {
                            accumulated_text.push_str(delta);
                            // Incremental persistence: save partial content so it
                            // survives a crash. First chunk appends, subsequent
                            // chunks update in-place.
                            if last_partial_save.elapsed() >= partial_save_interval {
                                if let Some(ref backend) = state.session_backend {
                                    let partial = operant_providers::ChatMessage::assistant(
                                        &accumulated_text,
                                    );
                                    if partial_saved {
                                        let _ = backend.update_last(session_key, &partial);
                                    } else {
                                        let _ = backend.append(session_key, &partial);
                                        partial_saved = true;
                                    }
                                }
                                last_partial_save = std::time::Instant::now();
                            }
                            serde_json::json!({ "type": "chunk", "content": delta })
                        }
                        TurnEvent::Thinking { delta } => {
                            serde_json::json!({ "type": "thinking", "content": delta })
                        }
                        TurnEvent::ToolCall { id, name, args } => {
                            serde_json::json!({ "type": "tool_call", "id": id, "name": name, "args": args })
                        }
                        TurnEvent::ToolResult { id, name, output } => {
                            serde_json::json!({ "type": "tool_result", "id": id, "name": name, "output": output })
                        }
                        TurnEvent::ApprovalRequest {
                            request_id,
                            tool_name,
                            arguments_summary,
                            timeout_secs,
                        } => serde_json::json!({
                            "type": "approval_request",
                            "request_id": request_id,
                            "tool": tool_name,
                            "arguments_summary": arguments_summary,
                            "timeout_secs": timeout_secs,
                        }),
                    };
                    let _ = sender.send(Message::Text(ws_msg.to_string().into())).await;
                }
            }
        }
    };

    let (result, ()) = tokio::join!(turn_fut, forward_fut);

    // ── Remove cancel token (turn finished) ──────────────────────
    {
        state
            .cancel_tokens
            .lock()
            .expect("cancel_tokens lock poisoned")
            .remove(session_key);
    }

    // Check if this turn was cancelled. Loop B encoded that as the
    // `ToolLoopCancelled` anyhow string; the facade carries it in the
    // structured exit reason instead (`Interrupted` covers /abort tokens,
    // client disconnects mid-turn, and approval-denial unwinds alike).
    let was_cancelled = matches!(agent.last_exit_reason(), Some(TurnExitReason::Interrupted));

    if was_cancelled {
        // Store partial content with interruption marker so the
        // conversation stays coherent for subsequent turns.
        let truncated = if accumulated_text.is_empty() {
            "[interrupted by user]".to_string()
        } else {
            format!("{accumulated_text}\n\n[interrupted by user]")
        };

        if let Some(ref backend) = state.session_backend {
            let assistant_msg = operant_providers::ChatMessage::assistant(&truncated);
            if partial_saved {
                let _ = backend.update_last(session_key, &assistant_msg);
            } else {
                let _ = backend.append(session_key, &assistant_msg);
            }
        }

        // Inform the client the turn was aborted
        let aborted = serde_json::json!({ "type": "aborted" });
        let _ = sender.send(Message::Text(aborted.to_string().into())).await;

        // Set session state to idle
        if let Some(ref backend) = state.session_backend {
            let _ = backend.set_session_state(session_key, "idle", None);
        }

        // Trace the cancelled turn so the doctor / replay tool sees it
        // alongside successful turns. #6001 follow-through.
        operant_runtime::observability::runtime_trace::record_event(
            "gateway_ws_turn",
            Some("ws"),
            Some(&provider_label),
            Some(&state.model),
            Some(&turn_id),
            Some(false),
            Some("interrupted by user"),
            serde_json::json!({ "session_key": session_key, "cancelled": true }),
        );

        return;
    }

    match result {
        Ok(response) => {
            // Persist final assistant response. If we saved partial content
            // during streaming, update it in-place; otherwise append fresh.
            if let Some(ref backend) = state.session_backend {
                let assistant_msg = operant_providers::ChatMessage::assistant(&response);
                if partial_saved {
                    let _ = backend.update_last(session_key, &assistant_msg);
                } else {
                    let _ = backend.append(session_key, &assistant_msg);
                }
            }

            // Fire-and-forget memory consolidation so facts from WS sessions
            // are extracted to long-term memory (Daily + Core categories).
            if state.auto_save {
                let mem = state.mem.clone();
                let provider = state.provider.clone();
                let model = state.model.clone();
                let user_msg = content.to_string();
                let assistant_resp = response.clone();
                tokio::spawn(async move {
                    if let Err(e) = operant_memory::consolidation::consolidate_turn(
                        provider.as_ref(),
                        &model,
                        mem.as_ref(),
                        &user_msg,
                        &assistant_resp,
                    )
                    .await
                    {
                        tracing::debug!("WS memory consolidation skipped: {e}");
                    }
                });
            }

            // Send chunk_reset so the client clears any accumulated draft
            // before the authoritative done message.
            let reset = serde_json::json!({ "type": "chunk_reset" });
            let _ = sender.send(Message::Text(reset.to_string().into())).await;

            // Compute cost from accumulated tokens + configured pricing,
            // then write the cost record so /api/cost and costs.jsonl reflect
            // this turn. Done before the done frame so cost_usd can ride along.
            let total_tokens = match (total_input_tokens, total_output_tokens) {
                (Some(i), Some(o)) => Some(i.saturating_add(o)),
                (Some(i), None) => Some(i),
                (None, Some(o)) => Some(o),
                (None, None) => None,
            };
            let cost_usd = record_turn_cost(
                state,
                &provider_label,
                &state.model,
                total_input_tokens,
                total_output_tokens,
            );

            let done = serde_json::json!({
                "type": "done",
                "full_response": response,
                "input_tokens": total_input_tokens,
                "output_tokens": total_output_tokens,
                "tokens_used": total_tokens,
                "cost_usd": cost_usd,
                "model": state.model,
                "provider": provider_label,
            });
            let _ = sender.send(Message::Text(done.to_string().into())).await;

            // Set session state to idle
            if let Some(ref backend) = state.session_backend {
                let _ = backend.set_session_state(session_key, "idle", None);
            }

            // Append a runtime-trace.jsonl record so a `operant doctor`
            // sweep sees gateway WS turns alongside channel and CLI turns.
            // Closes the gateway-side trace gap from #6001.
            operant_runtime::observability::runtime_trace::record_event(
                "gateway_ws_turn",
                Some("ws"),
                Some(&provider_label),
                Some(&state.model),
                Some(&turn_id),
                Some(true),
                None,
                serde_json::json!({
                    "session_key": session_key,
                    "input_tokens": total_input_tokens,
                    "output_tokens": total_output_tokens,
                    "tokens_used": total_tokens,
                    "cost_usd": cost_usd,
                }),
            );
        }
        Err(e) => {
            // Set session state to error
            if let Some(ref backend) = state.session_backend {
                let _ = backend.set_session_state(session_key, "error", Some(&turn_id));
            }

            tracing::error!(error = %e, "Agent turn failed");
            let sanitized = operant_providers::sanitize_api_error(&e.to_string());
            let error_code = if sanitized.to_lowercase().contains("api key")
                || sanitized.to_lowercase().contains("authentication")
                || sanitized.to_lowercase().contains("unauthorized")
            {
                "AUTH_ERROR"
            } else if sanitized.to_lowercase().contains("provider")
                || sanitized.to_lowercase().contains("model")
            {
                "PROVIDER_ERROR"
            } else {
                "AGENT_ERROR"
            };
            let err = serde_json::json!({
                "type": "error",
                "message": sanitized,
                "code": error_code,
            });
            let _ = sender.send(Message::Text(err.to_string().into())).await;

            // Broadcast error event
            let _ = state.event_tx.send(serde_json::json!({
                "type": "error",
                "component": "ws_chat",
                "message": sanitized,
            }));

            // Trace the failed turn so the doctor / replay tool sees the
            // failure mode and the turn_id can be cross-referenced with
            // costs.jsonl. #6001 follow-through.
            operant_runtime::observability::runtime_trace::record_event(
                "gateway_ws_turn",
                Some("ws"),
                Some(&provider_label),
                Some(&state.model),
                Some(&turn_id),
                Some(false),
                Some(&sanitized),
                serde_json::json!({ "session_key": session_key, "error_code": error_code }),
            );
        }
    }
}

/// Record token usage for the just-completed turn against the gateway's
/// cost tracker, returning the computed cost in USD (or `None` when no
/// tracker is configured or no usage was reported).
fn record_turn_cost(
    state: &AppState,
    provider_name: &str,
    model: &str,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
) -> Option<f64> {
    let tracker = state.cost_tracker.as_ref()?;
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    let input = input_tokens.unwrap_or(0);
    let output = output_tokens.unwrap_or(0);
    if input == 0 && output == 0 {
        return None;
    }
    let prices = state.config.lock().cost.prices.clone();
    // 3-tier model pricing lookup mirrors record_tool_loop_cost_usage so
    // streaming and non-streaming paths derive identical costs.
    let pricing = prices
        .get(model)
        .or_else(|| prices.get(&format!("{provider_name}/{model}")))
        .or_else(|| {
            model
                .rsplit_once('/')
                .and_then(|(_, suffix)| prices.get(suffix))
        });
    let usage = operant_runtime::cost::types::TokenUsage::new(
        model,
        input,
        output,
        pricing.map_or(0.0, |entry| entry.input),
        pricing.map_or(0.0, |entry| entry.output),
    );
    let cost_usd = usage.cost_usd;
    if let Err(error) = tracker.record_usage(usage) {
        tracing::warn!(
            provider = provider_name,
            model,
            "Failed to record gateway turn cost: {error}"
        );
    }
    Some(cost_usd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use operant_runtime::agent::reconciled::EvolutionConfig;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn extract_ws_token_from_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer zc_test123".parse().unwrap());
        assert_eq!(extract_ws_token(&headers, None), Some("zc_test123"));
    }

    #[test]
    fn extract_ws_token_from_subprotocol() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-websocket-protocol",
            "operant.v1, bearer.zc_sub456".parse().unwrap(),
        );
        assert_eq!(extract_ws_token(&headers, None), Some("zc_sub456"));
    }

    #[test]
    fn extract_ws_token_from_query_param() {
        let headers = HeaderMap::new();
        assert_eq!(
            extract_ws_token(&headers, Some("zc_query789")),
            Some("zc_query789")
        );
    }

    #[test]
    fn extract_ws_token_precedence_header_over_subprotocol() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer zc_header".parse().unwrap());
        headers.insert("sec-websocket-protocol", "bearer.zc_sub".parse().unwrap());
        assert_eq!(
            extract_ws_token(&headers, Some("zc_query")),
            Some("zc_header")
        );
    }

    #[test]
    fn pairing_gate_rejects_unauthenticated_ws_connections() {
        // Mirrors the auth branch of handle_ws_chat / handle_ws_nodes:
        // when pairing is required, an empty/missing token must not pass.
        use operant_config::pairing::PairingGuard;

        let guard = PairingGuard::new(true, &["valid-token".to_string()]);
        assert!(guard.require_pairing());
        assert!(!guard.is_authenticated(""));
        assert!(!guard.is_authenticated("wrong-token"));
        assert!(guard.is_authenticated("valid-token"));

        // A header-extracted token is exactly what the handler passes in.
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer valid-token".parse().unwrap());
        assert_eq!(extract_ws_token(&headers, None), Some("valid-token"));
        assert!(guard.is_authenticated(extract_ws_token(&headers, None).unwrap_or("")));
    }

    #[test]
    fn extract_ws_token_precedence_subprotocol_over_query() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-websocket-protocol", "bearer.zc_sub".parse().unwrap());
        assert_eq!(extract_ws_token(&headers, Some("zc_query")), Some("zc_sub"));
    }

    #[test]
    fn extract_ws_token_returns_none_when_empty() {
        let headers = HeaderMap::new();
        assert_eq!(extract_ws_token(&headers, None), None);
    }

    #[test]
    fn extract_ws_token_skips_empty_header_value() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer ".parse().unwrap());
        assert_eq!(
            extract_ws_token(&headers, Some("zc_fallback")),
            Some("zc_fallback")
        );
    }

    #[test]
    fn extract_ws_token_skips_empty_query_param() {
        let headers = HeaderMap::new();
        assert_eq!(extract_ws_token(&headers, Some("")), None);
    }

    #[test]
    fn extract_ws_token_subprotocol_with_multiple_entries() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-websocket-protocol",
            "operant.v1, bearer.zc_tok, other".parse().unwrap(),
        );
        assert_eq!(extract_ws_token(&headers, None), Some("zc_tok"));
    }

    #[test]
    fn session_scoped_events_only_match_their_session() {
        let target_event = serde_json::json!({
            "type": "message",
            "session_id": "operator-1",
            "content": "deploy finished"
        });
        let other_event = serde_json::json!({
            "type": "message",
            "session_id": "operator-2",
            "content": "different session"
        });
        let global_event = serde_json::json!({
            "type": "cron_result",
            "content": "global notification"
        });

        assert!(event_matches_session(&target_event, "operator-1"));
        assert!(!event_matches_session(&other_event, "operator-1"));
        assert!(event_matches_session(&global_event, "operator-1"));
    }

    #[test]
    fn resolve_session_cwd_uses_requested_cwd() {
        let requested = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();

        let resolved =
            resolve_session_cwd(Some(requested.path().to_str().unwrap()), fallback.path()).unwrap();

        assert_eq!(resolved, requested.path().canonicalize().unwrap());
    }

    #[test]
    fn resolve_session_cwd_uses_default_workspace_without_request() {
        let fallback = tempfile::tempdir().unwrap();

        let resolved = resolve_session_cwd(None, fallback.path()).unwrap();

        assert_eq!(resolved, fallback.path().canonicalize().unwrap());
    }

    #[test]
    fn resolve_session_cwd_rejects_missing_directory() {
        let fallback = tempfile::tempdir().unwrap();
        let missing = fallback.path().join("missing");

        let err = resolve_session_cwd(Some(missing.to_str().unwrap()), fallback.path())
            .expect_err("missing cwd should be rejected");

        assert!(err.to_string().contains("cwd is not a usable directory"));
    }

    #[test]
    fn needs_onboarding_ws_error_points_to_onboard() {
        let config = operant_config::schema::Config::default();
        let frame = needs_onboarding_ws_error(&config)
            .expect("empty model must produce a WS onboarding error");

        assert_eq!(frame["type"], "error");
        assert_eq!(frame["error"], "needs_onboarding");
        assert_eq!(frame["code"], "NEEDS_ONBOARDING");
        assert_eq!(frame["url"], "/onboard");
        let message = frame["message"]
            .as_str()
            .expect("onboarding WS error must include a message");
        assert!(
            !message.starts_with('{') && !message.ends_with('}'),
            "missing Fluent key fallback leaked into WS error message: {message:?}"
        );
        assert!(
            message.to_lowercase().contains("onboarding"),
            "WS onboarding message must explain the setup gap: {message:?}"
        );
    }

    #[test]
    fn needs_onboarding_ws_error_uses_current_configured_model() {
        let mut config = operant_config::schema::Config {
            providers: operant_config::providers::ProvidersConfig {
                fallback: Some("openai".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        config.ensure_fallback_provider().model = Some("openai/gpt-4o-mini".to_string());

        assert!(
            needs_onboarding_ws_error(&config).is_none(),
            "current configured model must allow WebSocket agent construction to continue"
        );
    }

    // Regression for #6514. The mid-turn `client_msg` arm in `forward_fut`
    // must (a) classify stream-end / close / error frames as "client gone"
    // and (b) cancel the turn token so `tokio::join!(turn_fut, forward_fut)`
    // can return — a bare `continue` hot-loops the select forever.
    #[derive(Debug, PartialEq, Eq)]
    enum DisconnectAction {
        Break,
        Continue,
        ProcessText,
    }

    fn classify_client_msg(
        msg: Option<Result<axum::extract::ws::Message, &'static str>>,
    ) -> DisconnectAction {
        use axum::extract::ws::Message;
        match msg {
            Some(Ok(Message::Text(_))) => DisconnectAction::ProcessText,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => DisconnectAction::Break,
            _ => DisconnectAction::Continue,
        }
    }

    #[test]
    fn mid_turn_client_msg_breaks_on_stream_end_close_or_err() {
        use axum::extract::ws::Message;
        assert_eq!(classify_client_msg(None), DisconnectAction::Break);
        assert_eq!(
            classify_client_msg(Some(Ok(Message::Close(None)))),
            DisconnectAction::Break,
        );
        assert_eq!(
            classify_client_msg(Some(Err("io"))),
            DisconnectAction::Break,
        );
        assert_eq!(
            classify_client_msg(Some(Ok(Message::Ping(Default::default())))),
            DisconnectAction::Continue,
        );
        assert_eq!(
            classify_client_msg(Some(Ok(Message::Text("{}".into())))),
            DisconnectAction::ProcessText,
        );
    }

    #[test]
    fn mid_turn_disconnect_cancel_unblocks_joined_turn() {
        let token = tokio_util::sync::CancellationToken::new();
        let clone_for_turn = token.clone();
        assert!(!clone_for_turn.is_cancelled());
        token.cancel();
        assert!(
            clone_for_turn.is_cancelled(),
            "cloned token (held by turn_fut via agent.turn_streamed) must observe cancellation"
        );
    }

    // ── W1.6: facade (Loop A) turn e2e through the exact ws.rs wiring ──
    //
    // Drives `ReconciledAgent::turn_streamed` through a scripted provider
    // and a permission-gated (`bash`) tool, resolving approvals through the
    // same cloned `ReconciledApprovals` registry the `forward_fut` select
    // uses, then cancelling through the same cancel-token + `deny_all()`
    // arm — asserting the SSE/frame-relevant `TurnEvent` sequence and the
    // exit behavior `process_chat_message` keys its `aborted`/`done` frames
    // on (`last_exit_reason`).

    enum WsStep {
        Text(&'static str),
        Tool(&'static str, &'static str),
    }

    struct WsScriptedProvider {
        steps: std::sync::Mutex<Vec<WsStep>>,
        next_id: std::sync::atomic::AtomicUsize,
    }

    impl WsScriptedProvider {
        fn new(steps: Vec<WsStep>) -> Arc<Self> {
            Arc::new(Self {
                steps: std::sync::Mutex::new(steps),
                next_id: std::sync::atomic::AtomicUsize::new(0),
            })
        }
    }

    #[async_trait::async_trait]
    impl operant_providers::Provider for WsScriptedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn chat(
            &self,
            _request: operant_api::provider::ChatRequest<'_>,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<operant_api::provider::ChatResponse> {
            // Tolerant script: extra core iterations (healing, post-denial
            // follow-ups) get the final answer again instead of panicking.
            let step = {
                let mut steps = self.steps.lock().expect("scripted provider steps");
                if steps.is_empty() {
                    WsStep::Text("approved and done")
                } else {
                    steps.remove(0)
                }
            };
            Ok(match step {
                WsStep::Text(t) => operant_api::provider::ChatResponse {
                    text: Some(t.to_string()),
                    tool_calls: Vec::new(),
                    usage: None,
                    reasoning_content: None,
                },
                WsStep::Tool(name, args) => {
                    let id = self
                        .next_id
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    operant_api::provider::ChatResponse {
                        text: Some(String::new()),
                        tool_calls: vec![operant_api::provider::ToolCall {
                            id: format!("call_{id}"),
                            name: name.to_string(),
                            arguments: args.to_string(),
                            extra_content: None,
                        }],
                        usage: None,
                        reasoning_content: None,
                    }
                }
            })
        }

        fn supports_vision(&self) -> bool {
            false
        }

        fn supports_streaming(&self) -> bool {
            false
        }

        fn stream_chat(
            &self,
            _request: operant_api::provider::ChatRequest<'_>,
            _model: &str,
            _temperature: Option<f64>,
            _options: operant_api::provider::StreamOptions,
        ) -> futures_util::stream::BoxStream<
            'static,
            Result<operant_api::provider::StreamEvent, operant_api::provider::StreamError>,
        > {
            unreachable!("non-streaming scripted provider")
        }
    }

    struct WsBashProbe;

    #[async_trait::async_trait]
    impl operant_core::tools::OperantTool for WsBashProbe {
        fn name(&self) -> &str {
            "bash"
        }

        fn description(&self) -> &str {
            "ws test probe"
        }

        fn schema(&self) -> operant_core::schema::ToolSchema {
            operant_core::schema::ToolSchema::new(
                "bash",
                "ws test probe",
                serde_json::json!({"type": "object", "properties": {}}),
            )
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _context: operant_core::tools::ToolContext,
        ) -> operant_core::tools::ToolResult {
            operant_core::tools::ToolResult {
                tool_call_id: String::new(),
                name: "bash".to_string(),
                success: true,
                content: "probe ok".to_string(),
                error: None,
                timed_out: false,
            }
        }
    }

    fn ws_facade_config() -> operant_core::agent::AgentConfig {
        operant_core::agent::AgentConfig {
            model: "demo".to_string(),
            max_iterations: 5,
            tool_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            system_prompt: None,
            stream: false,
            context_window: 8000,
            max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            loop_detection_enabled: true,
            // "smart": bash is permission-gated, so the turn parks on the
            // facade's approval bridge — the exact flow WS clients ride.
            approval_mode: "smart".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            skill_nudge_interval: 0,
            memory_review_interval: 0,
            max_retries: 3,
            tool_search: Default::default(),
        }
    }

    async fn build_ws_script_facade(provider: Arc<WsScriptedProvider>) -> ReconciledAgent {
        let registry = operant_core::tools::ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(WsBashProbe)
            .await
            .expect("register bash probe");
        let tmp = tempfile::tempdir().expect("tempdir");
        let database = Arc::new(
            operant_core::database::Database::init(tmp.path().join("ws-facade.db"))
                .expect("database init"),
        );
        ReconciledAgent::new(
            ws_facade_config(),
            provider,
            "scripted",
            registry,
            database,
            None,
            Arc::new(operant_runtime::observability::NoopObserver),
            None,
            EvolutionConfig::default(),
            None,
            std::sync::Arc::new(parking_lot::Mutex::new(None)),
            operant_core::interrupt::InterruptFlag::new(),
        )
    }

    #[tokio::test]
    async fn facade_ws_turn_toolcall_approval_roundtrip_and_cancel_exit() {
        // ── Turn 1: tool call → approval round-trip → answer ──────────
        let provider = WsScriptedProvider::new(vec![
            WsStep::Tool("bash", "{}"),
            WsStep::Text("approved and done"),
            // Turn 2's script: park on the approval gate again.
            WsStep::Tool("bash", "{}"),
            WsStep::Text("after the abort"),
        ]);
        let mut agent = build_ws_script_facade(Arc::clone(&provider)).await;
        let approvals = agent.approvals().clone();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<operant_runtime::agent::TurnEvent>(64);
        let forwarder = {
            let approvals = approvals;
            tokio::spawn(async move {
                let mut saw_tool_call = false;
                let mut saw_tool_result = false;
                let mut final_text = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        operant_runtime::agent::TurnEvent::ToolCall { name, .. } => {
                            assert_eq!(name, "bash");
                            saw_tool_call = true;
                        }
                        operant_runtime::agent::TurnEvent::ApprovalRequest {
                            request_id,
                            tool_name,
                            timeout_secs,
                            ..
                        } => {
                            assert_eq!(tool_name, "bash");
                            assert_eq!(timeout_secs, 120);
                            // The `forward_fut` client_msg arm routing:
                            assert!(
                                approvals.resolve(&request_id, ChannelApprovalResponse::Approve),
                                "live approval must resolve"
                            );
                        }
                        operant_runtime::agent::TurnEvent::ToolResult { name, output, .. } => {
                            assert_eq!(name, "bash");
                            assert_eq!(output, "probe ok");
                            saw_tool_result = true;
                        }
                        operant_runtime::agent::TurnEvent::Chunk { delta } => {
                            final_text.push_str(&delta)
                        }
                        _ => {}
                    }
                }
                (saw_tool_call, saw_tool_result, final_text)
            })
        };
        let response = agent
            .turn_streamed("run the probe", tx, None)
            .await
            .expect("turn 1 ok");
        let (saw_tool_call, saw_tool_result, final_text) =
            forwarder.await.expect("turn 1 forwarder");
        assert_eq!(response, "approved and done");
        assert_eq!(final_text, "approved and done");
        assert!(
            saw_tool_call,
            "tool_call frame precursor must precede approval"
        );
        assert!(saw_tool_result);
        assert_eq!(
            agent.last_exit_reason(),
            Some(TurnExitReason::TextResponse),
            "approved turn completes normally — done-frame path"
        );

        // ── Turn 2: cancel while parked on the approval gate ─────────
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<operant_runtime::agent::TurnEvent>(64);
        let forwarder = {
            let approvals2 = agent.approvals().clone();
            let cancel_token = cancel_token.clone();
            tokio::spawn(async move {
                let mut saw_approval_request = false;
                let mut saw_denial_surface = false;
                while let Some(event) = rx.recv().await {
                    match event {
                        operant_runtime::agent::TurnEvent::ApprovalRequest { .. } => {
                            saw_approval_request = true;
                            // The `forward_fut` cancel arm, verbatim: deny
                            // every parked approval, then trip the token.
                            approvals2.deny_all();
                            cancel_token.cancel();
                        }
                        // A denied tool surfaces as a ToolResult carrying the
                        // refusal (success=false → error text), mirroring the
                        // tool_result frame a WS client sees on abort.
                        operant_runtime::agent::TurnEvent::ToolResult { output, .. } => {
                            if output.contains("Permission denied by user") {
                                saw_denial_surface = true;
                            }
                        }
                        _ => {}
                    }
                }
                (saw_approval_request, saw_denial_surface)
            })
        };
        let started = std::time::Instant::now();
        let _result2 = agent
            .turn_streamed("run it again", tx, Some(cancel_token))
            .await;
        let (saw_approval_request, _saw_denial_surface) =
            forwarder.await.expect("turn 2 forwarder");
        assert!(
            saw_approval_request,
            "turn 2 must park on the approval gate before cancellation"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "deny_all() must unblock the parked approval long before core's \
             120s deadline (elapsed {:?})",
            started.elapsed()
        );
        // The exact expression `process_chat_message` keys the aborted frame
        // on after the W1.6 switch:
        assert!(
            matches!(agent.last_exit_reason(), Some(TurnExitReason::Interrupted)),
            "cancelled turn must map to Interrupted, not an error frame"
        );
    }
}
