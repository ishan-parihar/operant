//! Webhook subscriptions — the store behind `operant webhook`.
//!
//! A subscription records an INBOUND route: the path segment under
//! `/webhook/` that the external service posts to, the prompt and skills
//! that shape how the agent handles the event, and where the answer goes
//! (`deliver` / `deliver_chat_id`).
//!
//! Before iter-1127 the whole family lived in the CLI: the store was
//! written by `operant webhook subscribe` and read by NOTHING — the
//! runtime never dispatched to a subscription — and `operant webhook test`
//! printed "This would send a POST request" without sending one. The store
//! now lives in core so the signing scheme, the loader and the POST are
//! one implementation, and `operant webhook test` really posts.
//!
//! (iter-1132) The DISPATCH landed with the R1 (inbound) model: the
//! subscription's route is matched by the inbound adapter, the event is
//! handled by the agent, and the answer is delivered to the subscription's
//! `deliver` target instead of to the dead inbound channel. The R2
//! (outbound) alternative was rejected: `deliver`, `deliver_chat_id` and
//! `deliver_only` only mean anything when WE receive the event and answer
//! it somewhere. The reasoning is recorded in the audit ledger (RT-140).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::DeliveryTarget;

/// One webhook subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    /// The public URL of the endpoint (the map key). For an inbound
    /// subscription the route is derived from its last path segment unless
    /// `route` names one explicitly.
    pub url: String,
    /// The path segment under `/webhook/` this subscription answers to.
    /// `None` = the subscription is recorded but no route claims it (it
    /// never dispatches).
    #[serde(default)]
    pub route: Option<String>,
    /// The prompt that accompanies the event.
    #[serde(default)]
    pub prompt: Option<String>,
    /// The event names that reach this subscription.
    #[serde(default)]
    pub events: Vec<String>,
    /// The operator's description of the subscription.
    #[serde(default)]
    pub description: Option<String>,
    /// Skills the event's handling should use (the agent reads them with
    /// the skills tool).
    #[serde(default)]
    pub skills: Vec<String>,
    /// Where the agent's answer goes (e.g. "telegram:chat_id").
    #[serde(default)]
    pub deliver: Option<String>,
    /// The delivery chat id, when it differs from the deliver target.
    #[serde(default)]
    pub deliver_chat_id: Option<String>,
    /// The HMAC-SHA256 secret used to sign the request. A subscription
    /// with its own secret is verified against THAT secret, so two routes
    /// can use different ones.
    #[serde(default)]
    pub secret: Option<String>,
    /// Deliver only — do not run the agent for the event.
    #[serde(default)]
    pub deliver_only: bool,
}

impl WebhookSubscription {
    /// Where the answer to an event on this subscription goes.
    ///
    /// `deliver` is `platform:chat_id`; `deliver_chat_id` replaces the chat
    /// id when the operator wants a different conversation on the same
    /// platform. `None` when no usable target is named — the answer would
    /// otherwise go to the inbound channel, which cannot carry it.
    pub fn delivery_target(&self) -> Option<DeliveryTarget> {
        let spec = self.deliver.as_deref()?;
        let mut target = DeliveryTarget::parse(spec)?;
        if let Some(chat) = self
            .deliver_chat_id
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            target.channel_id = chat.to_string();
        }
        Some(target)
    }

    /// The content the agent sees for an event: the operator's prompt, the
    /// skills the event should use, and the event body itself.
    pub fn compose_event_prompt(&self, event: &str) -> String {
        let mut out = String::new();
        if let Some(prompt) = self
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            out.push_str(prompt);
            out.push_str("\n\n");
        }
        if !self.skills.is_empty() {
            out.push_str("<webhook_skills>\n");
            for skill in &self.skills {
                out.push_str(skill.trim());
                out.push('\n');
            }
            out.push_str("</webhook_skills>\n\n");
        }
        out.push_str("<webhook_event>\n");
        out.push_str(event);
        out.push_str("\n</webhook_event>");
        out
    }
    /// Whether this subscription handles an event with this name.
    ///
    /// An empty list — or a list containing `*` — accepts every event. A
    /// payload that names no event is a "message" event, which is what the
    /// CLI subscribes to by default.
    pub fn accepts_event(&self, event: &str) -> bool {
        self.events.is_empty()
            || self
                .events
                .iter()
                .any(|name| name.trim() == "*" || name.trim() == event)
    }
}

/// The route a URL claims: its last path segment.
///
/// `https://example.com/webhook/orders` claims the route `orders`, so the
/// external service posts to `POST /webhook/orders`. A segment that is not
/// a plain path token claims nothing — the operator then names the route
/// explicitly.
pub fn route_from_url(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())?;
    if last.is_empty()
        || !last
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(last.to_string())
}

/// The subscription that claims `route`, if any.
///
/// The store is read per request so a subscription the operator adds takes
/// effect without a gateway restart; the file is a handful of entries.
pub fn subscription_for_route(route: &str) -> Option<WebhookSubscription> {
    let subs = load_subscriptions().ok()?;
    subs.into_values()
        .find(|sub| sub.route.as_deref() == Some(route))
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

    /// The route a URL claims is its last path segment — so the operator's
    /// existing `subscribe <url>` starts naming a route without a new flag.
    #[test]
    fn the_route_is_the_last_path_segment_of_the_url() {
        assert_eq!(
            route_from_url("https://example.com/webhook/orders").as_deref(),
            Some("orders")
        );
        assert_eq!(
            route_from_url("https://example.com/webhook/orders/").as_deref(),
            Some("orders")
        );
        assert_eq!(
            route_from_url("https://example.com/webhook/orders?x=1").as_deref(),
            Some("orders")
        );
        // A segment that is not a plain path token claims nothing: the
        // operator then names the route explicitly.
        assert_eq!(route_from_url("https://example.com/webhook/ord ers"), None);
        assert_eq!(route_from_url("https://example.com/"), None);
    }

    /// The delivery target is `platform:chat_id`, and `deliver_chat_id`
    /// replaces the chat id on the same platform.
    #[test]
    fn the_delivery_target_is_parsed_from_the_deliver_spec() {
        let mut sub = subscription("https://example.com/webhook/orders");
        assert!(sub.delivery_target().is_none());

        sub.deliver = Some("telegram:chat-42".to_string());
        let target = sub.delivery_target().expect("a target");
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.channel_id, "chat-42");

        sub.deliver_chat_id = Some("chat-99".to_string());
        let target = sub.delivery_target().expect("a target");
        assert_eq!(target.platform, "telegram");
        assert_eq!(target.channel_id, "chat-99");

        // A spec with no platform or no chat id is no target at all: the
        // answer would otherwise be sent to the dead inbound channel.
        sub.deliver = Some("telegram".to_string());
        assert!(sub.delivery_target().is_none());
        sub.deliver = Some("telegram:".to_string());
        assert!(sub.delivery_target().is_none());
    }

    /// The prompt and the skills shape the event the agent sees; the event
    /// body is always there.
    #[test]
    fn the_composed_event_carries_the_prompt_the_skills_and_the_body() {
        let mut sub = subscription("https://example.com/webhook/orders");
        sub.prompt = Some("Summarise the order".to_string());
        sub.skills = vec!["billing".to_string(), "shipping".to_string()];

        let composed = sub.compose_event_prompt("{\"order_id\":\"A-17\"}");
        assert!(composed.contains("Summarise the order"), "{composed}");
        assert!(composed.contains("billing"), "{composed}");
        assert!(composed.contains("shipping"), "{composed}");
        assert!(composed.contains("A-17"), "{composed}");

        // With no prompt and no skills, the event body is the whole thing.
        let bare = subscription("https://example.com/webhook/orders");
        let composed = bare.compose_event_prompt("hello");
        assert!(composed.contains("hello"), "{composed}");
        assert!(!composed.contains("webhook_skills"), "{composed}");
    }

    /// A subscription with a route is found by that route; one without is
    /// never dispatched.
    #[test]
    fn a_subscription_is_found_by_its_route() {
        let mut claimed = subscription("https://example.com/webhook/orders");
        claimed.route = Some("orders".to_string());
        let unclaimed = subscription("https://example.com/webhook/other");
        let subs: HashMap<String, WebhookSubscription> = HashMap::from([
            (claimed.url.clone(), claimed),
            (unclaimed.url.clone(), unclaimed),
        ]);

        let found = subs
            .values()
            .find(|sub| sub.route.as_deref() == Some("orders"))
            .expect("the claimed subscription");
        assert_eq!(found.url, "https://example.com/webhook/orders");
        assert!(
            subs.values()
                .all(|sub| sub.route.as_deref() != Some("other")),
            "a subscription with no route claims nothing"
        );
    }

    /// A subscription with every field the CLI writes.
    fn subscription(url: &str) -> WebhookSubscription {
        WebhookSubscription {
            url: url.to_string(),
            route: None,
            prompt: None,
            events: vec!["message".to_string()],
            description: None,
            skills: Vec::new(),
            deliver: None,
            deliver_chat_id: None,
            secret: None,
            deliver_only: false,
        }
    }
}
