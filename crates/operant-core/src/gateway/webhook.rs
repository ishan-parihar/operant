//! `webhook` — extracted verbatim from gateway/mod.rs.

use crate::error::{Error, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::*;

/// Webhook adapter — HTTP server that receives webhook POSTs and forwards
/// them as IncomingMessages. Supports HMAC signature validation and
/// route-based webhook handling.
///
/// Routes:
///   POST /webhook/{route}  — receives a JSON payload, validates HMAC
///   signature (if configured), and forwards as an IncomingMessage.
///   POST /webhook/notion   — with [holon] enabled: hands the event to the
///   kernel's notion adapter in line (`apply`), so nothing resident stays
///   between Notion and the organization (boundary doc R4; the poll remains
///   the fallback).
///   GET  /health           — health check endpoint.
pub struct WebhookAdapter {
    enabled: bool,
    listen_addr: String,
    secret: Option<String>,
    /// The Holon hook: where the instance lives, how to reach its notion
    /// adapter script. None = the notion route is an ordinary webhook.
    holon: Option<HolonHook>,
}

/// What the notion arm needs to hand one event to the kernel's adapter.
// (plan-04 note) Debug completed in-flight by the concurrent agent: the
// peer's gateway/types.rs derive(Debug) on GatewayConfig requires it.
// Field values are paths + a string — no secrets, safe to derive.
#[derive(Clone, Debug)]
pub struct HolonHook {
    /// $HOLON_HOME of the instance the event belongs to.
    pub home: std::path::PathBuf,
    /// The adapter script (`adapters/notion/holon_notion.py`), run with the
    /// interpreter on PATH.
    pub script: std::path::PathBuf,
    /// Extra env for the run (the kernel exports its own through --home).
    pub python_path: Option<String>,
    /// The instance's SECRET webhook path (O-08): `holon_notion.py
    /// webhook-path` — a random, unguessable route the arm mounts at
    /// instead of a literal "notion". The adapter's token rotation is
    /// only valid behind this path (kernel H-05), so the arm also passes
    /// `--path-verified`.
    pub webhook_path: String,
}

/// O-08: at most two notion-adapter processes at a time — every event
/// otherwise spawns an unbounded `python3`, a cheap denial of service.
static NOTION_SEMAPHORE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

impl WebhookAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            listen_addr: "0.0.0.0:8080".to_string(),
            secret: None,
            holon: None,
        }
    }

    pub fn with_addr(mut self, addr: String) -> Self {
        self.listen_addr = addr;
        self
    }

    pub fn with_secret(mut self, secret: Option<String>) -> Self {
        self.secret = secret;
        self
    }

    /// Mount the Holon notion arm (docs/agentic-os phase F / H8).
    pub fn with_holon(mut self, hook: HolonHook) -> Self {
        self.holon = Some(hook);
        self
    }

    /// The gateway factory passes the hook through the shared GatewayConfig;
    /// naming it for that call site reads better than `map` at a distance.
    pub fn with_holon_opt(mut self, hook: Option<HolonHook>) -> Self {
        self.holon = hook;
        self
    }
}

#[async_trait]
impl PlatformAdapter for WebhookAdapter {
    fn name(&self) -> &str {
        "webhook"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!(addr = %self.listen_addr, "Webhook adapter starting");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Webhook adapter stopped");
        Ok(())
    }

    async fn send_message(&self, _message: OutgoingMessage) -> Result<()> {
        // Webhook adapter is inbound-only; sending messages back is done
        // via the deliver mechanism in the gateway runner, not here.
        Ok(())
    }

    async fn handle_update(&self, _update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        Ok(None)
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "webhook",
            "enabled": self.enabled,
            "listen_addr": self.listen_addr,
            "hmac_secret_configured": self.secret.is_some(),
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "NOTION_SEMAPHORE is a static that is never closed, so acquire cannot fail"
    )]
    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        use axum::Router;
        use axum::extract::{Path, State};
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::routing::get;

        let addr: std::net::SocketAddr = self
            .listen_addr
            .parse()
            .map_err(|e| Error::Config(format!("Invalid webhook listen addr: {e}")))?;

        let secret = self.secret.clone();
        let holon = self.holon.clone();
        let tx = message_tx.clone();

        // Build the axum router
        let app = Router::new()
            // Red-team F-4 (2026-10-15): an explicit body cap — a webhook
            // route fronts the public internet and pipes into a kernel
            // adapter; axum's default 2MB boundary left a boundary-size
            // request able to kill the serve task. 1MB with a clean 413;
            // the notion adapter's real events are a few KB.
            .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
            .route("/health", get(|| async { "ok" }))
            .route(
                "/webhook/{route}",
                // GET handler: WhatsApp/Meta webhook verification handshake.
                // Meta sends `GET /webhook/{route}?hub.mode=subscribe&hub.verify_token=<token>&hub.challenge=<int>`
                // when you first register the webhook URL in the Meta app dashboard.
                // We respond with the challenge value iff the verify_token matches
                // our secret. (iter-131 — closes the ponytail-audit gap "WhatsApp
                // webhook handshake (hub.mode=subscribe) not implemented → no
                // inbound from Meta".)
                get(
                    move |Path(_route): Path<String>,
                          State((_, secret, _)): State<(
                        mpsc::UnboundedSender<IncomingMessage>,
                        Option<String>,
                        Option<HolonHook>,
                    )>,
                          axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                        let mode = params.get("hub.mode").map(|s| s.as_str()).unwrap_or("");
                        let verify_token = params.get("hub.verify_token").map(|s| s.as_str()).unwrap_or("");
                        let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

                        if mode == "subscribe" {
                            // Verify the token matches our secret (if one is configured).
                            let token_ok = match &secret {
                                Some(expected) => verify_token == expected.as_str(),
                                None => true,
                            };
                            if token_ok {
                                debug!("WhatsApp/Meta webhook verification: challenge accepted");
                                return (axum::http::StatusCode::OK, challenge).into_response();
                            } else {
                                warn!("WhatsApp/Meta webhook verification: verify_token mismatch");
                                return (axum::http::StatusCode::FORBIDDEN, "verify_token mismatch").into_response();
                            }
                        }
                        (axum::http::StatusCode::BAD_REQUEST, "Expected hub.mode=subscribe").into_response()
                    },
                )
                .post(
                    move |Path(route): Path<String>,
                          headers: HeaderMap,
                          State((tx, secret, holon)): State<(
                        mpsc::UnboundedSender<IncomingMessage>,
                        Option<String>,
                        Option<HolonHook>,
                    )>,
                          body: axum::body::Bytes| async move {
                        // Red-team F-4 (2026-10-15): a Content-Length over 1MB is
                        // refused BEFORE the body streams — an oversized POST
                        // was observed killing the whole gateway silently
                        // (systemd journal: no panic, no signal, process gone).
                        // The header is always present for sized bodies; a
                        // chunked stream without one still hits the
                        // DefaultBodyLimit layer above.
                        if let Some(len) = headers
                            .get(axum::http::header::CONTENT_LENGTH)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            && len > 1024 * 1024
                        {
                            warn!(route = %route, len, "webhook body over 1MB refused");
                            return (
                                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                                "body over 1MB",
                            )
                                .into_response();
                        }
                        // ────────────────────────────────────────────────────────
                        // URL Verification Handshakes (iter-131)
                        // ────────────────────────────────────────────────────────
                        // Several platforms require a one-time handshake when you
                        // first register the webhook URL with them:
                        //
                        //   • Slack Events API — POSTs `{"type":"url_verification",
                        //     "challenge":"<token>"}` and expects the same token
                        //     back in the response body.
                        //
                        //   • WhatsApp Cloud API (Meta) — GETs the webhook with
                        //     `hub.mode=subscribe` + `hub.verify_token=<token>` +
                        //     `hub.challenge=<int>`. Expects the challenge value
                        //     back in the response body.
                        //
                        //   • Meta Webhooks (Instagram/Messenger) — same as
                        //     WhatsApp (the Meta Webhooks product is shared).
                        //
                        // These handshakes have NO HMAC signature (they happen
                        // before the platform starts signing events), so they
                        // must be handled BEFORE the signature check below.
                        // ────────────────────────────────────────────────────────

                        // Try parsing the body as JSON first (Slack url_verification
                        // is JSON; WhatsApp handshake is GET with query params).
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Slack url_verification
                            if v.get("type").and_then(|t| t.as_str()) == Some("url_verification")
                                && let Some(challenge) = v.get("challenge").and_then(|c| c.as_str()) {
                                    debug!("Slack url_verification challenge — responding with challenge token");
                                    return (
                                        axum::http::StatusCode::OK,
                                        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                                        challenge.to_string(),
                                    ).into_response();
                                }
                        }

                        // ── Holon notion arm (docs/agentic-os phase F / H8) ──
                        // One event handed to the kernel's adapter in line;
                        // nothing resident stays between Notion and the
                        // organization. Verification (signature + the
                        // platform's own handshake) is the adapter's — it
                        // carries the notion secret — so this arm runs BEFORE
                        // our generic HMAC check, like the handshakes above.
                        if let Some(hook) = holon.clone()
                            && !hook.webhook_path.is_empty()
                            && route == hook.webhook_path.trim_start_matches('/')
                        {
                            let sig = headers
                                .get("x-notion-signature")
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_string);
                            let _permit = NOTION_SEMAPHORE
                                .acquire()
                                .await
                                .expect("semaphore never closes");
                            let mut cmd = tokio::process::Command::new("python3");
                            cmd.arg(&hook.script)
                                .arg("--holon-home").arg(&hook.home)
                                .arg("apply")
                                // H-05: the adapter rotates the verification
                                // token ONLY behind the secret path this arm
                                // mounts at — say so, or every apply is
                                // refused with "token refused: apply is not
                                // mounted behind the secret path".
                                .arg("--path-verified")
                                .arg("--signature").arg(sig.unwrap_or_default())
                                .stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::piped());
                            if let Some(pp) = hook.python_path {
                                cmd.env("PYTHONPATH", pp);
                            }
                            match cmd.spawn() {
                                Ok(mut child) => {
                                    use tokio::io::AsyncWriteExt;
                                    if let Some(mut stdin) = child.stdin.take()
                                        && let Err(e) = stdin.write_all(&body).await
                                    {
                                        warn!(error = %e, "notion apply: writing body failed");
                                    }
                                    drop(child.stdin.take());
                                    match child.wait_with_output().await {
                                        Ok(out) => {
                                            // the adapter prints "<http status> <message>"
                                            // and exits 1 for any >= 400 — the status is
                                            // the stdout's leading integer, not the exit code
                                            let text =
                                                String::from_utf8_lossy(&out.stdout).to_string();
                                            let code = text
                                                .split_whitespace()
                                                .next()
                                                .and_then(|s| s.parse::<u16>().ok())
                                                .filter(|c| (200..600).contains(c))
                                                .unwrap_or(500);
                                            return (
                                                axum::http::StatusCode::from_u16(code)
                                                    .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
                                                text,
                                            ).into_response();
                                        }
                                        Err(e) => {
                                            warn!(error = %e, "notion apply: adapter run failed");
                                            return (axum::http::StatusCode::BAD_GATEWAY,
                                                    format!("notion apply failed: {e}"))
                                                .into_response();
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, "notion apply: spawn failed");
                                    return (axum::http::StatusCode::BAD_GATEWAY,
                                            format!("notion apply spawn failed: {e}"))
                                        .into_response();
                                }
                            }
                        }

                        // Validate HMAC signature if secret is configured.
                        // Uses standard HMAC-SHA256(secret, body) — not the
                        // old non-standard SHA256(route+secret). Supports
                        // multiple signature header names for interop:
                        // x-webhook-signature (custom), x-hub-signature-256
                        // (GitHub), Stripe-Signature (Stripe).
                        // (iter-101 — closes Bug #9 from iter-98 audit.)
                        if let Some(ref sec) = secret {
                            // Slack-specific signature verification: Slack uses
                            // `X-Slack-Signature` (HMAC-SHA256 hex of
                            // "v0:<X-Slack-Request-Timestamp>:<body>") +
                            // `X-Slack-Request-Timestamp`. We special-case
                            // this because Slack's format is non-standard.
                            // (iter-125 — closes the ponytail-audit security
                            // bug "Slack signing_secret collected but HMAC
                            // verification never performed".)
                            let slack_sig = headers
                                .get("x-slack-signature")
                                .and_then(|v| v.to_str().ok());
                            let slack_ts = headers
                                .get("x-slack-request-timestamp")
                                .and_then(|v| v.to_str().ok());

                            if let (Some(sig), Some(ts)) = (slack_sig, slack_ts) {
                                // Replay protection: reject requests older
                                // than 5 minutes (Slack's own recommendation).
                                if let Ok(ts_secs) = ts.parse::<i64>() {
                                    let now_secs = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs() as i64)
                                        .unwrap_or(0);
                                    if (now_secs - ts_secs).abs() > 300 {
                                        return (
                                            axum::http::StatusCode::UNAUTHORIZED,
                                            "Stale Slack request (replay-protected)",
                                        )
                                            .into_response();
                                    }
                                }
                                // Compute HMAC-SHA256(signing_secret, "v0:<ts>:<body>").
                                use hmac::{Hmac, Mac};
                                use sha2::Sha256;
                                type HmacSha256 = Hmac<Sha256>;
                                let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
                                    Ok(m) => m,
                                    Err(_) => {
                                        return (
                                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            "HMAC key error",
                                        )
                                            .into_response();
                                    }
                                };
                                let basestring = format!("v0:{}:", ts);
                                mac.update(basestring.as_bytes());
                                mac.update(&body);
                                let expected = mac.finalize().into_bytes();
                                let expected_hex = format!("v0={}", hex::encode(expected));
                                if !constant_time_eq::constant_time_eq(
                                    sig.as_bytes(),
                                    expected_hex.as_bytes(),
                                ) {
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "Invalid Slack signature",
                                    )
                                        .into_response();
                                }
                            } else {
                                // Fall back to the standard HMAC verification
                                // used by GitHub / Stripe / generic webhooks.
                                let sig = headers
                                    .get("x-webhook-signature")
                                    .or_else(|| headers.get("x-hub-signature-256"))
                                    .or_else(|| headers.get("stripe-signature"))
                                    .and_then(|v| v.to_str().ok());

                                if let Some(sig) = sig {
                                    // Strip "sha256=" prefix if present (GitHub/Stripe format).
                                    let sig_hex = sig.strip_prefix("sha256=").unwrap_or(sig);
                                    // Compute HMAC-SHA256(secret, body).
                                    use hmac::{Hmac, Mac};
                                    use sha2::Sha256;
                                    type HmacSha256 = Hmac<Sha256>;
                                    let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
                                        Ok(m) => m,
                                        Err(_) => {
                                            return (
                                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                                "HMAC key error",
                                            )
                                                .into_response();
                                        }
                                    };
                                    mac.update(&body);
                                    let expected = mac.finalize().into_bytes();
                                    let expected_hex = hex::encode(expected);
                                    // Constant-time comparison via the
                                    // constant_time_eq crate.
                                    if !constant_time_eq::constant_time_eq(sig_hex.as_bytes(), expected_hex.as_bytes()) {
                                        return (
                                            axum::http::StatusCode::UNAUTHORIZED,
                                            "Invalid signature",
                                        )
                                            .into_response();
                                    }
                                } else {
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "Missing signature header (x-slack-signature / x-webhook-signature / x-hub-signature-256 / stripe-signature)",
                                    )
                                        .into_response();
                                }
                            }
                        }

                        // Parse the body as the message content. Try JSON
                        // first (most webhooks send JSON); fall back to UTF-8
                        // text. The route name becomes the channel_id.
                        // (iter-101 — previously the body was thrown away and
                        // the agent received "Webhook received on /{route}".)
                        //
                        // iter-131: Slack Events API forwarding. Slack sends
                        // event callbacks as `{"type":"event_callback","event":
                        // {"type":"message","text":"...","user":"...","channel":"..."}}`.
                        // We detect this shape and forward it as a real
                        // IncomingMessage with platform="slack" instead of the
                        // generic "webhook" platform.
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Slack event_callback forwarding.
                            if v.get("type").and_then(|t| t.as_str()) == Some("event_callback")
                                && let Some(event) = v.get("event")
                                    && event.get("type").and_then(|t| t.as_str()) == Some("message") {
                                        // Skip bot messages (prevents echo loops).
                                        let is_bot = event
                                            .get("bot_id")
                                            .or_else(|| event.get("bot_profile"))
                                            .is_some();
                                        if !is_bot {
                                            let content = event.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                            if !content.is_empty() {
                                                let channel = event.get("channel").and_then(|c| c.as_str()).unwrap_or("").to_string();
                                                let user = event.get("user").and_then(|u| u.as_str()).unwrap_or("slack").to_string();
                                                let ts = event.get("ts").and_then(|t| t.as_str())
                                                    .and_then(|s| s.split('.').next().and_then(|n| n.parse::<i64>().ok()))
                                                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                                                let slack_msg = IncomingMessage {
                                                    platform: "slack".to_string(),
                                                    channel_id: channel,
                                                    user_id: user.clone(),
                                                    username: user,
                                                    content,
                                                    is_group_chat: true,  // Slack events come from channels by default
                                                    is_channel_post: false,
                                                    timestamp: ts,
                                                    thread_id: event.get("thread_ts")
                                                        .and_then(|t| t.as_str())
                                                        .and_then(|s| s.split('.').next().and_then(|n| n.parse::<i64>().ok())),
                                                    raw: v.clone(),
                                                    media_urls: Vec::new(),
                                                };
                                                let _ = tx.send(slack_msg);
                                            }
                                        }
                                        // Always 200 OK to Slack — otherwise it retries.
                                        return (axum::http::StatusCode::OK, "ok").into_response();
                                    }

                            // WhatsApp Cloud API event forwarding. Meta sends
                            // `{"entry":[{"changes":[{"value":{"messages":[{"from":"...","text":{"body":"..."}}]}}]}]}`.
                            if let Some(entry) = v.get("entry").and_then(|e| e.as_array()).and_then(|a| a.first())
                                && let Some(change) = entry.get("changes").and_then(|c| c.as_array()).and_then(|a| a.first())
                                    && let Some(messages) = change.get("value").and_then(|val| val.get("messages")).and_then(|m| m.as_array()) {
                                        for msg in messages {
                                            let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or("").to_string();
                                            let text = msg.get("text").and_then(|t| t.get("body")).and_then(|b| b.as_str()).unwrap_or("").to_string();
                                            if !text.is_empty() {
                                                let wa_msg = IncomingMessage {
                                                    platform: "whatsapp".to_string(),
                                                    channel_id: from.clone(),
                                                    user_id: from.clone(),
                                                    username: change.get("value").and_then(|val| val.get("contacts")).and_then(|c| c.as_array()).and_then(|a| a.first()).and_then(|c| c.get("profile")).and_then(|p| p.get("name")).and_then(|n| n.as_str()).unwrap_or(&from).to_string(),
                                                    content: text,
                                                    is_group_chat: false,
                                                    is_channel_post: false,
                                                    timestamp: msg.get("timestamp").and_then(|t| t.as_str()).and_then(|s| s.parse::<i64>().ok()).unwrap_or_else(|| chrono::Utc::now().timestamp()),
                                                    thread_id: None,
                                                    media_urls: Vec::new(),
                                                    raw: msg.clone(),
                                                };
                                                let _ = tx.send(wa_msg);
                                            }
                                        }
                                        return (axum::http::StatusCode::OK, "ok").into_response();
                                    }
                        }

                        let content = if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Try common fields: text, message, content, body, data.
                            // If none match, pretty-print the whole JSON.
                            v.get("text")
                                .or_else(|| v.get("message"))
                                .or_else(|| v.get("content"))
                                .or_else(|| v.get("body"))
                                .or_else(|| v.get("data"))
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default())
                        } else {
                            // Not JSON — use raw UTF-8 text.
                            String::from_utf8_lossy(&body).to_string()
                        };

                        let msg = IncomingMessage {
                            platform: "webhook".to_string(),
                            channel_id: format!("webhook:{route}"),
                            user_id: "webhook".to_string(),
                            username: "Webhook".to_string(),
                            content,
                            is_group_chat: false,
                            is_channel_post: false,
                            timestamp: chrono::Utc::now().timestamp(),
                            thread_id: None,
                            media_urls: Vec::new(),
                            raw: serde_json::from_slice(&body).unwrap_or(serde_json::json!({"route": route})),
                        };

                        if tx.send(msg).is_err() {
                            return (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "Channel closed",
                            )
                                .into_response();
                        }

                        (axum::http::StatusCode::OK, "accepted").into_response()
                    },
                ),
            )
            .with_state((tx, secret, holon));

        // Spawn the HTTP server
        // Bind before spawning: a port conflict must come back as this method's
        // error (the caller logs it), not as a panic inside a detached task that
        // leaves the adapter reporting itself as started.
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tokio::spawn(async move {
            info!("Webhook HTTP server starting");
            if let Err(e) = axum::serve(listener, app).await {
                error!(error = %e, "Webhook HTTP server error");
            }
        });

        info!("Webhook adapter started with HTTP server");
        Ok(())
    }

    async fn send_message_to_channel(
        &self,
        _channel_id: &str,
        _message: &OutgoingMessage,
    ) -> Result<String> {
        // Webhook adapter is inbound-only
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_with_channel_returns_bind_error_when_port_is_taken() {
        let Ok(occupied) = std::net::TcpListener::bind("127.0.0.1:0") else {
            panic!("an ephemeral loopback port should bind");
        };
        let Ok(addr) = occupied.local_addr() else {
            panic!("a bound listener has a local address");
        };
        let adapter = WebhookAdapter::new(true).with_addr(addr.to_string());
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(adapter.start_with_channel(tx).await.is_err());
    }
}
