//! Webhook subscriptions — the store behind `operant webhook`.
//!
//! A subscription records an outbound destination: a URL, the events that
//! should reach it, an optional prompt, an optional signing secret, and
//! where the agent's answer goes (`deliver` / `deliver_chat_id`).
//!
//! Before iter-1127 the whole family lived in the CLI: the store was
//! written by `operant webhook subscribe` and read by NOTHING — the
//! runtime never dispatched to a subscription — and `operant webhook test`
//! printed "This would send a POST request" without sending one. The store
//! now lives in core so the signing scheme, the loader and the POST are
//! one implementation, and `operant webhook test` really posts.
//!
//! The DISPATCH itself is still open: the runtime does not yet deliver
//! events to a subscription. That gap is recorded in the audit ledger
//! (RT-140) together with the two candidate models, because the choice
//! between them is a product decision, not an implementation detail.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One webhook subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    /// Where the event is posted.
    pub url: String,
    /// The prompt that accompanies the event.
    #[serde(default)]
    pub prompt: Option<String>,
    /// The event names that reach this subscription.
    #[serde(default)]
    pub events: Vec<String>,
    /// The operator's description of the subscription.
    #[serde(default)]
    pub description: Option<String>,
    /// Skills enabled for the event.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Where the agent's answer goes (e.g. "telegram:chat_id").
    #[serde(default)]
    pub deliver: Option<String>,
    /// The delivery chat id, when it differs from the deliver target.
    #[serde(default)]
    pub deliver_chat_id: Option<String>,
    /// The HMAC-SHA256 secret used to sign the request.
    #[serde(default)]
    pub secret: Option<String>,
    /// Deliver only — do not run the agent for the event.
    #[serde(default)]
    pub deliver_only: bool,
}

/// Where the subscriptions live.
pub fn subscriptions_path() -> PathBuf {
    crate::platform::operant_data_dir().join("webhook_subscriptions.json")
}

/// Load the subscriptions. A missing file is an empty set.
pub fn load_subscriptions() -> Result<HashMap<String, WebhookSubscription>, String> {
    let path = subscriptions_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {e}", path.display()))
}

/// Save the subscriptions atomically (temp file + rename).
pub fn save_subscriptions(subs: &HashMap<String, WebhookSubscription>) -> Result<(), String> {
    let path = subscriptions_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(subs)
        .map_err(|e| format!("Failed to serialise the subscriptions: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("Failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("Failed to replace {}: {e}", path.display()))?;
    Ok(())
}

/// Sign a body with HMAC-SHA256, in the format the INBOUND webhook
/// adapter accepts: `sha256=<hex>` (the adapter also accepts a bare hex
/// digest, and reads `x-webhook-signature`, `x-hub-signature-256` or
/// `stripe-signature`).
pub fn sign_body(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        // An HMAC key of any length is valid, so this cannot fail for a
        // string key; fall back to an unsigned body rather than panicking.
        Err(_) => return String::new(),
    };
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// The result of a test post.
#[derive(Debug, Clone)]
pub struct PostOutcome {
    /// The HTTP status the endpoint answered with.
    pub status: u16,
    /// The response body (truncated).
    pub body: String,
}

/// POST a JSON body to a URL, signing it when a secret is given. The
/// response body is truncated to a readable length.
pub async fn post_json(url: &str, body: &str, secret: Option<&str>) -> Result<PostOutcome, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("could not build the HTTP client: {e}"))?;
    let mut request = client
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string());
    if let Some(secret) = secret
        && !secret.is_empty()
    {
        request = request.header("x-webhook-signature", sign_body(secret, body.as_bytes()));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("could not reach {url}: {e}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let body = if text.len() > 500 {
        format!("{}…", &text[..500])
    } else {
        text
    };
    Ok(PostOutcome { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known HMAC-SHA256 vector (key "secret", message "hello"). The
    /// expectation is the standard digest, not this crate's own output.
    #[test]
    fn the_signature_matches_the_known_hmac_vector() {
        assert_eq!(
            sign_body("secret", b"hello"),
            "sha256=88aab3ede8d3adf94d26ab90d3bafd4a2083070c3bcce9c014ee04a443847c0b"
        );
    }

    #[tokio::test]
    async fn a_test_post_reaches_the_endpoint_and_carries_the_signature() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let body = r#"{"event":"ping"}"#;
        let expected_signature = sign_body("secret", body.as_bytes());

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let read = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..read]).to_string();
            let response = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";
            let _ = stream.write_all(response.as_bytes()).await;
            request
        });

        let outcome = post_json(
            &format!("http://127.0.0.1:{port}/hook"),
            body,
            Some("secret"),
        )
        .await
        .expect("post");
        assert_eq!(outcome.status, 200);
        assert_eq!(outcome.body, "ok");

        let request = server.await.expect("join");
        assert!(request.contains("x-webhook-signature"), "{request}");
        assert!(request.contains(&expected_signature), "{request}");
        assert!(request.contains("{\"event\":\"ping\"}"), "{request}");
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_the_failure() {
        // Port 1 on loopback: nothing listens there.
        let err = post_json("http://127.0.0.1:1/hook", "{}", None)
            .await
            .expect_err("must fail");
        assert!(err.contains("could not reach"), "{err}");
    }
}
