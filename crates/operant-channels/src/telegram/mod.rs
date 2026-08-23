use anyhow::Context;
use async_trait::async_trait;
use directories::UserDirs;
use operant_api::channel::{Channel, ChannelMessage, SendMessage};
use operant_config::schema::{Config, StreamMode};
use operant_runtime::security::pairing::PairingGuard;
use parking_lot::Mutex;
use reqwest::multipart::{Form, Part};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::fs;
// Split modules (dedup pass 6) - paths unchanged.
mod channel_impl;
mod helpers;
#[cfg(test)]
mod tests;

pub(crate) use helpers::*;

/// Telegram channel — long-polls the Bot API for updates
pub struct TelegramChannel {
    bot_token: String,
    allowed_users: Arc<RwLock<Vec<String>>>,
    pairing: Option<PairingGuard>,
    client: reqwest::Client,
    typing_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stream_mode: StreamMode,
    draft_update_interval_ms: u64,
    last_draft_edit: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    mention_only: bool,
    bot_username: Mutex<Option<String>>,
    /// Base URL for the Telegram Bot API. Defaults to `https://api.telegram.org`.
    /// Override for local Bot API servers or testing.
    api_base: String,
    transcription: Option<operant_config::schema::TranscriptionConfig>,
    transcription_manager: Option<std::sync::Arc<super::transcription::TranscriptionManager>>,
    voice_transcriptions: Mutex<std::collections::HashMap<String, String>>,
    workspace_dir: Option<std::path::PathBuf>,
    ack_reactions: bool,
    tts_config: Option<operant_config::schema::TtsConfig>,
    voice_chats: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    pending_voice:
        Arc<std::sync::Mutex<std::collections::HashMap<String, (String, std::time::Instant)>>>,
    /// Per-channel proxy URL override.
    proxy_url: Option<String>,
    /// Pre-computed tool command specs (name, description) for bot command registration.
    tool_command_specs: Vec<(String, String)>,
    /// Pending approval requests: callback_data key → oneshot sender.
    /// `listen()` resolves these when a matching `callback_query` arrives.
    pending_approvals: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<
                String,
                tokio::sync::oneshot::Sender<operant_api::channel::ChannelApprovalResponse>,
            >,
        >,
    >,
    /// Pending multiple-choice questions: callback_data key → oneshot sender
    /// of the chosen option text plus the original option list (so the tap's
    /// index-based callback_data can be mapped back to text). `request_choice`
    /// registers an entry and `listen()` resolves it when the matching
    /// `choice:` callback_query arrives (hermes model-picker drill-down
    /// parity).
    pending_choices: Arc<tokio::sync::Mutex<std::collections::HashMap<String, PendingChoice>>>,
    /// Seconds to wait for the operator to tap an inline-keyboard button on a
    /// tool approval prompt before auto-denying. Configurable via
    /// `channels.telegram.approval_timeout_secs`. Default: 120.
    approval_timeout_secs: u64,
    /// When true, each DM chat gets its own forum topic and replies are
    /// routed into it (hermes `_setup_dm_topics` parity).
    dm_topics_enabled: bool,
    /// Name for the per-chat DM topic. Default: "General".
    dm_topic_name: String,
    /// Cache: chat_id -> forum topic thread_id. Populated lazily via
    /// `ensure_dm_topic`; persisted to the config dir state file.
    dm_topic_threads: std::sync::Mutex<std::collections::HashMap<String, i64>>,
    /// Most recent chat a message arrived from, recorded by `listen()`. Used
    /// by `request_choice` (which has no recipient parameter) to address the
    /// inline-keyboard question to the active conversation. Falls back to the
    /// generic send+listen flow when no message has arrived yet.
    last_chat_id: std::sync::Mutex<Option<String>>,
    /// When true, outbound messages include `link_preview_options` disabling
    /// Telegram link previews (hermes `disable_link_previews` parity).
    /// Default: true (previews on).
    link_previews_enabled: bool,
    /// Per-chat typing-indicator cooldown: chat_id -> instant until which
    /// `sendChatAction` refreshes are suppressed after a transient failure
    /// (rate limit, timeout). Hermes `_record_typing_cooldown` parity.
    typing_cooldown_secs: f64,
    typing_cooldown_until:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>>,
    /// Fallback IPs for the Bot API host (hermes `fallback_ips` parity). When
    /// non-empty, `http_client()` pins the host to one of these IPs (rotated
    /// per rebuild) instead of relying on DNS.
    fallback_ips: Vec<String>,
    /// Round-robin index into `fallback_ips`, shared with the watchdog loops.
    fallback_ip_index: Arc<parking_lot::Mutex<usize>>,
    /// Polling-recovery state shared between `listen()` and the watchdog
    /// loops: generation watch, pending-probe strikes, debounce timestamp.
    poll_recovery: PollRecoveryState,
    /// JoinHandles of the spawned watchdog loops (heartbeat + pending probe).
    /// Aborted on `listen()` return and on drop.
    poll_watchdogs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditMessageResult {
    Success,
    NotModified,
    Failed(reqwest::StatusCode),
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn supports_draft_updates(&self) -> bool {
        self.stream_mode != StreamMode::Off
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        if self.stream_mode == StreamMode::Off {
            return Ok(None);
        }

        let (chat_id, thread_id) = Self::parse_reply_target(&message.recipient);
        let initial_text = if message.content.is_empty() {
            "...".to_string()
        } else {
            message.content.clone()
        };

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": initial_text,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }
        if let Some(lp) = self.link_preview_json() {
            body["link_preview_options"] = lp;
        }

        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram sendMessage (draft) failed: {err}");
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let message_id = resp_json
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|id| id.as_i64())
            .map(|id| id.to_string());

        self.last_draft_edit
            .lock()
            .insert(chat_id.to_string(), std::time::Instant::now());

        Ok(message_id)
    }

    async fn update_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let (chat_id, _) = Self::parse_reply_target(recipient);

        // Rate-limit edits per chat
        {
            let last_edits = self.last_draft_edit.lock();
            if let Some(last_time) = last_edits.get(&chat_id) {
                let elapsed = u64::try_from(last_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                if elapsed < self.draft_update_interval_ms {
                    return Ok(());
                }
            }
        }

        // Truncate to Telegram limit for mid-stream edits (UTF-8 safe)
        let display_text = if text.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            let mut end = 0;
            for (idx, ch) in text.char_indices() {
                let next = idx + ch.len_utf8();
                if next > TELEGRAM_MAX_MESSAGE_LENGTH {
                    break;
                }
                end = next;
            }
            &text[..end]
        } else {
            text
        };

        let message_id_parsed = match message_id.parse::<i64>() {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("Invalid Telegram message_id '{message_id}': {e}");
                return Ok(());
            }
        };

        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id_parsed,
            "text": display_text,
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            self.last_draft_edit
                .lock()
                .insert(chat_id.clone(), std::time::Instant::now());
        } else {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            tracing::debug!("Telegram editMessageText failed ({status}): {err}");
        }

        Ok(())
    }

    async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let text = &strip_tool_call_tags(text);
        let (chat_id, thread_id) = Self::parse_reply_target(recipient);

        // Queue TTS voice reply — immediate mode since text is already final
        self.try_queue_voice_reply(recipient, text, true);

        // Clean up rate-limit tracking for this chat
        self.last_draft_edit.lock().remove(&chat_id);

        // Parse attachments before processing
        let (text_without_markers, attachments) = parse_attachment_markers(text);

        // Parse message ID once for reuse
        let msg_id = match message_id.parse::<i64>() {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!("Invalid Telegram message_id '{message_id}': {e}");
                None
            }
        };

        // If we have attachments, delete the draft and send fresh messages
        // (Telegram editMessageText can't add attachments)
        if !attachments.is_empty() {
            // Delete the draft message
            if let Some(id) = msg_id {
                let _ = self
                    .client
                    .post(self.api_url("deleteMessage"))
                    .json(&serde_json::json!({
                        "chat_id": chat_id,
                        "message_id": id,
                    }))
                    .send()
                    .await;
            }

            // Send text without markers
            if !text_without_markers.is_empty() {
                self.send_text_chunks(&text_without_markers, &chat_id, thread_id.as_deref())
                    .await?;
            }

            // Send attachments
            for attachment in &attachments {
                self.send_attachment(&chat_id, thread_id.as_deref(), attachment)
                    .await?;
            }

            return Ok(());
        }

        // If text exceeds limit, delete draft and send as chunked messages
        if text.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            if let Some(id) = msg_id {
                let _ = self
                    .client
                    .post(self.api_url("deleteMessage"))
                    .json(&serde_json::json!({
                        "chat_id": chat_id,
                        "message_id": id,
                    }))
                    .send()
                    .await;
            }

            // Fall back to chunked send
            return self
                .send_text_chunks(text, &chat_id, thread_id.as_deref())
                .await;
        }

        let Some(id) = msg_id else {
            return self
                .send_text_chunks(text, &chat_id, thread_id.as_deref())
                .await;
        };

        // Try editing with HTML formatting
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": id,
            "text": Self::markdown_to_telegram_html(text),
            "parse_mode": "HTML",
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?;

        match Self::classify_edit_message_response(resp).await {
            EditMessageResult::Success | EditMessageResult::NotModified => return Ok(()),
            EditMessageResult::Failed(status) => {
                tracing::debug!(
                    status = ?status,
                    "Telegram finalize_draft HTML edit failed; retrying without parse_mode"
                );
            }
        }

        // HTML failed — retry without parse_mode
        let plain_body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": id,
            "text": text,
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&plain_body)
            .send()
            .await?;

        match Self::classify_edit_message_response(resp).await {
            EditMessageResult::Success | EditMessageResult::NotModified => return Ok(()),
            EditMessageResult::Failed(status) => {
                tracing::warn!(
                    status = ?status,
                    "Telegram finalize_draft plain edit failed; attempting delete+send fallback"
                );
            }
        }

        let delete_resp = self
            .client
            .post(self.api_url("deleteMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "message_id": id,
            }))
            .send()
            .await;

        match delete_resp {
            Ok(resp) if resp.status().is_success() => {
                self.send_text_chunks(text, &chat_id, thread_id.as_deref())
                    .await
            }
            Ok(resp) => {
                tracing::warn!(
                    status = ?resp.status(),
                    "Telegram finalize_draft delete failed; skipping sendMessage to avoid duplicate"
                );
                Ok(())
            }
            Err(err) => {
                tracing::warn!(
                    "Telegram finalize_draft delete request failed: {err}; skipping sendMessage to avoid duplicate"
                );
                Ok(())
            }
        }
    }

    async fn cancel_draft(&self, recipient: &str, message_id: &str) -> anyhow::Result<()> {
        let (chat_id, _) = Self::parse_reply_target(recipient);
        self.last_draft_edit.lock().remove(&chat_id);

        let message_id = match message_id.parse::<i64>() {
            Ok(id) => id,
            Err(e) => {
                tracing::debug!("Invalid Telegram draft message_id '{message_id}': {e}");
                return Ok(());
            }
        };

        let response = self
            .client
            .post(self.api_url("deleteMessage"))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::debug!("Telegram deleteMessage failed ({status}): {body}");
        }

        Ok(())
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        // Strip tool_call tags before processing to prevent Markdown parsing failures
        let content = strip_tool_call_tags(&message.content);

        // Parse recipient: "chat_id" or "chat_id:thread_id" format
        let (chat_id, thread_id) = match message.recipient.split_once(':') {
            Some((chat, thread)) => (chat, Some(thread)),
            None => (message.recipient.as_str(), None),
        };

        // Voice chat mode: send text normally AND queue a voice note of the
        // final answer. Text in → text out. Voice in → text + voice out.
        self.try_queue_voice_reply(&message.recipient, &content, false);

        // Always send text reply (voice chat gets both text and voice)
        let (text_without_markers, attachments) = parse_attachment_markers(&content);

        if !attachments.is_empty() {
            if !text_without_markers.is_empty() {
                self.send_text_chunks(&text_without_markers, chat_id, thread_id)
                    .await?;
            }

            for attachment in &attachments {
                self.send_attachment(chat_id, thread_id, attachment).await?;
            }

            return Ok(());
        }

        if let Some(attachment) = parse_path_only_attachment(&content) {
            self.send_attachment(chat_id, thread_id, &attachment)
                .await?;
            return Ok(());
        }

        self.send_text_chunks(&content, chat_id, thread_id).await
    }
    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let mut offset: i64 = 0;

        if self.mention_only {
            let _ = self.get_bot_username().await;
        }

        tracing::info!("Telegram channel listening for messages...");

        // Restore persisted DM-topic thread ids across restarts.
        self.load_dm_topic_state();

        // T4 polling resilience: spawn the watchdog loops that detect wedged
        // TCP sockets (heartbeat getMe probe) and a stuck getUpdates consumer
        // (pending-update probe). They share the recovery watch channel and
        // bump the generation to restart the long-poll loop below.
        self.spawn_poll_watchdogs();

        // Startup probe / reconnect verification: claim the getUpdates slot
        // with a timeout=0 request. A previous daemon's 30-second poll may
        // still be active on Telegram's server; we retry until the slot is
        // ours. Fatal errors (401/403) stop the channel; 409 backs off past
        // the competing poll window; transient errors retry (hermes
        // `_verify_polling_after_reconnect` parity).
        if let Err(e) = self.verify_polling_slot(&mut offset).await {
            self.abort_poll_watchdogs();
            return Err(e);
        }

        tracing::debug!("Polling slot verified; entering main long-poll loop.");

        self.register_bot_commands().await;

        // Set by a recovery trigger, a client-side poll timeout, or a
        // transient API error: re-verify the slot before resuming the 30s
        // long-poll so we never race a competing session.
        let mut needs_verify = false;

        loop {
            if needs_verify {
                match self.verify_polling_slot(&mut offset).await {
                    Ok(()) => needs_verify = false,
                    Err(e) => {
                        self.abort_poll_watchdogs();
                        return Err(e);
                    }
                }
            }

            if self.mention_only {
                let missing_username = self.bot_username.lock().is_none();
                if missing_username {
                    let _ = self.get_bot_username().await;
                }
            }

            let url = self.api_url("getUpdates");
            let body = serde_json::json!({
                "offset": offset,
                "timeout": 30,
                "allowed_updates": ["message", "callback_query"]
            });

            // Subscribe before sending so a recovery trigger that fires while
            // this request is in flight abandons it (hermes generation
            // verifier parity). A fresh subscription only sees NEW bumps.
            let mut recovery_rx = self.poll_recovery.generation.subscribe();
            let request = self.http_client().post(&url).json(&body).send();
            let timed =
                tokio::time::timeout(Duration::from_secs(POLL_CLIENT_TIMEOUT_SECS), request);
            let resp = tokio::select! {
                r = timed => match r {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        tracing::warn!("Telegram poll error: {e}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    Err(_) => {
                        // Client-side ceiling exceeded: the TCP path is wedged
                        // (CLOSE-WAIT) even though the API-level 30s timeout
                        // never surfaced an error. Re-verify the slot before
                        // resuming (hermes heartbeat/CLOSE-WAIT parity).
                        tracing::warn!(
                            "Telegram long-poll request timed out client-side; \
            connection suspect — re-verifying slot"
                        );
                        needs_verify = true;
                        tokio::time::sleep(Duration::from_secs(POLL_WEDGE_BACKOFF_SECS)).await;
                        continue;
                    }
                },
                _ = recovery_rx.changed() => {
                    tracing::warn!(
                        "Telegram poll recovery requested; re-verifying slot \
            before resuming long-poll"
                    );
                    needs_verify = true;
                    continue;
                }
            };

            let data: serde_json::Value = match resp.json().await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("Telegram parse error: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let ok = data
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if !ok {
                let error_code = data
                    .get("error_code")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                let description = data
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown Telegram API error");

                match PollErrorClass::from_error_code(error_code) {
                    PollErrorClass::Fatal => {
                        // Auth/validation errors must not churn the loop (hermes
                        // `_looks_like_network_error` parity). Stop polling.
                        tracing::error!(
                            "Telegram getUpdates fatal error (code={error_code}): \
{description}; stopping polling"
                        );
                        self.abort_poll_watchdogs();
                        anyhow::bail!(
                            "telegram getUpdates fatal error (code={error_code}): {description}"
                        );
                    }
                    PollErrorClass::Conflict => {
                        tracing::warn!(
                            "Telegram polling conflict (409): {description}. \
Ensure only one `operant` process is using this bot token."
                        );
                        // Back off for 35 seconds — longer than Telegram's 30-second poll
                        // timeout — so any competing session (e.g. a stale connection from
                        // a previous daemon) has time to expire before we retry.
                        tokio::time::sleep(Duration::from_secs(35)).await;
                        needs_verify = true;
                    }
                    PollErrorClass::Network => {
                        tracing::warn!(
                            "Telegram getUpdates API error (code={}): {description}",
                            error_code
                        );
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        needs_verify = true;
                    }
                }
                continue;
            }

            // A healthy response proves the consumer is draining the queue;
            // reset the pending-update probe strikes so a single in-flight
            // update between probes never escalates (hermes parity).
            *self.poll_recovery.pending_stuck_strikes.lock() = 0;

            if let Some(results) = data.get("result").and_then(serde_json::Value::as_array) {
                for update in results {
                    // Advance offset past this update
                    if let Some(uid) = update.get("update_id").and_then(serde_json::Value::as_i64) {
                        offset = uid + 1;
                    }

                    // ── Handle callback_query (inline keyboard taps) ──
                    if let Some(cb) = update.get("callback_query") {
                        let cb_id = cb
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        let cb_data = cb
                            .get("data")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();

                        if let Some(rest) = cb_data.strip_prefix("approval:")
                            && let Some((approval_id, action)) = rest.rsplit_once(':')
                        {
                            let response = match action {
                                "approve" => {
                                    Some(operant_api::channel::ChannelApprovalResponse::Approve)
                                }
                                "always" => Some(
                                    operant_api::channel::ChannelApprovalResponse::AlwaysApprove,
                                ),
                                "deny" => Some(operant_api::channel::ChannelApprovalResponse::Deny),
                                other => {
                                    tracing::warn!("Unknown approval callback action: {other}");
                                    None
                                }
                            };

                            if let Some(resp) = response
                                && let Some(sender) =
                                    self.pending_approvals.lock().await.remove(approval_id)
                            {
                                let _ = sender.send(resp);
                            }

                            // Answer the callback query to dismiss the spinner.
                            let answer_text = match action {
                                "approve" => "✅ Approved",
                                "always" => "✅✅ Always approved",
                                "deny" => "❌ Denied",
                                _ => "⚠️ Unknown action",
                            };
                            let answer_body = serde_json::json!({
                                "callback_query_id": cb_id,
                                "text": answer_text,
                            });
                            if let Err(e) = self
                                .http_client()
                                .post(self.api_url("answerCallbackQuery"))
                                .json(&answer_body)
                                .send()
                                .await
                            {
                                tracing::warn!("answerCallbackQuery failed: {e}");
                            }
                        }

                        // ── Multiple-choice taps (`choice:` prefix) ──
                        // Resolve the pending `request_choice` oneshot with the
                        // selected option text. Index-based callback_data keeps
                        // payloads under Telegram's 64-byte limit.
                        if let Some(rest) = cb_data.strip_prefix("choice:")
                            && let Some((choice_id, idx_str)) = rest.rsplit_once(':')
                            && let Ok(idx) = idx_str.parse::<usize>()
                        {
                            let answered = if let Some((sender, choices)) =
                                self.pending_choices.lock().await.remove(choice_id)
                            {
                                let choice_text = choices
                                    .get(idx)
                                    .cloned()
                                    .unwrap_or_else(|| format!("option {}", idx + 1));
                                let _ = sender.send(choice_text);
                                true
                            } else {
                                false
                            };
                            let answer_body = serde_json::json!({
                                "callback_query_id": cb_id,
                                "text": if answered { "✅ Selected" } else { "⚠️ Expired" },
                            });
                            if let Err(e) = self
                                .http_client()
                                .post(self.api_url("answerCallbackQuery"))
                                .json(&answer_body)
                                .send()
                                .await
                            {
                                tracing::warn!("answerCallbackQuery (choice) failed: {e}");
                            }
                        }

                        continue; // callback_query is not a regular message
                    }

                    let mut msg = if let Some(m) = self.parse_update_message(update) {
                        m
                    } else if let Some(m) = self.try_parse_voice_message(update).await {
                        m
                    } else if let Some(m) = self.try_parse_attachment_message(update).await {
                        m
                    } else {
                        Box::pin(self.handle_unauthorized_message(update)).await;
                        continue;
                    };

                    if self.ack_reactions
                        && let Some((reaction_chat_id, reaction_message_id)) =
                            Self::extract_update_message_target(update)
                    {
                        self.try_add_ack_reaction_nonblocking(
                            reaction_chat_id,
                            reaction_message_id,
                        );
                    }

                    // Record the chat for `request_choice` (no recipient
                    // parameter) before DM-topic rewriting mutates the target.
                    if let Some((chat_id, _)) = Self::extract_update_message_target(update) {
                        *self.last_chat_id.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(chat_id);
                    }

                    // DM topics: when enabled and this is a private-chat
                    // message, ensure the chat's forum topic exists and route
                    // the reply into it (hermes `ensure_dm_topic` parity).
                    // The typing indicator still goes to the bare chat id.
                    let typing_chat_id = if self.dm_topics_enabled {
                        if let Some(tid) = self.ensure_dm_topic(&msg.reply_target).await {
                            msg.reply_target = format!("{}:{}", msg.reply_target, tid);
                            msg.thread_ts = Some(tid.to_string());
                        }
                        msg.reply_target
                            .split_once(':')
                            .map(|(chat, _)| chat.to_string())
                            .unwrap_or_else(|| msg.reply_target.clone())
                    } else {
                        msg.reply_target.clone()
                    };

                    // Send "typing" indicator immediately when we receive a message
                    let typing_body = serde_json::json!({
                        "chat_id": typing_chat_id,
                        "action": "typing"
                    });
                    let _ = self
                        .http_client()
                        .post(self.api_url("sendChatAction"))
                        .json(&typing_body)
                        .send()
                        .await; // Ignore errors for typing indicator

                    if tx.send(msg).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        let timeout_duration = Duration::from_secs(5);

        match tokio::time::timeout(
            timeout_duration,
            self.http_client().get(self.api_url("getMe")).send(),
        )
        .await
        {
            Ok(Ok(resp)) => resp.status().is_success(),
            Ok(Err(e)) => {
                tracing::debug!("Telegram health check failed: {e}");
                false
            }
            Err(_) => {
                tracing::debug!("Telegram health check timed out after 5s");
                false
            }
        }
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.stop_typing(recipient).await?;

        let client = self.http_client();
        let url = self.api_url("sendChatAction");
        let chat_id = recipient.to_string();
        let cooldown_secs = self.typing_cooldown_secs;
        let cooldown_until = Arc::clone(&self.typing_cooldown_until);

        let handle = tokio::spawn(async move {
            loop {
                // Suppress refreshes while this chat is in cooldown after a
                // transient failure (hermes `_typing_in_cooldown` parity):
                // hammering sendChatAction into a rate limit makes it worse
                // and spams the API/logs.
                let in_cooldown = {
                    let mut map = cooldown_until.lock().unwrap_or_else(|e| e.into_inner());
                    match map.get(&chat_id) {
                        Some(until) if *until > std::time::Instant::now() => true,
                        _ => {
                            map.remove(&chat_id);
                            false
                        }
                    }
                };
                if !in_cooldown {
                    let body = serde_json::json!({
                        "chat_id": &chat_id,
                        "action": "typing"
                    });
                    match client.post(&url).json(&body).send().await {
                        Ok(r) if r.status().is_success() => {
                            // Success clears any stale cooldown.
                            cooldown_until
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&chat_id);
                        }
                        Ok(r) => {
                            let status = r.status();
                            tracing::debug!(
                                "Telegram typing indicator failed ({status}); suppressing refreshes for {cooldown_secs}s"
                            );
                            cooldown_until
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(
                                    chat_id.clone(),
                                    std::time::Instant::now()
                                        + Duration::from_secs_f64(cooldown_secs),
                                );
                        }
                        Err(e) => {
                            tracing::debug!(
                                "Telegram typing indicator error: {e}; suppressing refreshes for {cooldown_secs}s"
                            );
                            cooldown_until
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(
                                    chat_id.clone(),
                                    std::time::Instant::now()
                                        + Duration::from_secs_f64(cooldown_secs),
                                );
                        }
                    }
                }
                // Telegram typing indicator expires after 5s; refresh at 4s
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });

        let mut guard = self.typing_handle.lock();
        *guard = Some(handle);

        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        let mut guard = self.typing_handle.lock();
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn request_approval(
        &self,
        recipient: &str,
        request: &operant_api::channel::ChannelApprovalRequest,
    ) -> anyhow::Result<Option<operant_api::channel::ChannelApprovalResponse>> {
        use operant_api::channel::ChannelApprovalResponse;

        // Parse recipient for chat_id + optional thread_id ("chat_id:thread_id" format).
        let (chat_id, thread_id) = recipient
            .split_once(':')
            .map_or((recipient, None), |(c, t)| (c, Some(t)));

        // Unique key embedded in callback_data so listen() can route the tap.
        let approval_id = uuid::Uuid::new_v4().to_string();

        let tool = Self::escape_html(&request.tool_name);
        let args = Self::escape_html(&request.arguments_summary);
        let text = format!(
            "\u{1f527} <b>Tool approval required</b>\n\n\
             Tool: <code>{tool}</code>\n\
             {args}\n\n\
             Tap a button below:",
        );

        let reply_markup = serde_json::json!({
            "inline_keyboard": [[
                { "text": "✅ Approve",  "callback_data": format!("approval:{}:approve", approval_id) },
                { "text": "❌ Deny",     "callback_data": format!("approval:{}:deny", approval_id) },
                { "text": "✅✅ Always", "callback_data": format!("approval:{}:always", approval_id) },
            ]]
        });

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": reply_markup,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        // Register the oneshot BEFORE sending the message to avoid a race
        // where the user taps the button before the sender is in the map.
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_approvals
            .lock()
            .await
            .insert(approval_id.clone(), tx);

        let resp = self
            .http_client()
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => {
                self.pending_approvals.lock().await.remove(&approval_id);
                let status = r.status();
                let err = r.text().await.unwrap_or_default();
                anyhow::bail!("Telegram sendMessage (approval) failed ({status}): {err}");
            }
            Err(e) => {
                self.pending_approvals.lock().await.remove(&approval_id);
                return Err(e.into());
            }
        }

        // Wait for the user to tap a button. Timeout is configurable via
        // `channels.telegram.approval_timeout_secs` (default 120s).
        let result =
            match tokio::time::timeout(Duration::from_secs(self.approval_timeout_secs), rx).await {
                Ok(Ok(response)) => Some(response),
                _ => {
                    // Timeout or sender dropped — clean up and deny.
                    self.pending_approvals.lock().await.remove(&approval_id);
                    Some(ChannelApprovalResponse::Deny)
                }
            };

        Ok(result)
    }

    async fn request_choice(
        &self,
        question: &str,
        choices: &[String],
        timeout: std::time::Duration,
    ) -> anyhow::Result<Option<String>> {
        if choices.is_empty() {
            anyhow::bail!("TelegramChannel.request_choice requires at least one choice");
        }

        // `request_choice` carries no recipient, so address the question to
        // the most recent chat a message arrived from. If none has been seen
        // yet (channel never listened), signal the caller to fall back to the
        // generic send + listen flow.
        let chat_id = match self
            .last_chat_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            Some(c) => c,
            None => return Ok(None),
        };

        // Unique key embedded in callback_data so listen() can route the tap.
        let choice_id = uuid::Uuid::new_v4().to_string();
        let question_html = Self::escape_html(question);
        let text = format!("\u{2753} <b>{question_html}</b>\n\nTap an option below:");

        // One button per choice; callback_data carries the choice index so
        // the selected option text is recovered without echoing user content
        // into callback_data (which Telegram limits to 64 bytes).
        let buttons: Vec<serde_json::Value> = choices
            .iter()
            .enumerate()
            .map(|(idx, choice)| {
                serde_json::json!({ "text": choice, "callback_data": format!("choice:{choice_id}:{idx}") })
            })
            .collect();
        // Wrap in a single row for small choice sets; split at 4 for larger.
        let rows: Vec<Vec<serde_json::Value>> = buttons.chunks(4).map(|c| c.to_vec()).collect();
        let reply_markup = serde_json::json!({ "inline_keyboard": rows });

        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": reply_markup,
        });

        // Register the oneshot BEFORE sending to avoid a race where the user
        // taps before the sender is in the map. The choice list rides along so
        // `listen()` can map the index-based callback back to text.
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_choices
            .lock()
            .await
            .insert(choice_id.clone(), (tx, choices.to_vec()));

        let resp = self
            .http_client()
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => {
                self.pending_choices.lock().await.remove(&choice_id);
                let status = r.status();
                let err = r.text().await.unwrap_or_default();
                anyhow::bail!("Telegram sendMessage (choice) failed ({status}): {err}");
            }
            Err(e) => {
                self.pending_choices.lock().await.remove(&choice_id);
                return Err(e.into());
            }
        }

        // Wait for the user to tap a button, honoring the caller's timeout.
        let result = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(choice)) => Some(choice),
            _ => {
                // Timeout or sender dropped — clean up.
                self.pending_choices.lock().await.remove(&choice_id);
                None
            }
        };

        Ok(result)
    }
}

/// Abort the polling watchdog tasks when the channel is dropped, so an
/// externally-aborted `listen()` never leaves orphaned probes hitting the API.
impl Drop for TelegramChannel {
    fn drop(&mut self) {
        for handle in self.poll_watchdogs.get_mut().drain(..) {
            handle.abort();
        }
    }
}

/// Heartbeat loop (hermes `_polling_heartbeat_loop` parity): probe `getMe` on
/// the general request path every interval with a short deadline. A long-poll
/// waiting for Telegram's 30-second window never surfaces a dead TCP socket
/// (CLOSE-WAIT), but a connect-level failure here does — and triggers a polling
/// recovery, so the main loop re-verifies the slot and resumes on a fresh
/// connection.
async fn poll_heartbeat_loop(
    recovery: &PollRecoveryState,
    api_base: &str,
    token: &str,
    proxy: Option<&str>,
    fallbacks: &[String],
    fallback_index: &parking_lot::Mutex<usize>,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(POLLING_HEARTBEAT_INTERVAL_SECS)).await;
        let url = format!("{api_base}/bot{token}/getMe");
        let probe = async {
            let client = build_telegram_api_client(api_base, proxy, fallbacks, fallback_index);
            match client.get(&url).send().await {
                Ok(resp) => resp
                    .error_for_status()
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        };
        match tokio::time::timeout(POLLING_HEARTBEAT_PROBE_TIMEOUT, probe).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                recovery.trigger(&format!("heartbeat getMe failed: {e}"));
            }
            Err(_) => {
                recovery.trigger("heartbeat getMe timed out");
            }
        }
    }
}

/// Pending-update probe loop (hermes `_probe_pending_updates` parity): read
/// `getWebhookInfo().pending_update_count` on the general path. A queue that
/// stays at/above the threshold across two consecutive probes while we believe
/// we are polling means the long-poll consumer is wedged (updates are queuing
/// server-side but never delivered) — escalate to a polling recovery. Any
/// healthy probe resets the strikes.
async fn probe_pending_updates_loop(
    recovery: &PollRecoveryState,
    api_base: &str,
    token: &str,
    proxy: Option<&str>,
    fallbacks: &[String],
    fallback_index: &parking_lot::Mutex<usize>,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(POLLING_PENDING_PROBE_INTERVAL_SECS)).await;
        let url = format!("{api_base}/bot{token}/getWebhookInfo");
        let client = build_telegram_api_client(api_base, proxy, fallbacks, fallback_index);
        let pending = match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|data| {
                    data.get("ok")
                        .and_then(serde_json::Value::as_bool)
                        .filter(|ok| *ok)
                        .and_then(|_| {
                            data.pointer("/result/pending_update_count")
                                .and_then(serde_json::Value::as_i64)
                        })
                }),
            _ => None,
        };
        let Some(pending) = pending else {
            // Transient getWebhookInfo failure — never escalate on the probe
            // itself failing, only on a stuck queue.
            continue;
        };
        let escalate = probe_pending_escalate(
            pending,
            &mut recovery.pending_stuck_strikes.lock(),
            POLLING_PENDING_STUCK_THRESHOLD,
            POLLING_PENDING_STUCK_STRIKES,
        );
        *recovery.last_pending_count.lock() = pending;
        if escalate {
            recovery.trigger(&format!(
                "pending update queue wedged (pending_update_count={pending})"
            ));
        }
    }
}
