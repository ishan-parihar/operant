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

/// Verify a webhook request's signature against `sec`.
///
/// Three shapes, in order:
/// - Slack (`X-Slack-Signature` = HMAC-SHA256 hex of `"v0:<ts>:<body>"`
///   plus `X-Slack-Request-Timestamp`, replay-protected within 5 minutes);
/// - Stripe (`Stripe-Signature: t=<ts>,v1=<hex>` where the HMAC covers
///   `"<ts>.<body>"`, same 5-minute replay window, multiple `v1=` entries
///   accepted as Stripe sends them during signing-secret rotation);
/// - the standard `HMAC-SHA256(secret, body)` compared against
///   `x-webhook-signature` (custom) or `x-hub-signature-256` (GitHub).
///
/// All comparisons are constant-time. `Ok(())` means verified; the `Err`
/// pair is the status and message the route must answer with.
///
/// Shared by the event path and the Slack `url_verification` handshake:
/// Slack signs the handshake like every other request, so echoing a
/// challenge before this check answered unauthenticated strangers (the
/// challenge itself is attacker-supplied and leaks nothing, but the
/// endpoint must not verify nothing).
fn verify_request_signature(
    sec: &str,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> std::result::Result<(), (axum::http::StatusCode, &'static str)> {
    let now_secs = unix_now_secs();

    let slack_sig = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok());
    let slack_ts = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok());

    if let (Some(sig), Some(ts)) = (slack_sig, slack_ts) {
        // Replay protection: reject requests older than 5 minutes
        // (Slack's own recommendation).
        if let Ok(ts_secs) = ts.parse::<i64>()
            && (now_secs - ts_secs).abs() > 300
        {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Stale Slack request (replay-protected)",
            ));
        }
        // Compute HMAC-SHA256(signing_secret, "v0:<ts>:<body>").
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
            Ok(m) => m,
            Err(_) => {
                return Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "HMAC key error",
                ));
            }
        };
        let basestring = format!("v0:{ts}:");
        mac.update(basestring.as_bytes());
        mac.update(body);
        let expected = mac.finalize().into_bytes();
        let expected_hex = format!("v0={}", hex::encode(expected));
        if !constant_time_eq::constant_time_eq(sig.as_bytes(), expected_hex.as_bytes()) {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Invalid Slack signature",
            ));
        }
        Ok(())
    } else if let Some(stripe) = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
    {
        // Stripe signs `HMAC-SHA256(secret, "{t}.{body}")` and ships it as
        // `t=<ts>,v1=<hex>`. (ponytail audit 2026-10-24: this used to fall
        // into the generic branch, comparing the WHOLE `t=...,v1=...`
        // header against a plain HMAC of the body — a comparison no
        // genuine Stripe event can ever pass, so every real event was
        // 401'd and Stripe retried, then disabled the endpoint.)
        let Some((ts, sigs)) = parse_stripe_signature(stripe) else {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Malformed Stripe signature header (expected t=<ts>,v1=<hex>)",
            ));
        };
        if (now_secs - ts).abs() > 300 {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Stale Stripe request (replay-protected)",
            ));
        }
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
            Ok(m) => m,
            Err(_) => {
                return Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "HMAC key error",
                ));
            }
        };
        let basestring = format!("{ts}.");
        mac.update(basestring.as_bytes());
        mac.update(body);
        let expected_hex = hex::encode(mac.finalize().into_bytes());
        // Any match verifies (Stripe sends several v1 during secret
        // rotation); every comparison stays constant-time.
        if sigs
            .iter()
            .any(|sig| constant_time_eq::constant_time_eq(sig.as_bytes(), expected_hex.as_bytes()))
        {
            Ok(())
        } else {
            Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Invalid Stripe signature",
            ))
        }
    } else {
        // Standard HMAC verification (GitHub / generic webhooks).
        let sig = headers
            .get("x-webhook-signature")
            .or_else(|| headers.get("x-hub-signature-256"))
            .and_then(|v| v.to_str().ok());

        let Some(sig) = sig else {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Missing signature header (x-slack-signature / x-webhook-signature / x-hub-signature-256 / stripe-signature)",
            ));
        };
        // Strip "sha256=" prefix if present (GitHub format).
        let sig_hex = sig.strip_prefix("sha256=").unwrap_or(sig);
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
            Ok(m) => m,
            Err(_) => {
                return Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "HMAC key error",
                ));
            }
        };
        mac.update(body);
        let expected = mac.finalize().into_bytes();
        let expected_hex = hex::encode(expected);
        if !constant_time_eq::constant_time_eq(sig_hex.as_bytes(), expected_hex.as_bytes()) {
            return Err((axum::http::StatusCode::UNAUTHORIZED, "Invalid signature"));
        }
        Ok(())
    }
}

/// Seconds since the Unix epoch; 0 if the clock is before the epoch.
fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a Stripe `Stripe-Signature` header (`t=<ts>,v1=<hex>[,v1=<hex>…]`)
/// into its timestamp and its `v1` signatures. `None` when the header lacks
/// a parseable `t=` or has no `v1=` at all — such a header is not a Stripe
/// signature and must not be verified against anything.
fn parse_stripe_signature(header: &str) -> Option<(i64, Vec<&str>)> {
    let mut ts: Option<i64> = None;
    let mut sigs: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("t=") {
            ts = v.parse::<i64>().ok();
        } else if let Some(v) = part.strip_prefix("v1=") {
            sigs.push(v);
        }
    }
    if sigs.is_empty() {
        None
    } else {
        Some((ts?, sigs))
    }
}

impl WebhookAdapter {
    /// The loopback default: a webhook receiver that accepts unsigned POSTs
    /// must not sit on every interface out of the box. Binding beyond the
    /// machine (`0.0.0.0`, a LAN IP) is an explicit `[gateway] webhooks_addr`
    /// decision, made together with `webhooks_secret`.
    pub const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:8080";

    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            listen_addr: Self::DEFAULT_LISTEN_ADDR.to_string(),
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
        // unauthenticated command channel pointed at the agent". The
        // default listen address is loopback for exactly this reason
        // (ponytail audit 2026-10-24); an operator who sets a wider
        // `webhooks_addr` without a secret has reopened the exposure, so
        // say so, loudly, at startup: the operator cannot otherwise tell
        // an unauthenticated receiver from a verified one.
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
                            // Verify the token matches our secret (if one is
                            // configured). Constant-time, like every HMAC
                            // comparison in this file — an ordinary `==`
                            // short-circuits on the first differing byte and
                            // leaks the token's prefix length over the wire.
                            let token_ok = match &secret {
                                Some(expected) => constant_time_eq::constant_time_eq(
                                    verify_token.as_bytes(),
                                    expected.as_bytes(),
                                ),
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
                        // (iter-1132) The route may be claimed by a webhook
                        // subscription. A subscription with its own secret is
                        // verified against THAT secret, so two routes can use
                        // different ones; without one, the adapter's global
                        // `[gateway] webhooks_secret` applies. Resolved BEFORE
                        // the handshakes below: the Slack url_verification
                        // echo is authenticated with the same secret the
                        // events on this route use.
                        let subscription =
                            super::webhook_subscriptions::subscription_for_route(&route);
                        let effective_secret: Option<String> = subscription
                            .as_ref()
                            .and_then(|s| s.secret.clone())
                            .or_else(|| secret.clone());

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
                        // must be handled BEFORE the signature check below —
                        // EXCEPT Slack's: Slack signs url_verification like every
                        // other request, so under a configured secret the
                        // challenge echo is refused without a valid signature
                        // (answering it unauthenticated verified nothing).
                        // ────────────────────────────────────────────────────────

                        // Try parsing the body as JSON first (Slack url_verification
                        // is JSON; WhatsApp handshake is GET with query params).
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Slack url_verification
                            if v.get("type").and_then(|t| t.as_str()) == Some("url_verification")
                                && let Some(challenge) = v.get("challenge").and_then(|c| c.as_str()) {
                                    if let Some(ref sec) = effective_secret
                                        && let Err((code, msg)) =
                                            verify_request_signature(sec, &headers, &body)
                                    {
                                        return (code, msg).into_response();
                                    }
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

                        // A signature that verified is what makes the event a
                        // machine event rather than an anonymous request.
                        let mut authenticated = false;

                        // Validate HMAC signature if secret is configured.
                        // Uses standard HMAC-SHA256(secret, body) — not the
                        // old non-standard SHA256(route+secret). Supports
                        // multiple signature header names for interop:
                        // x-webhook-signature (custom), x-hub-signature-256
                        // (GitHub), Stripe-Signature (Stripe), plus Slack's
                        // non-standard v0 shape.
                        // (iter-101 closes Bug #9 from iter-98 audit; iter-125
                        // closed the ponytail-audit security bug "Slack
                        // signing_secret collected but HMAC verification never
                        // performed"; the checks live in
                        // [`verify_request_signature`] above, shared with the
                        // url_verification handshake.)
                        if let Some(ref sec) = effective_secret {
                            if let Err((code, msg)) = verify_request_signature(sec, &headers, &body)
                            {
                                return (code, msg).into_response();
                            }
                            // The signature matched (the helper returns on a
                            // mismatch).
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
                                                // A closed channel means the
                                                // gateway is gone: answer 5xx
                                                // so Slack retries instead of
                                                // trusting an "ok" that lost
                                                // the event.
                                                if tx.send(slack_msg).is_err() {
                                                    return (
                                                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                                        "Channel closed",
                                                    )
                                                        .into_response();
                                                }
                                            }
                                        }
                                        // 200 on the paths above — a non-2xx
                                        // makes Slack retry, which is exactly
                                        // what we want only when the event was
                                        // NOT delivered.
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
                                                // Same contract as the Slack
                                                // arm above: a closed channel
                                                // must NOT be acked, or Meta
                                                // drops the event on a 200.
                                                if tx.send(wa_msg).is_err() {
                                                    return (
                                                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                                        "Channel closed",
                                                    )
                                                        .into_response();
                                                }
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

    /// Sign a body the Slack way: `v0=<hex HMAC-SHA256(secret, "v0:<ts>:<body>")>`.
    fn slack_sign(secret: &str, ts: i64, body: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(format!("v0:{ts}:").as_bytes());
        mac.update(body.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// POST a body and return the status AND body of the response.
    async fn post_body_response(
        port: u16,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (u16, String) {
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
        let status = response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        // The body starts after the first blank line.
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    /// A Slack `url_verification` challenge is REFUSED unsigned when a
    /// secret is configured. The handshake used to run before the
    /// signature check and echoed any challenge to any caller — Slack
    /// signs url_verification like every other request, so refusing the
    /// unsigned shape closes an unauthenticated reflector while keeping
    /// real registration working (next test).
    /// GET a path and return the status code and body — the handshake twin
    /// of `post_body_response`. The GET arm had no test at all until now.
    async fn get_body_response(port: u16, path: &str) -> (u16, String) {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read");
        let status = response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    /// The Meta/WhatsApp GET handshake echoes the challenge only for the
    /// matching verify token, and the comparison is constant-time like every
    /// other secret comparison in this adapter (an `==` short-circuits on
    /// the first differing byte; the timing property itself is not
    /// unit-testable, so this pins the behavior around it).
    #[tokio::test]
    async fn the_meta_handshake_echoes_the_challenge_only_for_the_right_token() {
        let (port, _rx) = start_on_ephemeral(Some("shared-secret")).await;
        let (status, body) = get_body_response(
            port,
            "/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=shared-secret&hub.challenge=12391",
        )
        .await;
        assert_eq!(status, 200, "the matching token is accepted");
        assert_eq!(body, "12391", "the challenge is echoed back verbatim");

        let (status, _) = get_body_response(
            port,
            "/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=wrong-token&hub.challenge=12391",
        )
        .await;
        assert_eq!(status, 403, "a wrong token is refused");

        let (status, _) = get_body_response(port, "/webhook/whatsapp?hub.challenge=12391").await;
        assert_eq!(
            status, 400,
            "a GET without hub.mode=subscribe is a bad request"
        );
    }

    #[tokio::test]
    async fn a_url_verification_challenge_is_refused_unsigned_under_a_secret() {
        let (port, _rx) = start_on_ephemeral(Some("shared-secret")).await;
        let status = post_body(
            port,
            "/webhook/slack",
            &[],
            r#"{"type":"url_verification","challenge":"echo-me"}"#,
        )
        .await;
        assert_eq!(
            status, 401,
            "an unsigned url_verification must be refused when a secret is configured"
        );
    }

    /// The SAME handshake with a valid Slack signature is answered — the
    /// challenge comes back and the registration still works.
    #[tokio::test]
    async fn a_url_verification_challenge_is_answered_with_a_slack_signature() {
        let (port, _rx) = start_on_ephemeral(Some("shared-secret")).await;
        let body = r#"{"type":"url_verification","challenge":"echo-me"}"#;
        let ts = chrono::Utc::now().timestamp();
        let sig = slack_sign("shared-secret", ts, body);
        let (status, response_body) = post_body_response(
            port,
            "/webhook/slack",
            &[
                ("x-slack-signature", sig.as_str()),
                ("x-slack-request-timestamp", &ts.to_string()),
            ],
            body,
        )
        .await;
        assert_eq!(status, 200, "a signed handshake is answered");
        assert!(
            response_body.contains("echo-me"),
            "the challenge is echoed back: {response_body}"
        );
    }

    /// A Slack `event_callback` on a gateway whose channel is gone must
    /// NOT be acked. The arm used `let _ = tx.send(..)` and returned 200 —
    /// Slack trusted the ok, never retried, and the event was silently
    /// lost. The generic arm already answered 500; the Slack arm now
    /// matches it.
    #[tokio::test]
    async fn a_slack_event_on_a_closed_channel_is_not_acked() {
        let (port, rx) = start_on_ephemeral(None).await;
        drop(rx); // the gateway is gone
        let status = post_body(
            port,
            "/webhook/slack",
            &[],
            r#"{"type":"event_callback","event":{"type":"message","text":"hello","user":"u1","channel":"c1"}}"#,
        )
        .await;
        assert_eq!(
            status, 500,
            "a dropped event must not be acked — Slack retries on 5xx"
        );
    }

    /// Same contract for the WhatsApp/Meta event arm.
    #[tokio::test]
    async fn a_whatsapp_event_on_a_closed_channel_is_not_acked() {
        let (port, rx) = start_on_ephemeral(None).await;
        drop(rx);
        let status = post_body(
            port,
            "/webhook/whatsapp",
            &[],
            r#"{"entry":[{"changes":[{"value":{"messages":[{"from":"15550001111","text":{"body":"hi"},"timestamp":"1700000000"}]}}]}]}"#,
        )
        .await;
        assert_eq!(
            status, 500,
            "a dropped WhatsApp event must not be acked — Meta retries on 5xx"
        );
    }

    /// Ponytail audit 2026-10-24: the out-of-the-box listener must be
    /// loopback. A default of 0.0.0.0 combined with the optional secret
    /// made "enable webhooks" mean "unauthenticated command channel on
    /// every interface" — the adapter default and the gateway factory
    /// fallback (gateway_runner.rs) both read this constant, so pinning it
    /// here pins both.
    #[test]
    fn default_listen_addr_is_loopback() {
        assert_eq!(WebhookAdapter::DEFAULT_LISTEN_ADDR, "127.0.0.1:8080");
        assert_eq!(WebhookAdapter::new(true).listen_addr, "127.0.0.1:8080");
    }

    /// Stripe signs `HMAC-SHA256(secret, "{t}.{body}")` and ships it as
    /// `t=<ts>,v1=<hex>` — with every `v1=` repeated during signing-secret
    /// rotation. The audit found the whole header compared against a plain
    /// body HMAC, which no genuine Stripe event can ever match; these pin
    /// the real contract.
    fn stripe_v1(secret: &str, ts: i64, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(format!("{ts}.").as_bytes());
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn stripe_headers(
        secret: &str,
        ts: i64,
        body: &[u8],
        extra_v1: &[&str],
    ) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        let mut value = format!("t={ts},v1={}", stripe_v1(secret, ts, body));
        for v in extra_v1 {
            value.push_str(&format!(",v1={v}"));
        }
        headers.insert(
            axum::http::HeaderName::from_static("stripe-signature"),
            axum::http::HeaderValue::from_str(&value).expect("header value"),
        );
        headers
    }

    fn unix_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs() as i64
    }

    #[test]
    fn a_genuine_stripe_signature_verifies() {
        let body = br#"{"type":"payment_intent.succeeded"}"#;
        let headers = stripe_headers("whsec_test", unix_now(), body, &[]);
        assert!(verify_request_signature("whsec_test", &headers, body).is_ok());
    }

    #[test]
    fn a_rotating_stripe_secret_verifies_via_any_v1() {
        let body = br#"{"id":"evt_1"}"#;
        // The current secret's v1 plus a rotation-era v1 that no longer
        // matches — Stripe's own rotation shape.
        let rotation_era = "deadbeef".repeat(32);
        let headers = stripe_headers("whsec_test", unix_now(), body, &[&rotation_era]);
        assert!(verify_request_signature("whsec_test", &headers, body).is_ok());
    }

    #[test]
    fn a_wrong_stripe_signature_is_refused() {
        let body = br#"{"id":"evt_2"}"#;
        let headers = stripe_headers("whsec_test", unix_now(), body, &[]);
        let err =
            verify_request_signature("whsec_other", &headers, body).expect_err("must not verify");
        assert_eq!(err.0, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(err.1, "Invalid Stripe signature");
    }

    #[test]
    fn a_stale_stripe_timestamp_is_refused() {
        let body = br#"{"id":"evt_3"}"#;
        let stale = unix_now() - 3600;
        let headers = stripe_headers("whsec_test", stale, body, &[]);
        let err = verify_request_signature("whsec_test", &headers, body)
            .expect_err("replay window is 5 minutes");
        assert_eq!(err.0, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(err.1, "Stale Stripe request (replay-protected)");
    }

    #[test]
    fn a_malformed_stripe_header_is_refused() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_static("stripe-signature"),
            axum::http::HeaderValue::from_static("t=1234567890"),
        );
        let err = verify_request_signature("whsec_test", &headers, b"{}")
            .expect_err("no v1 means nothing to verify");
        assert_eq!(err.0, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            err.1,
            "Malformed Stripe signature header (expected t=<ts>,v1=<hex>)"
        );
    }
}
