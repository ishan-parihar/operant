use anyhow::Context;
use async_trait::async_trait;
use operant_api::channel::{
    Channel, ChannelApprovalRequest, ChannelApprovalResponse, ChannelMessage, SendMessage,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, oneshot};

#[derive(Clone)]
struct CachedSlackDisplayName {
    display_name: String,
    expires_at: Instant,
}

/// Slack channel — polls conversations.history via Web API
#[allow(clippy::struct_excessive_bools)]
pub struct SlackChannel {
    bot_token: String,
    app_token: Option<String>,
    channel_ids: Vec<String>,
    allowed_users: Vec<String>,
    thread_replies: bool,
    mention_only: bool,
    strict_mention_in_thread: bool,
    group_reply_allowed_sender_ids: Vec<String>,
    user_display_name_cache: Mutex<HashMap<String, CachedSlackDisplayName>>,
    workspace_dir: Option<PathBuf>,
    /// Maps channel_id -> thread_ts for active assistant threads (used for status indicators).
    active_assistant_thread: Mutex<HashMap<String, String>>,
    /// Use the newer `markdown` block type (richer formatting, 12k char limit).
    use_markdown_blocks: bool,
    /// Per-channel proxy URL override.
    proxy_url: Option<String>,
    /// Voice transcription config — when set, audio file attachments are
    /// downloaded, transcribed, and their text inlined into the message.
    transcription: Option<operant_config::schema::TranscriptionConfig>,
    transcription_manager: Option<std::sync::Arc<super::transcription::TranscriptionManager>>,
    /// Enable progressive draft message updates via `chat.update`.
    stream_drafts: bool,
    /// Minimum interval (ms) between draft edits to stay within Slack rate limits.
    draft_update_interval_ms: u64,
    /// Per-channel rate-limit tracker for draft edits.
    last_draft_edit: Mutex<HashMap<String, Instant>>,
    /// Maps lazy placeholder IDs to real Slack message timestamps.
    /// `send_draft` returns a placeholder without posting; the real message
    /// is created on the first `update_draft` call.
    lazy_draft_ts: tokio::sync::Mutex<HashMap<String, String>>,
    /// Emoji reaction name (without colons) that cancels an in-flight request.
    cancel_reaction: Option<String>,
    pending_approvals: Arc<AsyncMutex<HashMap<String, oneshot::Sender<ChannelApprovalResponse>>>>,
    /// Seconds to wait for an operator reply to a `request_approval` prompt
    /// before treating the silence as a deny. Default 300.
    approval_timeout_secs: u64,
}

// Split modules (dedup pass 8) - paths unchanged.
mod channel_impl;
mod helpers;
#[cfg(test)]
mod tests;

pub(crate) use helpers::*;

const SLACK_TRUNCATION_INDICATOR: &str = "\n\n...[message truncated]";

/// Split `text` into chunks of at most `max_chars`, breaking at newline or
/// space boundaries when possible. Returns at most `max_chunks` pieces; if the
/// text would require more, the last chunk includes a truncation indicator.
fn split_text_into_chunks(text: &str, max_chars: usize, max_chunks: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() && chunks.len() < max_chunks {
        let is_last_slot = chunks.len() + 1 == max_chunks;

        if remaining.len() <= max_chars && !is_last_slot {
            chunks.push(remaining.to_string());
            break;
        }

        if is_last_slot {
            // Last allowed slot: if remaining fits, just push it.
            if remaining.len() <= max_chars {
                chunks.push(remaining.to_string());
            } else {
                // Truncate with indicator.
                let avail = max_chars - SLACK_TRUNCATION_INDICATOR.len();
                let break_at = remaining[..avail]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .or_else(|| remaining[..avail].rfind(' ').map(|i| i + 1))
                    .unwrap_or(avail);
                let mut chunk = remaining[..break_at].to_string();
                chunk.push_str(SLACK_TRUNCATION_INDICATOR);
                chunks.push(chunk);
            }
            break;
        }

        // Normal chunk: find a good break point.
        let limit = max_chars.min(remaining.len());
        let break_at = remaining[..limit]
            .rfind('\n')
            .map(|i| i + 1)
            .or_else(|| remaining[..limit].rfind(' ').map(|i| i + 1))
            .unwrap_or(limit);

        chunks.push(remaining[..break_at].to_string());
        remaining = &remaining[break_at..];
    }

    chunks
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        // Detect Block Kit payloads produced by the `/config` command.
        let body = if let Some(blocks_json) =
            message.content.strip_prefix(crate::util::BLOCK_KIT_PREFIX)
        {
            let blocks: serde_json::Value = serde_json::from_str(blocks_json)
                .context("invalid Block Kit JSON in runtime command response")?;
            let mut body = serde_json::json!({
                "channel": message.recipient,
                "text": "Model configuration",
                "blocks": blocks
            });
            if let Some(ts) = self.outbound_thread_ts(message) {
                body["thread_ts"] = serde_json::json!(ts);
            }
            body
        } else {
            let mut body = serde_json::json!({
                "channel": message.recipient,
                "text": message.content
            });

            // Add rich formatting blocks, split into chunks for the per-block limit.
            // The newer `markdown` block type (12k chars) offers richer formatting but
            // isn't available on all workspaces, causing `invalid_blocks` errors (#4563).
            // Default to the universally supported `section` block with `mrkdwn`.
            let block_limit = if self.use_markdown_blocks {
                SLACK_MARKDOWN_BLOCK_MAX_CHARS
            } else {
                SLACK_BLOCK_TEXT_MAX_CHARS
            };
            if message.content.len() <= SLACK_MARKDOWN_BLOCK_MAX_CHARS {
                let chunks = split_text_into_chunks(
                    &message.content,
                    block_limit,
                    SLACK_MAX_BLOCKS_PER_MESSAGE,
                );
                let blocks: Vec<serde_json::Value> = chunks
                    .into_iter()
                    .map(|chunk| {
                        if self.use_markdown_blocks {
                            serde_json::json!({
                                "type": "markdown",
                                "text": chunk
                            })
                        } else {
                            serde_json::json!({
                                "type": "section",
                                "text": {
                                    "type": "mrkdwn",
                                    "text": chunk
                                }
                            })
                        }
                    })
                    .collect();
                body["blocks"] = serde_json::Value::Array(blocks);
            }

            if let Some(ts) = self.outbound_thread_ts(message) {
                body["thread_ts"] = serde_json::json!(ts);
            }
            body
        };

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&body);
            anyhow::bail!("Slack chat.postMessage failed ({status}): {sanitized}");
        }

        // Slack returns 200 for most app-level errors; check JSON "ok" field
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack chat.postMessage failed: {err}");
        }

        Ok(())
    }

    fn supports_draft_updates(&self) -> bool {
        self.stream_drafts
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        if !self.stream_drafts {
            return Ok(None);
        }

        // Return a lazy placeholder — the real message is posted on the
        // first update_draft call so we don't show "..." before any output.
        let thread_ts = self.outbound_thread_ts(message).unwrap_or_default();
        let lazy_id = format!("{LAZY_DRAFT_PREFIX}{}:{}", message.recipient, thread_ts);
        Ok(Some(lazy_id))
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    async fn update_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        // with the first real content (instead of showing "...").
        if message_id.starts_with(LAZY_DRAFT_PREFIX)
            && self.resolve_draft_ts(message_id).await.is_none()
        {
            // First call — post the message. This blocks intentionally so the
            // ts is stored before any subsequent update_draft or finalize_draft.
            let _ = self.materialize_lazy_draft(message_id, text).await;
            self.last_draft_edit
                .lock()
                .expect("last_draft_edit lock")
                .insert(recipient.to_string(), Instant::now());
            return Ok(());
        }

        // Resolve the real ts (may be a lazy ID that was already materialized).
        let real_ts = match self.resolve_draft_ts(message_id).await {
            Some(ts) => ts,
            None => return Ok(()),
        };

        // Rate-limit edits per channel
        {
            let last_edits = self.last_draft_edit.lock().expect("last_draft_edit lock");
            if let Some(last_time) = last_edits.get(recipient) {
                let elapsed_ms = u64::try_from(last_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                if elapsed_ms < self.draft_update_interval_ms {
                    return Ok(());
                }
            }
        }

        // Mark as sent NOW (before the HTTP call) to prevent queuing
        // another update while this one is in flight.
        self.last_draft_edit
            .lock()
            .expect("last_draft_edit lock")
            .insert(recipient.to_string(), Instant::now());

        // Fire-and-forget: spawn the HTTP call so we don't block the
        // draft updater task (which would back-pressure the tool loop).
        let display_text = if text.len() > SLACK_MESSAGE_MAX_CHARS {
            text[..text
                .char_indices()
                .take_while(|(idx, _)| *idx < SLACK_MESSAGE_MAX_CHARS)
                .last()
                .map_or(0, |(idx, ch)| idx + ch.len_utf8())]
                .to_string()
        } else {
            text.to_string()
        };

        let client = self.http_client();
        let token = self.bot_token.clone();
        let channel = recipient.to_string();
        tokio::spawn(async move {
            let mut body = serde_json::json!({
                "channel": channel,
                "ts": real_ts,
                "text": &display_text,
            });
            if display_text.len() <= SLACK_MARKDOWN_BLOCK_MAX_CHARS {
                body["blocks"] = serde_json::json!([{
                    "type": "markdown",
                    "text": &display_text
                }]);
            }
            match client
                .post("https://slack.com/api/chat.update")
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(resp_body) = resp.json::<serde_json::Value>().await
                        && resp_body.get("ok") != Some(&serde_json::Value::Bool(true))
                    {
                        let err = resp_body
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("unknown");
                        tracing::debug!("Slack chat.update (draft) failed: {err}");
                    }
                }
                Err(e) => {
                    tracing::debug!("Slack chat.update (draft) HTTP error: {e}");
                }
            }
        });

        Ok(())
    }

    async fn update_draft_progress(
        &self,
        recipient: &str,
        _message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let status_line = text.trim().lines().last().unwrap_or("").trim();
        // Skip "Thinking..." — the typing indicator already conveys that.
        // Only show tool-related progress in the status bar.
        if status_line.is_empty() || status_line.starts_with("\u{1f914}") {
            return Ok(());
        }
        self.set_assistant_status(recipient, status_line).await;
        Ok(())
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        // Clean up rate-limit tracking and lazy draft map
        self.last_draft_edit
            .lock()
            .expect("last_draft_edit lock")
            .remove(recipient);

        // Extract thread_ts from the lazy draft ID ("lazy:{channel}:{thread_ts}")
        // so fallback sends preserve thread context.
        let draft_thread_ts = message_id
            .strip_prefix(LAZY_DRAFT_PREFIX)
            .and_then(|rest| rest.find(':').map(|pos| &rest[pos + 1..]))
            .filter(|ts| !ts.is_empty())
            .map(String::from);

        let real_ts = self.resolve_draft_ts(message_id).await;
        // Clean up lazy mapping
        self.lazy_draft_ts.lock().await.remove(message_id);

        let Some(real_ts) = real_ts else {
            // Draft was never materialized — just send as a fresh message
            let msg = SendMessage::new(text, recipient).in_thread(draft_thread_ts);
            return self.send(&msg).await;
        };

        // If text exceeds Slack limit, delete draft and send as regular message
        if text.len() > SLACK_MESSAGE_MAX_CHARS {
            let _ = self.delete_message(recipient, &real_ts).await;
            let msg = SendMessage::new(text, recipient).in_thread(draft_thread_ts);
            return self.send(&msg).await;
        }

        // Edit the draft with the final formatted content
        let mut body = serde_json::json!({
            "channel": recipient,
            "ts": real_ts,
            "text": text,
        });

        // Use markdown blocks for rich formatting when it fits
        if text.len() <= SLACK_MARKDOWN_BLOCK_MAX_CHARS {
            body["blocks"] = serde_json::json!([{
                "type": "markdown",
                "text": text
            }]);
        }

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.update")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let resp_body: serde_json::Value = resp.json().await?;
        if resp_body.get("ok") == Some(&serde_json::Value::Bool(true)) {
            return Ok(());
        }

        // Fallback: delete draft and send fresh
        let err = resp_body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");
        tracing::debug!("Slack chat.update (finalize) failed: {err}; falling back to delete+send");

        let _ = self.delete_message(recipient, &real_ts).await;
        let msg = SendMessage::new(text, recipient).in_thread(draft_thread_ts);
        self.send(&msg).await
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    async fn cancel_draft(&self, recipient: &str, message_id: &str) -> anyhow::Result<()> {
        self.last_draft_edit
            .lock()
            .expect("last_draft_edit lock")
            .remove(recipient);
        let real_ts = self.resolve_draft_ts(message_id).await;
        self.lazy_draft_ts.lock().await.remove(message_id);
        if let Some(ts) = real_ts {
            self.delete_message(recipient, &ts).await
        } else {
            Ok(())
        }
    }

    async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> anyhow::Result<()> {
        let ts = extract_slack_ts(message_id);
        let name = unicode_emoji_to_slack_name(emoji);

        let body = serde_json::json!({
            "channel": channel_id,
            "timestamp": ts,
            "name": name
        });

        let resp = self
            .http_client()
            .post("https://slack.com/api/reactions.add")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&text);
            anyhow::bail!("Slack reactions.add failed ({status}): {sanitized}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            if err != "already_reacted" {
                anyhow::bail!("Slack reactions.add failed: {err}");
            }
        }

        Ok(())
    }

    async fn remove_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> anyhow::Result<()> {
        let ts = extract_slack_ts(message_id);
        let name = unicode_emoji_to_slack_name(emoji);

        let body = serde_json::json!({
            "channel": channel_id,
            "timestamp": ts,
            "name": name
        });

        let resp = self
            .http_client()
            .post("https://slack.com/api/reactions.remove")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&text);
            anyhow::bail!("Slack reactions.remove failed ({status}): {sanitized}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            if err != "no_reaction" {
                anyhow::bail!("Slack reactions.remove failed: {err}");
            }
        }

        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let bot_user_id = self.get_bot_user_id().await.unwrap_or_default();
        let scoped_channels = self.scoped_channel_ids();
        if self.configured_app_token().is_some() {
            tracing::info!("Slack channel listening in Socket Mode");
            return self
                .listen_socket_mode(tx, &bot_user_id, scoped_channels)
                .await;
        }

        let mut discovered_channels: Vec<String> = Vec::new();
        let mut last_discovery = Instant::now();
        let mut last_ts_by_channel: HashMap<String, String> = HashMap::new();
        // Active thread tracker: thread_ts -> (channel_id, last_seen_reply_ts, last_activity)
        let mut active_threads: HashMap<String, (String, String, Instant)> = HashMap::new();

        if let Some(ref channel_ids) = scoped_channels {
            tracing::info!(
                "Slack channel listening on {} configured channel(s): {}",
                channel_ids.len(),
                channel_ids.join(", ")
            );
        } else {
            tracing::info!(
                "Slack channel_id/channel_ids not set (or wildcard only); listening across all accessible channels."
            );
        }

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            let target_channels = if let Some(ref channel_ids) = scoped_channels {
                channel_ids.clone()
            } else {
                if discovered_channels.is_empty()
                    || last_discovery.elapsed() >= Duration::from_secs(60)
                {
                    match self.list_accessible_channels().await {
                        Ok(channels) => {
                            if channels != discovered_channels {
                                tracing::info!(
                                    "Slack auto-discovery refreshed: listening on {} channel(s).",
                                    channels.len()
                                );
                            }
                            discovered_channels = channels;
                        }
                        Err(e) => {
                            tracing::warn!("Slack channel discovery failed: {e}");
                        }
                    }
                    last_discovery = Instant::now();
                }

                discovered_channels.clone()
            };

            if target_channels.is_empty() {
                tracing::debug!("Slack: no accessible channels discovered yet");
                continue;
            }

            for channel_id in target_channels {
                let had_cursor = last_ts_by_channel.contains_key(&channel_id);
                let bootstrap_ts = Self::slack_now_ts();
                let cursor_ts =
                    Self::ensure_poll_cursor(&mut last_ts_by_channel, &channel_id, &bootstrap_ts);
                if !had_cursor {
                    tracing::debug!(
                        "Slack: initialized cursor for channel {} at {} to prevent historical replay",
                        channel_id,
                        cursor_ts
                    );
                }
                let params = vec![
                    ("channel", channel_id.clone()),
                    ("limit", "10".to_string()),
                    ("oldest", cursor_ts),
                ];

                let Some(data) = self.fetch_history_with_retry(&channel_id, &params).await else {
                    continue;
                };

                if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
                    // Register thread parents discovered in channel history.
                    for (thread_ts, latest_reply) in Self::extract_active_threads(messages) {
                        let entry = active_threads.entry(thread_ts.clone()).or_insert_with(|| {
                            (channel_id.clone(), thread_ts.clone(), Instant::now())
                        });
                        if latest_reply > entry.1 {
                            entry.1 = latest_reply;
                        }
                        entry.2 = Instant::now();
                    }

                    // Messages come newest-first, reverse to process oldest first
                    for msg in messages.iter().rev() {
                        let subtype = msg.get("subtype").and_then(|value| value.as_str());
                        if !Self::is_supported_message_subtype(subtype) {
                            continue;
                        }
                        let ts = msg.get("ts").and_then(|t| t.as_str()).unwrap_or("");
                        let user = msg
                            .get("user")
                            .and_then(|u| u.as_str())
                            .unwrap_or("unknown");
                        let last_ts = last_ts_by_channel
                            .get(&channel_id)
                            .map(String::as_str)
                            .unwrap_or("");

                        // Skip bot's own messages
                        if user == bot_user_id {
                            continue;
                        }

                        // Sender validation
                        if !self.is_user_allowed(user) {
                            tracing::warn!(
                                "Slack: ignoring message from unauthorized user: {user}"
                            );
                            continue;
                        }

                        if ts <= last_ts {
                            continue;
                        }

                        let is_group_message = Self::is_group_channel_id(&channel_id);
                        let is_thread_reply =
                            msg.get("thread_ts").and_then(|v| v.as_str()).is_some();
                        let allow_sender_without_mention =
                            is_group_message && self.is_group_sender_trigger_enabled(user);
                        let require_mention = self.mention_only
                            && is_group_message
                            && !allow_sender_without_mention
                            && (!is_thread_reply || self.strict_mention_in_thread);
                        let Some(normalized_text) = self
                            .build_incoming_content(msg, require_mention, &bot_user_id)
                            .await
                        else {
                            continue;
                        };

                        last_ts_by_channel.insert(channel_id.clone(), ts.to_string());
                        let sender = self.resolve_sender_identity(user).await;

                        if let Some((token, response)) =
                            crate::util::parse_approval_reply(&normalized_text)
                        {
                            let mut map = self.pending_approvals.lock().await;
                            if let Some(ap_sender) = map.remove(&token) {
                                let _ = ap_sender.send(response);
                                continue;
                            }
                        }

                        let channel_msg = ChannelMessage {
                            id: format!("slack_{channel_id}_{ts}"),
                            sender,
                            reply_target: channel_id.clone(),
                            content: normalized_text,
                            channel: "slack".to_string(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                            thread_ts: if self.thread_replies {
                                Self::inbound_thread_ts(msg, ts)
                            } else {
                                Self::inbound_thread_ts_genuine_only(msg)
                            },
                            interruption_scope_id: Self::inbound_interruption_scope_id(msg, ts),
                            attachments: vec![],
                        };

                        if tx.send(channel_msg).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }

            // Poll active threads for new replies via conversations.replies.
            Self::evict_stale_threads(&mut active_threads, Instant::now());
            let thread_snapshot: Vec<(String, String, String)> = active_threads
                .iter()
                .map(|(thread_ts, (ch, last_reply, _))| {
                    (thread_ts.clone(), ch.clone(), last_reply.clone())
                })
                .collect();

            for (thread_ts, thread_channel_id, last_reply_ts) in thread_snapshot {
                let Some(data) = self
                    .fetch_thread_replies_with_retry(&thread_channel_id, &thread_ts, &last_reply_ts)
                    .await
                else {
                    continue;
                };

                let Some(replies) = data.get("messages").and_then(|m| m.as_array()) else {
                    continue;
                };

                for reply in replies {
                    let reply_ts = reply.get("ts").and_then(|v| v.as_str()).unwrap_or_default();
                    if reply_ts.is_empty() || reply_ts <= last_reply_ts.as_str() {
                        continue;
                    }
                    let subtype = reply.get("subtype").and_then(|v| v.as_str());
                    if !Self::is_supported_message_subtype(subtype) {
                        continue;
                    }

                    let user = reply
                        .get("user")
                        .and_then(|u| u.as_str())
                        .unwrap_or_default();
                    if user.is_empty() || user == bot_user_id {
                        continue;
                    }
                    if !self.is_user_allowed(user) {
                        continue;
                    }

                    // Thread replies never require a mention — we always respond
                    // inside threads the bot is already participating in.
                    let require_mention = false;
                    let Some(normalized_text) = self
                        .build_incoming_content(reply, require_mention, &bot_user_id)
                        .await
                    else {
                        continue;
                    };

                    // Update the last-seen reply ts for this thread.
                    if let Some(entry) = active_threads.get_mut(&thread_ts) {
                        if reply_ts > entry.1.as_str() {
                            entry.1 = reply_ts.to_string();
                        }
                        entry.2 = Instant::now();
                    }

                    let sender = self.resolve_sender_identity(user).await;

                    if let Some((token, response)) =
                        crate::util::parse_approval_reply(&normalized_text)
                    {
                        let mut map = self.pending_approvals.lock().await;
                        if let Some(ap_sender) = map.remove(&token) {
                            let _ = ap_sender.send(response);
                            continue;
                        }
                    }

                    let channel_msg = ChannelMessage {
                        id: format!("slack_{thread_channel_id}_{reply_ts}"),
                        sender,
                        reply_target: thread_channel_id.clone(),
                        content: normalized_text,
                        channel: "slack".to_string(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        thread_ts: Some(thread_ts.clone()),
                        interruption_scope_id: Some(thread_ts.clone()),
                        attachments: vec![],
                    };

                    if tx.send(channel_msg).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        let bot_ok = match self
            .http_client()
            .get("https://slack.com/api/auth.test")
            .bearer_auth(&self.bot_token)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                Self::slack_api_call_succeeded(status, &body)
            }
            Err(_) => false,
        };
        let socket_mode_enabled = self.configured_app_token().is_some();
        let socket_mode_ok = if socket_mode_enabled {
            self.open_socket_mode_url().await.is_ok()
        } else {
            true
        };
        Self::evaluate_health(bot_ok, socket_mode_enabled, socket_mode_ok)
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let thread_ts = {
            let map = self
                .active_assistant_thread
                .lock()
                .map_err(|e| anyhow::anyhow!("lock poisoned: {e}"))?;
            match map.get(recipient) {
                Some(ts) => ts.clone(),
                None => return Ok(()),
            }
        };

        let body = serde_json::json!({
            "channel_id": recipient,
            "thread_ts": thread_ts,
            "status": "is thinking...",
        });

        // Gracefully ignore errors — non-assistant contexts will return errors.
        if let Ok(resp) = self
            .http_client()
            .post("https://slack.com/api/assistant.threads.setStatus")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            && !resp.status().is_success()
        {
            tracing::debug!(
                "assistant.threads.setStatus returned {}; ignoring",
                resp.status()
            );
        }

        Ok(())
    }

    async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        // When using draft streaming, the final response is delivered via
        // chat.update (not chat.postMessage), so the Assistants API status
        // does not auto-clear. Explicitly clear it.
        if self.stream_drafts {
            self.set_assistant_status(recipient, "").await;
        }
        Ok(())
    }

    async fn request_approval(
        &self,
        recipient: &str,
        request: &ChannelApprovalRequest,
    ) -> anyhow::Result<Option<ChannelApprovalResponse>> {
        let token = crate::util::new_approval_token();

        let (tx, rx) = oneshot::channel();
        self.pending_approvals
            .lock()
            .await
            .insert(token.clone(), tx);

        // Socket Mode: send interactive Block Kit buttons.
        // Polling mode: send plain text with token-echo instructions.
        let send_result = if self.app_token.is_some() {
            let body = serde_json::json!({
                "channel": recipient,
                "text": format!("APPROVAL REQUIRED [{token}]\nTool: {}\nArgs: {}", request.tool_name, request.arguments_summary),
                "blocks": [{
                    "type": "section",
                    "text": {
                        "type": "mrkdwn",
                        "text": format!("*APPROVAL REQUIRED* [`{token}`]\n*Tool:* `{}`\n*Args:* {}", request.tool_name, request.arguments_summary),
                    }
                }, {
                    "type": "actions",
                    "elements": [
                        { "type": "button", "text": { "type": "plain_text", "text": "Approve" }, "action_id": format!("approval_{token}_approve"), "style": "primary" },
                        { "type": "button", "text": { "type": "plain_text", "text": "Deny" }, "action_id": format!("approval_{token}_deny"), "style": "danger" },
                        { "type": "button", "text": { "type": "plain_text", "text": "Always" }, "action_id": format!("approval_{token}_always") },
                    ]
                }]
            });
            self.http_client()
                .post("https://slack.com/api/chat.postMessage")
                .bearer_auth(&self.bot_token)
                .json(&body)
                .send()
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from)
        } else {
            self.send(&SendMessage::new(
                format!(
                    "APPROVAL REQUIRED [{token}]\nTool: {}\nArgs: {}\n\nReply: \"{token} yes\", \"{token} no\", or \"{token} always\"",
                    request.tool_name, request.arguments_summary,
                ),
                recipient,
            ))
            .await
        };

        if let Err(err) = send_result {
            self.pending_approvals.lock().await.remove(&token);
            return Err(err);
        }

        let response =
            match tokio::time::timeout(Duration::from_secs(self.approval_timeout_secs), rx).await {
                Ok(Ok(resp)) => resp,
                _ => {
                    self.pending_approvals.lock().await.remove(&token);
                    ChannelApprovalResponse::Deny
                }
            };
        Ok(Some(response))
    }
}
