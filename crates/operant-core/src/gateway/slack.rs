//! `slack` — extracted verbatim from gateway/mod.rs.

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::gateway_markdown::markdown_to_slack_mrkdwn;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;

use super::*;

/// Slack adapter.
///
/// **OUTBOUND ONLY.** Operant can post to Slack; it cannot receive from
/// it. The Socket Mode WebSocket the doc used to describe DOES NOT EXIST
/// in this tree: `start()` logs "event-based, no polling" and returns, and
/// nothing opens a `wss://` connection or reads the Events API.
///
/// The `handle_update` parser below is therefore DEAD CODE — no caller
/// ever feeds it an update (the trait's `handle_update` is dispatched by
/// the Telegram and Discord pollers, and by the webhook adapter's HTTP
/// handler; Slack has neither).
///
/// The gateway reports this adapter as "enabled" when a token is
/// configured, which made an operator believe they could receive Slack
/// messages. `config_json` now says `inbound: false`.
///
/// Fixing it means implementing Socket Mode: the `apps.connections.open`
/// handshake, the WebSocket, the reconnect, and the event routing. That
/// is a feature, not a patch, and it is tracked as RT-86.
pub struct SlackAdapter {
    token: Option<String>,
    enabled: bool,
    /// A per-instance override for the Slack API base.
    ///
    /// The default is the configured base. A per-instance override lets a
    /// test point ONE adapter at a local server without touching the
    /// process-global runtime config — which `runtime_config()` reads
    /// from ~20 call sites, so mutating it in a test would leak into
    /// every other test in the process.
    api_base_override: Option<String>,
}

impl SlackAdapter {
    /// Create a new Slack adapter.
    ///
    /// The token is used for the OUTBOUND `chat.postMessage` call. It
    /// does not authenticate an inbound connection, because there is no
    /// inbound connection.
    pub fn new(token: Option<String>) -> Self {
        let enabled = token.is_some();
        Self {
            token,
            enabled,
            api_base_override: None,
        }
    }

    /// The API base this adapter talks to.
    fn api_base(&self) -> String {
        if let Some(ref base) = self.api_base_override {
            return base.clone();
        }
        runtime_config().gateway.slack_api_base.clone()
    }

    /// Point this adapter at a different Slack API base.
    ///
    /// A per-instance override, so a test can use a local server without
    /// mutating the process-global runtime config — which
    /// `runtime_config()` is read from ~20 call sites, so mutating it in a
    /// test would leak the mock base into every other test in the
    /// process.
    #[cfg(test)]
    pub(crate) fn with_api_base(mut self, base: &str) -> Self {
        self.api_base_override = Some(base.to_string());
        self
    }
}

/// The messages a Slack body is sent as.
///
/// BOTH Slack send paths go through this: the plain send and the
/// streaming send (`send_message_return_id`). They are separate
/// implementations, and the streaming one was left unchunked for a while
/// — so a long answer on Slack was rejected with `invalid_text` and lost
/// while the plain send delivered it. The shared seam is what keeps them
/// in step.
fn slack_chunks(text: &str) -> Vec<String> {
    chunk_text(text, SLACK_MESSAGE_LIMIT)
}

/// Slack's per-message character limit.
///
/// `chat.postMessage` answers `{"ok": false, "error": "invalid_text"}`
/// over this limit — a 200, so a status check alone does not catch it.
const SLACK_MESSAGE_LIMIT: usize = 40_000;

/// The id of a message Slack has sent, from the response body.
///
/// Slack returns `{"ok": true, "ts": "1712.0001", ...}` — the `ts` is
/// the message id, and it is what `chat.update` needs. The TRAIT's
/// default `send_message_return_id` returns an EMPTY id, which the
/// streaming path reads as "this platform cannot edit" (RT-105).
fn slack_message_id(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("ts").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn slack_send_outcome(status: u16, body: &str) -> Result<()> {
    if !(200..300).contains(&status) {
        return Err(Error::Agent(format!(
            "Slack API error: HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body)
        && parsed.get("ok").and_then(|v| v.as_bool()) == Some(false)
    {
        let err = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(Error::Agent(format!("Slack API error: {err}")));
    }
    Ok(())
}

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        info!("Slack adapter started (event-based, no polling)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Slack adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let client = shared_http_client().clone();
        // Propagate missing token via `?` instead of `.unwrap()` (iter-125 —
        // closes the ponytail-audit "token unwrap panics" security bug).
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Slack token not configured".to_string()))?;

        // Slack uses mrkdwn format, NOT standard Markdown. Raw **bold**
        // renders as literal asterisks. Convert before sending.
        // (Bug #15 from iter-98 audit.)
        //
        // Slack's limit is 40,000 characters per message — over that,
        // `chat.postMessage` answers `{"ok": false, "error":
        // "invalid_text"}` and the message is NOT delivered. So the text
        // is CHUNKED, the way the Telegram and Discord sends are: a long
        // answer arrives as several messages instead of not at all.
        let text = markdown_to_slack_mrkdwn(&message.content);
        let chunks = slack_chunks(&text);

        for chunk in chunks {
            let body = serde_json::json!({
                "channel": message.channel_id,
                "text": chunk,
                // Prevent link-preview noise.
                "unfurl_links": false,
                "unfurl_media": false,
            });

            let resp = client
                .post(format!(
                    "{}/chat.postMessage",
                    self.api_base().trim_end_matches('/')
                ))
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            // The response must be CHECKED. Two ways a Slack send fails:
            //
            // 1. A non-2xx status — a revoked token (401), a missing scope
            //    (403), a rate limit (429).
            // 2. **A 200 with `{"ok": false, "error": "not_in_channel"}`** —
            //    Slack's API convention. A status check alone does NOT catch
            //    this, and it is the common case: a bot that is not in the
            //    channel, or a message that violates a workspace rule.
            //
            // The old code discarded the response entirely, so BOTH looked
            // like a success: the delivery ledger recorded "delivered" and
            // the operator saw a silent no-op. (The same defect class as
            // the SMTP reply discarding, RT-73, and the Telegram status,
            // RT-71.)
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            slack_send_outcome(status.as_u16(), &body_text)?;
        }

        Ok(())
    }

    /// Send a message and return its id, so the streaming path can EDIT
    /// it.
    ///
    /// The TRAIT's default sends the message and returns an EMPTY id,
    /// which the streaming path reads as "this platform cannot edit" — so
    /// on Slack the user saw the first chunk of the answer and then the
    /// full answer as a second message, with no incremental updates
    /// (RT-105). Slack CAN edit, so it returns the real id.
    ///
    /// The send is done here rather than by calling `send_message`,
    /// because the id has to be read out of the response — and the plain
    /// send discards it.
    async fn send_message_return_id(&self, message: OutgoingMessage) -> Result<String> {
        let client = shared_http_client().clone();
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Slack token not configured".to_string()))?;

        // The text is CHUNKED, like the plain send. Over Slack's 40,000
        // character limit `chat.postMessage` answers
        // `{"ok": false, "error": "invalid_text"}` — a 200, so the status
        // check alone does not catch it — and the message is NOT
        // delivered.
        //
        // This is the path the STREAMING path uses, so the gap was not
        // cosmetic: a long answer on Slack was rejected and lost, while
        // the plain send (which chunks) delivered it.
        let text = markdown_to_slack_mrkdwn(&message.content);
        let chunks = slack_chunks(&text);

        // The id is the FIRST chunk's `ts` — that is the message the
        // streaming path edits.
        let mut first_id = String::new();
        for (index, chunk) in chunks.into_iter().enumerate() {
            let body = serde_json::json!({
                "channel": message.channel_id,
                "text": chunk,
                "unfurl_links": false,
                "unfurl_media": false,
            });

            let resp = client
                .post(format!(
                    "{}/chat.postMessage",
                    self.api_base().trim_end_matches('/')
                ))
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            slack_send_outcome(status.as_u16(), &body_text)?;

            if index == 0 {
                // Slack's message id is the message's `ts`.
                first_id = slack_message_id(&body_text);
            }
        }

        Ok(first_id)
    }

    /// Edit a message Slack has already sent.
    ///
    /// Slack's edit is `POST /chat.update` with the channel and the
    /// message's `ts`. The TRAIT's default is a no-op that returns
    /// `Ok("")` — so before this, the streaming path's edits on Slack did
    /// nothing.
    ///
    /// Slack's error convention is a 200 with `{"ok": false}`, so BOTH the
    /// status and the body decide (the same rule as the plain send,
    /// RT-100).
    async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let client = shared_http_client().clone();
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Slack token not configured".to_string()))?;

        let text = markdown_to_slack_mrkdwn(&message.content);
        let body = serde_json::json!({
            "channel": channel_id,
            "ts": message_id,
            "text": text,
        });

        let resp = client
            .post(format!(
                "{}/chat.update",
                self.api_base().trim_end_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::Agent(format!(
                "Slack API error: HTTP {status}: {}",
                body_text.chars().take(200).collect::<String>()
            )));
        }
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body_text)
            && parsed.get("ok").and_then(|v| v.as_bool()) == Some(false)
        {
            let err = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(Error::Agent(format!("Slack API error: {err}")));
        }
        Ok(message_id.to_string())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse Slack event
        let event = match update.get("event") {
            Some(e) => e,
            None => return Ok(None),
        };

        let msg_type = event
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();

        if msg_type != "message" {
            return Ok(None);
        }

        let user = event
            .get("user")
            .and_then(|u| u.as_str())
            .unwrap_or_default()
            .to_string();

        let content = event
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        let channel = event
            .get("channel")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();

        if content.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            IncomingMessage::new("slack", user.clone(), user, channel, content).with_raw(update),
        ))
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "slack",
            "enabled": self.enabled,
            "has_token": self.token.is_some(),
            // The adapter can SEND but not RECEIVE. The gateway's status
            // report used to say "enabled" and nothing more, so an
            // operator believed they could receive Slack messages.
            "inbound": false,
            "outbound": true,
        })
    }

    async fn start_with_channel(
        &self,
        _message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        self.start().await
    }

    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let mut msg = OutgoingMessage::new(channel_id.to_string(), message.content.clone());
        msg.parse_markdown = message.parse_markdown;
        msg.reply_to = message.reply_to.clone();
        self.send_message(msg).await?;
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A Slack send the API REJECTS must be reported as a failure.
    ///
    /// Slack answers a rejected request with **HTTP 200 and
    /// `{"ok": false, "error": "..."}`** — so a status check alone does
    /// not catch it, and the old code discarded the response entirely, so
    /// the delivery ledger recorded "delivered" for a message that was
    /// never sent.
    ///
    /// The check is a PURE function over the response, so the test does
    /// not need a server. An earlier version drove it over HTTP, and two
    /// tests raced: the OS reused the freed ephemeral port and the
    /// process-global `reqwest` client's connection pool reused the
    /// stale connection, so each test received the OTHER server's answer.
    #[test]
    fn a_slack_send_rejected_with_ok_false_is_a_failure() {
        let err = slack_send_outcome(200, r#"{"ok":false,"error":"not_in_channel"}"#)
            .expect_err("a rejected send must be an error");
        let err = err.to_string();
        assert!(
            err.contains("not_in_channel"),
            "the error must name the API's error, got: {err}"
        );
    }

    /// A non-2xx status is a failure too.
    #[test]
    fn a_slack_send_with_an_error_status_is_a_failure() {
        for status in [400u16, 401, 403, 429, 500] {
            let err = slack_send_outcome(status, r#"{"ok":true}"#)
                .expect_err("an error status must be an error");
            let err = err.to_string();
            assert!(
                err.contains(&status.to_string()),
                "the error must name the status, got: {err}"
            );
        }
    }

    /// A counting HTTP server for the Slack API.
    ///
    /// It answers every `chat.postMessage` with a SUCCESS and COUNTS the
    /// requests, so a test can assert how many messages a body was sent
    /// as. It binds port 0 (an ephemeral port the OS picks) and keeps
    /// answering until the test ends — so a second request is not
    /// refused, and the port is not recycled under a later test.
    ///
    /// The count is shared with the test through an `Arc`, and the server
    /// runs on the test's own runtime, so a request is answered.
    async fn counting_slack_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let port = listener.local_addr().expect("the bound address").port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                // Read the WHOLE request: the header block, then exactly
                // `Content-Length` body bytes.
                //
                // Reading only the headers would answer mid-body, and
                // reqwest would fail with "connection closed before
                // message completed" — the over-limit body is 40,001
                // bytes. Reading to EOF would deadlock: the client waits
                // for the response, and the response waits for the
                // client's EOF.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break None,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break Some(pos + 4);
                            }
                        }
                    }
                };
                let Some(header_end) = header_end else {
                    continue;
                };
                // The content length, from the header block.
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.trim().eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                // And the body.
                while buf.len() < header_end + content_length {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                // The count means "messages sent", so it is taken once the
                // request is fully read.
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // The response's Content-Length is COMPUTED from the
                // payload — a wrong one makes reqwest read past the end
                // of the body and fail the whole request.
                let payload = r#"{"ok":true,"channel":"C1","ts":"1.0"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://127.0.0.1:{port}"), count)
    }

    /// The STREAMING send must CHUNK an over-limit body.
    ///
    /// Over Slack's 40,000 character limit `chat.postMessage` answers
    /// `{"ok": false, "error": "invalid_text"}` — a 200, so the status
    /// check alone does not catch it — and the message is NOT delivered.
    ///
    /// `send_message` and `send_message_return_id` are SEPARATE
    /// implementations, and the streaming one was left unchunked for a
    /// while — so a long answer on Slack was rejected and lost while the
    /// plain send delivered it.
    ///
    /// This test drives the ADAPTER against a counting server, so it
    /// proves the streaming path really sends more than one message. A
    /// test that asserted on the chunking HELPER would pass against the
    /// pre-fix code — the helper was called, just not by the path that
    /// mattered.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_slack_streaming_send_chunks_an_over_limit_body() {
        let (base, count) = counting_slack_server().await;

        // The per-instance base, NOT the process-global runtime config:
        // `runtime_config()` is read from ~20 call sites, so mutating it
        // in a test would leak the mock base into every other test in the
        // process.
        let adapter = SlackAdapter::new(Some("xoxb-token".to_string())).with_api_base(&base);
        let body = "x".repeat(SLACK_MESSAGE_LIMIT + 1);
        let message = OutgoingMessage::new("C1", &body);

        let id = adapter
            .send_message_return_id(message)
            .await
            .expect("the streaming send must succeed");
        assert!(!id.is_empty(), "the send must return the message id");

        assert!(
            count.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "an over-limit body must be sent as more than one message, got {}",
            count.load(std::sync::atomic::Ordering::SeqCst)
        );
    }

    /// A SHORT body is one message — the chunking must not split it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_short_slack_body_is_one_message() {
        let (base, count) = counting_slack_server().await;

        let adapter = SlackAdapter::new(Some("xoxb-token".to_string())).with_api_base(&base);
        let message = OutgoingMessage::new("C1", "a short answer");
        adapter
            .send_message_return_id(message)
            .await
            .expect("the streaming send must succeed");

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a short body must be one message"
        );
    }

    /// A Slack send the API ACCEPTS must succeed.
    #[test]
    fn a_slack_send_accepted_by_the_api_succeeds() {
        slack_send_outcome(200, r#"{"ok":true,"channel":"C123","ts":"1.0"}"#)
            .expect("an accepted send must succeed");
    }

    /// The message id must be recoverable from Slack's response body —
    /// the streaming path needs it to EDIT the message.
    ///
    /// The TRAIT's default `send_message_return_id` returns an EMPTY id,
    /// which the streaming path reads as "this platform cannot edit" — so
    /// on Slack the user saw the first chunk of the answer and then the
    /// full answer as a second message (RT-105).
    #[test]
    fn the_slack_message_id_is_recoverable() {
        assert_eq!(
            slack_message_id(r#"{"ok":true,"ts":"1712.0001","channel":"C1"}"#),
            "1712.0001"
        );
        // A body with no ts yields an empty id — which is the sentinel
        // that keeps the later ticks on the edit branch.
        assert_eq!(slack_message_id(r#"{"ok":true,"channel":"C1"}"#), "");
        assert_eq!(slack_message_id("not json"), "");
    }
}
