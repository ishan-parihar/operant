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
///   POST /webhook/notion   — with `[holon]` enabled: hands the event to the
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

        // An adapter with no secret accepts ANY POST body and forwards it
        // into the agent as a message. That is a legitimate configuration
        // for a receiver on a private network, so it is NOT refused — but
        // it is the difference between "a webhook endpoint" and "an
        // unauthenticated command channel pointed at the agent", and the
        // default listen address is 0.0.0.0, which is the public
        // internet. Say so, loudly, at startup: the operator cannot
        // otherwise tell an unauthenticated receiver from a verified one.
        //
        // (iter-1009 — the HMAC check is `if let Some(ref sec) = secret`,
        // so with no secret the whole verification block is skipped and the
        // request is accepted.)
        if secret.is_none() {
            warn!(
                addr = %addr,
                "Webhook adapter is running WITHOUT a signing secret: every POST \
                 to /webhook/<route> is accepted unverified and forwarded into \
                 the agent as a message. Set [gateway] webhooks_secret to enable \
                 HMAC-SHA256 verification, or bind the adapter to a private \
                 address. (A route claimed by a webhook subscription that has \
                 its own secret IS verified against that secret.)"
            );
        }

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

                        // (iter-1132) The route may be claimed by a webhook
                        // subscription. A subscription with its own secret is
                        // verified against THAT secret, so two routes can use
                        // different ones; without one, the adapter's global
                        // `[gateway] webhooks_secret` applies.
                        let subscription =
                            super::webhook_subscriptions::subscription_for_route(&route);
                        let effective_secret: Option<String> = subscription
                            .as_ref()
                            .and_then(|s| s.secret.clone())
                            .or_else(|| secret.clone());
                        // A signature that verified is what makes the event a
                        // machine event rather than an anonymous request.
                        let mut authenticated = false;

                        // Validate HMAC signature if secret is configured.
                        // Uses standard HMAC-SHA256(secret, body) — not the
                        // old non-standard SHA256(route+secret). Supports
                        // multiple signature header names for interop:
                        // x-webhook-signature (custom), x-hub-signature-256
                        // (GitHub), Stripe-Signature (Stripe).
                        // (iter-101 — closes Bug #9 from iter-98 audit.)
                        if let Some(ref sec) = effective_secret {
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
                            // The signature matched (both branches above
                            // return on a mismatch).
                            authenticated = true;
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
                                                    deliver_to: None,
                                                    deliver_only: false,
                                                    authenticated,
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
                                                    deliver_to: None,
                                                    deliver_only: false,
                                                    authenticated,
                                                    raw: msg.clone(),
                                                };
                                                let _ = tx.send(wa_msg);
                                            }
                                        }
                                        return (axum::http::StatusCode::OK, "ok").into_response();
                                    }
                        }

                        let (content, event_name) = if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // The event NAME: the payload names it, and a
                            // payload that names none is a "message" event —
                            // which is what the CLI subscribes to by default.
                            let name = v
                                .get("event")
                                .or_else(|| v.get("type"))
                                .and_then(|n| n.as_str())
                                .unwrap_or("message")
                                .to_string();
                            // Try common fields: text, message, content, body, data.
                            // If none match, pretty-print the whole JSON.
                            let text = v.get("text")
                                .or_else(|| v.get("message"))
                                .or_else(|| v.get("content"))
                                .or_else(|| v.get("body"))
                                .or_else(|| v.get("data"))
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default());
                            (text, name)
                        } else {
                            // Not JSON — use raw UTF-8 text.
                            (String::from_utf8_lossy(&body).to_string(), "message".to_string())
                        };

                        // (iter-1132) A subscription that does not list this
                        // event does not apply: the event is handled exactly
                        // as before. (The route's subscription still governs
                        // the SIGNATURE — that was decided before the body
                        // was read.)
                        let subscription = subscription
                            .as_ref()
                            .filter(|sub| sub.accepts_event(&event_name))
                            .cloned();

                        // (iter-1132) The subscription that claims this route
                        // shapes the event and names where the answer goes.
                        // Without one, the event is handled exactly as before.
                        let (content, deliver_to, deliver_only) = match &subscription {
                            // deliver-only: the EVENT is delivered, so the
                            // agent's prompt has no reader — deliver the body.
                            Some(sub) if sub.deliver_only => {
                                (content, sub.delivery_target(), true)
                            }
                            Some(sub) => (
                                sub.compose_event_prompt(&content),
                                sub.delivery_target(),
                                false,
                            ),
                            None => (content, None, false),
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
                            deliver_to,
                            deliver_only,
                            authenticated,
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

    /// Start a webhook adapter on an ephemeral port and return the port.
    ///
    /// The RECEIVER is kept alive: the handler forwards every accepted
    /// message into the channel, and a dropped receiver makes the send
    /// fail — which is a 500 AFTER the signature check, and would make
    /// the test measure the wrong thing.
    /// Start the adapter on an ephemeral port.
    ///
    /// The port is chosen by binding a probe socket on port 0 and
    /// DROPPING it — and that is a TOCTOU race: between the drop and the
    /// adapter's own bind, another test's ephemeral bind can take the
    /// port, and the adapter then fails with `AddrInUse`. With the
    /// suite's 16 test threads, that is roughly one full-suite run in
    /// ten — and it reads as a product failure ("the webhook adapter
    /// cannot start") when it is a test-harness race.
    ///
    /// So: try a few ports. The retry is on the HARNESS side only — the
    /// adapter's behaviour is not changed.
    async fn start_on_ephemeral(
        secret: Option<&str>,
    ) -> (u16, mpsc::UnboundedReceiver<IncomingMessage>) {
        let mut last_error = None;
        for _ in 0..5 {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
            let port = probe.local_addr().expect("the bound address").port();
            drop(probe);
            let mut adapter = WebhookAdapter::new(true).with_addr(format!("127.0.0.1:{port}"));
            adapter = adapter.with_secret(secret.map(str::to_string));
            let (tx, rx) = mpsc::unbounded_channel();
            match adapter.start_with_channel(tx).await {
                Ok(()) => return (port, rx),
                Err(e) => {
                    let in_use = e.to_string().contains("Address already in use");
                    last_error = Some(e);
                    if !in_use {
                        break;
                    }
                }
            }
        }
        panic!("the adapter starts: {last_error:?}");
    }

    /// POST a body and return the status code.
    async fn post(port: u16, path: &str, headers: &[(&str, &str)]) -> u16 {
        post_body(port, path, headers, "{}").await
    }

    /// POST a specific body and return the status code.
    async fn post_body(port: u16, path: &str, headers: &[(&str, &str)], body: &str) -> u16 {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read");
        response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0)
    }

    /// With a secret configured, an UNSIGNED request is refused.
    ///
    /// The HMAC check is `if let Some(ref sec) = secret`, so with a secret
    /// the whole verification block runs and an unsigned body never
    /// reaches the agent. Without this test the block could be deleted and
    /// nothing would notice.
    #[tokio::test]
    async fn a_signed_adapter_refuses_an_unsigned_request() {
        let (port, _rx) = start_on_ephemeral(Some("shared-secret")).await;
        let status = post(port, "/webhook/github", &[]).await;
        assert_eq!(
            status, 401,
            "an unsigned request must be refused when a secret is configured"
        );
    }

    /// With NO secret, the request is accepted — and that is a WARNING,
    /// not a silent default.
    ///
    /// The adapter with no secret is a legitimate private-network
    /// configuration, so the behaviour is not changed. What the fix added
    /// is the startup warning, so the operator can tell an unauthenticated
    /// receiver from a verified one.
    #[tokio::test]
    async fn an_adapter_with_no_secret_accepts_an_unsigned_request() {
        let (port, mut rx) = start_on_ephemeral(None).await;
        let status = post(port, "/webhook/github", &[]).await;
        assert_eq!(
            status, 200,
            "with no secret the request is accepted — the warning is the guard"
        );
        // And it really did reach the agent: the message was forwarded.
        let forwarded = rx.try_recv().expect("the message was forwarded");
        assert_eq!(forwarded.platform, "webhook");
        assert_eq!(forwarded.channel_id, "webhook:github");
    }

    /// A subscription that claims a route DISPATCHES: the event is shaped
    /// by the subscription's prompt and skills, the answer's destination is
    /// named, and the request is authenticated by the subscription's own
    /// secret.
    ///
    /// This is the RT-140 dispatch. Without it the store was written and
    /// read by nothing: the event was handled by the agent and the answer
    /// was dropped on the floor (the webhook adapter is inbound-only).
    #[tokio::test]
    async fn a_subscription_route_dispatches_the_event() {
        use std::collections::HashMap;

        let _guard = crate::platform::test_env_lock().await;
        let home = std::env::temp_dir().join(format!("operant-webhook-sub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("data")).expect("the data dir");
        // The home is set BEFORE the store is written: the store path is
        // derived from it, so writing first would touch the operator's real
        // `~/.operant/data/webhook_subscriptions.json`.
        unsafe { std::env::set_var("HERMES_HOME", &home) };
        let mut subs: HashMap<String, super::webhook_subscriptions::WebhookSubscription> =
            HashMap::new();
        subs.insert(
            "https://example.com/webhook/orders".to_string(),
            super::webhook_subscriptions::WebhookSubscription {
                url: "https://example.com/webhook/orders".to_string(),
                route: Some("orders".to_string()),
                prompt: Some("Summarise the order".to_string()),
                events: vec!["message".to_string()],
                description: None,
                skills: vec!["billing".to_string()],
                deliver: Some("telegram:chat-42".to_string()),
                deliver_chat_id: None,
                secret: Some("shared-secret".to_string()),
                deliver_only: false,
            },
        );
        super::webhook_subscriptions::save_subscriptions(&subs).expect("the store is written");

        let (port, mut rx) = start_on_ephemeral(None).await;
        let body = r#"{"order_id":"A-17","total":42}"#;
        let signature = super::webhook_subscriptions::sign_body("shared-secret", body.as_bytes());
        let status = post_body(
            port,
            "/webhook/orders",
            &[("x-webhook-signature", signature.as_str())],
            body,
        )
        .await;
        assert_eq!(status, 200, "a signed event on a claimed route is accepted");

        let forwarded = rx.recv().await.expect("the message was forwarded");
        assert!(
            forwarded.content.contains("Summarise the order"),
            "the subscription's prompt shapes the event: {}",
            forwarded.content
        );
        assert!(
            forwarded.content.contains("billing"),
            "the subscription's skills shape the event: {}",
            forwarded.content
        );
        assert!(
            forwarded.content.contains("A-17"),
            "the event body reaches the agent: {}",
            forwarded.content
        );
        let target = forwarded
            .deliver_to
            .as_ref()
            .expect("the subscription names where the answer goes");
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.channel_id, "chat-42");
        assert!(!forwarded.deliver_only);
        assert!(
            forwarded.authenticated,
            "a verified signature is what makes the event a machine event"
        );

        // A route nobody claimed is NOT authenticated: the adapter's own
        // secret applies, and with none configured the event is anonymous.
        let (port2, mut rx2) = start_on_ephemeral(None).await;
        let status = post_body(port2, "/webhook/unclaimed", &[], "{}").await;
        assert_eq!(status, 200);
        let anonymous = rx2.recv().await.expect("the message was forwarded");
        assert!(
            !anonymous.authenticated,
            "an unclaimed route with no secret is an anonymous event"
        );
        assert!(anonymous.deliver_to.is_none());

        unsafe { std::env::remove_var("HERMES_HOME") };
        let _ = std::fs::remove_dir_all(&home);
    }
}
