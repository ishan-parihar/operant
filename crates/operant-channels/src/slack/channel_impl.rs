//! Inherent methods on `SlackChannel` extracted verbatim.

use anyhow::Context;
use base64::Engine as _;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use operant_api::channel::{ChannelApprovalResponse, ChannelMessage, SendMessage};
use reqwest::header::HeaderMap;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::*;

impl SlackChannel {
    pub fn new(
        bot_token: String,
        app_token: Option<String>,
        channel_ids: Vec<String>,
        allowed_users: Vec<String>,
    ) -> Self {
        Self {
            bot_token,
            app_token,
            channel_ids,
            allowed_users,
            thread_replies: true,
            mention_only: false,
            strict_mention_in_thread: false,
            group_reply_allowed_sender_ids: Vec::new(),
            user_display_name_cache: Mutex::new(HashMap::new()),
            workspace_dir: None,
            active_assistant_thread: Mutex::new(HashMap::new()),
            use_markdown_blocks: false,
            proxy_url: None,
            transcription: None,
            transcription_manager: None,
            stream_drafts: false,
            draft_update_interval_ms: SLACK_DRAFT_UPDATE_INTERVAL_MS,
            last_draft_edit: Mutex::new(HashMap::new()),
            lazy_draft_ts: tokio::sync::Mutex::new(HashMap::new()),
            cancel_reaction: None,
            pending_approvals: Arc::new(AsyncMutex::new(HashMap::new())),
            approval_timeout_secs: 300,
        }
    }

    /// Configure group-chat trigger policy.
    pub fn with_group_reply_policy(
        mut self,
        mention_only: bool,
        allowed_sender_ids: Vec<String>,
    ) -> Self {
        self.mention_only = mention_only;
        self.group_reply_allowed_sender_ids =
            Self::normalize_group_reply_allowed_sender_ids(allowed_sender_ids);
        self
    }

    /// Configure whether outbound replies stay in the originating Slack thread.
    pub fn with_thread_replies(mut self, thread_replies: bool) -> Self {
        self.thread_replies = thread_replies;
        self
    }

    /// When true (and `mention_only` is also true), require an @-mention
    /// for messages inside a Slack thread too. Default: false (threads
    /// bypass the mention requirement so follow-ups don't need @).
    pub fn with_strict_mention_in_thread(mut self, strict: bool) -> Self {
        self.strict_mention_in_thread = strict;
        self
    }

    /// Configure workspace directory used for persisting inbound Slack attachments.
    pub fn with_workspace_dir(mut self, dir: PathBuf) -> Self {
        self.workspace_dir = Some(dir);
        self
    }

    /// Set a per-channel proxy URL that overrides the global proxy config.
    /// Enable the newer `markdown` block type for richer formatting.
    /// Only use this if your Slack workspace supports it.
    pub fn with_markdown_blocks(mut self, enabled: bool) -> Self {
        self.use_markdown_blocks = enabled;
        self
    }

    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }

    pub fn with_approval_timeout_secs(mut self, secs: u64) -> Self {
        self.approval_timeout_secs = secs;
        self
    }

    /// Configure voice transcription for audio file attachments.
    pub fn with_transcription(
        mut self,
        config: operant_config::schema::TranscriptionConfig,
    ) -> Self {
        if !config.enabled {
            return self;
        }
        match crate::transcription::TranscriptionManager::new(&config) {
            Ok(m) => {
                self.transcription_manager = Some(std::sync::Arc::new(m));
                self.transcription = Some(config);
            }
            Err(e) => {
                tracing::warn!(
                    "transcription manager init failed, voice transcription disabled: {e}"
                );
            }
        }
        self
    }

    /// Enable progressive draft message streaming via `chat.update`.
    pub fn with_streaming(mut self, enabled: bool, interval_ms: u64) -> Self {
        self.stream_drafts = enabled;
        if interval_ms > 0 {
            self.draft_update_interval_ms = interval_ms;
        }
        self
    }

    /// Set the emoji reaction name that cancels an in-flight request.
    pub fn with_cancel_reaction(mut self, reaction: Option<String>) -> Self {
        self.cancel_reaction = reaction;
        self
    }

    /// Delete a Slack message by channel + timestamp.
    pub(crate) async fn delete_message(&self, channel_id: &str, ts: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "channel": channel_id,
            "ts": ts,
        });

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.delete")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let resp_body: serde_json::Value = resp.json().await?;
        if resp_body.get("ok") != Some(&serde_json::Value::Bool(true)) {
            let err = resp_body
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            tracing::debug!("Slack chat.delete failed: {err}");
        }

        Ok(())
    }

    /// Resolve a possibly-lazy draft ID to a real Slack message ts.
    /// If the ID starts with `LAZY_DRAFT_PREFIX`, the message hasn't been
    /// posted yet — this method returns `None`. Otherwise returns the ID as-is,
    /// or the previously resolved real ts from the lazy map.
    pub(crate) async fn resolve_draft_ts(&self, message_id: &str) -> Option<String> {
        if !message_id.starts_with(LAZY_DRAFT_PREFIX) {
            return Some(message_id.to_string());
        }
        self.lazy_draft_ts.lock().await.get(message_id).cloned()
    }

    /// Post the initial draft message and store the mapping from
    /// lazy placeholder ID to real Slack ts.
    pub(crate) async fn materialize_lazy_draft(
        &self,
        lazy_id: &str,
        text: &str,
    ) -> anyhow::Result<Option<String>> {
        // Parse channel + thread_ts from the lazy ID: "lazy:{channel}:{thread_ts}"
        let rest = lazy_id.strip_prefix(LAZY_DRAFT_PREFIX).unwrap_or(lazy_id);
        let (channel_id, thread_ts) = match rest.find(':') {
            Some(pos) => {
                let ts = &rest[pos + 1..];
                (&rest[..pos], if ts.is_empty() { None } else { Some(ts) })
            }
            None => (rest, None),
        };

        let mut body = serde_json::json!({
            "channel": channel_id,
            "text": text,
        });
        if text.len() <= SLACK_MARKDOWN_BLOCK_MAX_CHARS {
            body["blocks"] = serde_json::json!([{
                "type": "markdown",
                "text": text
            }]);
        }
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let resp_body: serde_json::Value = resp.json().await?;
        if resp_body.get("ok") != Some(&serde_json::Value::Bool(true)) {
            let err = resp_body
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack chat.postMessage (lazy draft) failed: {err}");
        }

        let ts = resp_body
            .get("ts")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        if let Some(ref real_ts) = ts {
            self.lazy_draft_ts
                .lock()
                .await
                .insert(lazy_id.to_string(), real_ts.clone());
        }

        Ok(ts)
    }

    /// Set the Assistants API status bar text for a channel's active thread.
    pub(crate) async fn set_assistant_status(&self, channel_id: &str, status: &str) {
        let thread_ts = {
            let map = match self.active_assistant_thread.lock() {
                Ok(m) => m,
                Err(_) => return,
            };
            match map.get(channel_id) {
                Some(ts) => ts.clone(),
                None => return,
            }
        };

        let body = serde_json::json!({
            "channel_id": channel_id,
            "thread_ts": thread_ts,
            "status": status,
        });

        let _ = self
            .http_client()
            .post("https://slack.com/api/assistant.threads.setStatus")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await;
    }

    pub(crate) fn http_client(&self) -> reqwest::Client {
        operant_config::schema::build_channel_proxy_client_with_timeouts(
            "channel.slack",
            self.proxy_url.as_deref(),
            30,
            10,
        )
    }

    /// Post a new Slack message and return the message timestamp (`ts`).
    ///
    /// This is a lower-level helper that exposes the `ts` value needed for
    /// subsequent `chat.update` calls. For simple sends, use the [`Channel::send`]
    /// trait method instead.
    pub async fn post_message(&self, channel: &str, text: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let raw = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&raw);
            anyhow::bail!("Slack chat.postMessage failed ({status}): {sanitized}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack chat.postMessage failed: {err}");
        }

        parsed
            .get("ts")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Slack chat.postMessage response missing 'ts'"))
    }

    /// Update an existing Slack message in-place using `chat.update`.
    ///
    /// `channel` is the channel ID and `ts` is the timestamp of the original
    /// message (returned by `post_message`).
    pub async fn update_message(&self, channel: &str, ts: &str, text: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
        });

        let resp = self
            .http_client()
            .post("https://slack.com/api/chat.update")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let raw = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&raw);
            anyhow::bail!("Slack chat.update failed ({status}): {sanitized}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack chat.update failed: {err}");
        }

        Ok(())
    }

    /// Check if a Slack user ID is in the allowlist.
    /// Empty list means deny everyone until explicitly configured.
    /// `"*"` means allow everyone.
    pub(crate) fn is_user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.iter().any(|u| u == "*" || u == user_id)
    }

    pub(crate) fn is_group_sender_trigger_enabled(&self, user_id: &str) -> bool {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return false;
        }

        self.group_reply_allowed_sender_ids
            .iter()
            .any(|entry| entry == "*" || entry == user_id)
    }

    pub(crate) fn outbound_thread_ts<'a>(&self, message: &'a SendMessage) -> Option<&'a str> {
        if self.thread_replies {
            message.thread_ts.as_deref()
        } else {
            None
        }
    }

    /// Get the bot's own user ID so we can ignore our own messages
    pub(crate) async fn get_bot_user_id(&self) -> Option<String> {
        let resp: serde_json::Value = self
            .http_client()
            .get("https://slack.com/api/auth.test")
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        resp.get("user_id")
            .and_then(|u| u.as_str())
            .map(String::from)
    }

    /// Resolve the thread identifier for inbound Slack messages.
    /// Replies carry `thread_ts` (root thread id); top-level messages only have `ts`.
    pub(crate) fn inbound_thread_ts(msg: &serde_json::Value, ts: &str) -> Option<String> {
        msg.get("thread_ts")
            .and_then(|t| t.as_str())
            .or(if ts.is_empty() { None } else { Some(ts) })
            .map(str::to_string)
    }

    /// Like `inbound_thread_ts`, but only returns a value when Slack's own
    /// `thread_ts` field is present (genuine thread reply). Does **not** fall
    /// back to the message's `ts`, so top-level messages get `None`. Used when
    /// `thread_replies=false` so that all top-level messages from the same user
    /// share a single conversation session key.
    pub(crate) fn inbound_thread_ts_genuine_only(msg: &serde_json::Value) -> Option<String> {
        msg.get("thread_ts")
            .and_then(|t| t.as_str())
            .map(str::to_string)
    }

    /// Returns the interruption scope identifier for a Slack message.
    ///
    /// Returns `Some(thread_ts)` only when the message is a genuine thread reply
    /// (Slack's `thread_ts` field is present and differs from the message's own `ts`).
    /// Returns `None` for top-level messages and thread parent messages (where
    /// `thread_ts == ts`), placing them in the 3-component scope key
    /// (`channel_reply_target_sender`).
    ///
    /// Intentional: top-level messages and threaded replies are separate conversational
    /// scopes and should not cancel each other's in-flight tasks.
    pub(crate) fn inbound_interruption_scope_id(
        msg: &serde_json::Value,
        ts: &str,
    ) -> Option<String> {
        msg.get("thread_ts")
            .and_then(|t| t.as_str())
            .filter(|&t| t != ts)
            .map(str::to_string)
    }

    pub(crate) fn normalized_channel_id(input: Option<&str>) -> Option<String> {
        input
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "*")
            .map(ToOwned::to_owned)
    }

    /// Resolve the effective channel scope from `channel_ids`.
    /// Returns `None` when empty (wildcard discovery).
    pub(crate) fn scoped_channel_ids(&self) -> Option<Vec<String>> {
        let mut seen = HashSet::new();
        let ids: Vec<String> = self
            .channel_ids
            .iter()
            .filter_map(|entry| Self::normalized_channel_id(Some(entry)))
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if ids.is_empty() { None } else { Some(ids) }
    }

    pub(crate) fn configured_app_token(&self) -> Option<String> {
        self.app_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(crate) fn normalize_group_reply_allowed_sender_ids(sender_ids: Vec<String>) -> Vec<String> {
        let mut normalized = sender_ids
            .into_iter()
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        normalized
    }

    pub(crate) fn user_cache_ttl() -> Duration {
        Duration::from_secs(SLACK_USER_CACHE_TTL_SECS)
    }

    pub(crate) fn sanitize_display_name(name: &str) -> Option<String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub(crate) fn extract_user_display_name(payload: &serde_json::Value) -> Option<String> {
        let user = payload.get("user")?;
        let profile = user.get("profile");

        let candidates = [
            profile
                .and_then(|p| p.get("display_name"))
                .and_then(|v| v.as_str()),
            profile
                .and_then(|p| p.get("display_name_normalized"))
                .and_then(|v| v.as_str()),
            profile
                .and_then(|p| p.get("real_name_normalized"))
                .and_then(|v| v.as_str()),
            profile
                .and_then(|p| p.get("real_name"))
                .and_then(|v| v.as_str()),
            user.get("real_name").and_then(|v| v.as_str()),
            user.get("name").and_then(|v| v.as_str()),
        ];

        for candidate in candidates.into_iter().flatten() {
            if let Some(display_name) = Self::sanitize_display_name(candidate) {
                return Some(display_name);
            }
        }

        None
    }

    pub(crate) fn cached_sender_display_name(&self, user_id: &str) -> Option<String> {
        let now = Instant::now();
        let Ok(mut cache) = self.user_display_name_cache.lock() else {
            return None;
        };

        if let Some(entry) = cache.get(user_id)
            && now <= entry.expires_at
        {
            return Some(entry.display_name.clone());
        }

        cache.remove(user_id);
        None
    }

    pub(crate) fn cache_sender_display_name(&self, user_id: &str, display_name: &str) {
        let Ok(mut cache) = self.user_display_name_cache.lock() else {
            return;
        };
        if cache.len() >= SLACK_USER_CACHE_MAX_ENTRIES {
            let now = Instant::now();
            cache.retain(|_, v| v.expires_at > now);
        }
        cache.insert(
            user_id.to_string(),
            CachedSlackDisplayName {
                display_name: display_name.to_string(),
                expires_at: Instant::now() + Self::user_cache_ttl(),
            },
        );
    }

    pub(crate) async fn fetch_sender_display_name(&self, user_id: &str) -> Option<String> {
        let resp = match self
            .http_client()
            .get("https://slack.com/api/users.info")
            .bearer_auth(&self.bot_token)
            .query(&[("user", user_id)])
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!("Slack users.info request failed for {user_id}: {err}");
                return None;
            }
        };

        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&body);
            tracing::warn!("Slack users.info failed for {user_id} ({status}): {sanitized}");
            return None;
        }

        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if payload.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = payload
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            tracing::warn!("Slack users.info returned error for {user_id}: {err}");
            return None;
        }

        Self::extract_user_display_name(&payload)
    }

    pub(crate) async fn resolve_sender_identity(&self, user_id: &str) -> String {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return String::new();
        }

        if let Some(display_name) = self.cached_sender_display_name(user_id) {
            return display_name;
        }

        if let Some(display_name) = self.fetch_sender_display_name(user_id).await {
            self.cache_sender_display_name(user_id, &display_name);
            return display_name;
        }

        user_id.to_string()
    }

    pub(crate) fn is_group_channel_id(channel_id: &str) -> bool {
        matches!(channel_id.chars().next(), Some('C' | 'G'))
    }

    pub(crate) fn contains_bot_mention(text: &str, bot_user_id: &str) -> bool {
        if bot_user_id.is_empty() {
            return false;
        }
        text.contains(&format!("<@{bot_user_id}>"))
    }

    pub(crate) fn strip_bot_mentions(text: &str, bot_user_id: &str) -> String {
        if bot_user_id.is_empty() {
            return text.trim().to_string();
        }
        text.replace(&format!("<@{bot_user_id}>"), " ")
            .trim()
            .to_string()
    }

    pub(crate) fn normalize_incoming_text(
        text: &str,
        require_mention: bool,
        bot_user_id: &str,
    ) -> Option<String> {
        if require_mention && !Self::contains_bot_mention(text, bot_user_id) {
            return None;
        }

        // Always strip bot mentions so the model sees clean text,
        // even in threads where the mention wasn't required.
        Some(Self::strip_bot_mentions(text, bot_user_id))
    }

    #[cfg(test)]
    pub(crate) fn normalize_incoming_content(
        text: &str,
        require_mention: bool,
        bot_user_id: &str,
    ) -> Option<String> {
        let normalized = Self::normalize_incoming_text(text, require_mention, bot_user_id)?;
        if normalized.is_empty() {
            return None;
        }
        Some(normalized)
    }

    pub(crate) fn is_supported_message_subtype(subtype: Option<&str>) -> bool {
        matches!(subtype, None | Some("file_share" | "thread_broadcast"))
    }

    pub(crate) fn compose_incoming_content(
        text: String,
        attachment_blocks: Vec<String>,
    ) -> Option<String> {
        let mut sections = Vec::new();
        if !text.trim().is_empty() {
            sections.push(text.trim().to_string());
        }
        for block in attachment_blocks {
            if !block.trim().is_empty() {
                sections.push(block);
            }
        }

        if sections.is_empty() {
            None
        } else {
            Some(sections.join("\n\n"))
        }
    }

    pub(crate) async fn build_incoming_content(
        &self,
        message: &serde_json::Value,
        require_mention: bool,
        bot_user_id: &str,
    ) -> Option<String> {
        let text = message
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let normalized_text = Self::normalize_incoming_text(text, require_mention, bot_user_id)?;
        let attachment_blocks = self.render_file_attachments(message).await;
        let permalink_blocks = self.resolve_permalink_blocks(&normalized_text).await;
        let mut blocks = attachment_blocks;
        blocks.extend(permalink_blocks);
        Self::compose_incoming_content(normalized_text, blocks)
    }

    pub(crate) async fn resolve_permalink_blocks(&self, text: &str) -> Vec<String> {
        let permalinks = Self::extract_slack_permalinks(text);
        if permalinks.is_empty() {
            return Vec::new();
        }
        let tasks = permalinks
            .into_iter()
            .map(|permalink| async move { self.resolve_slack_permalink(&permalink).await });

        futures_util::stream::iter(tasks)
            .buffer_unordered(SLACK_ATTACHMENT_RENDER_CONCURRENCY)
            .filter_map(|block| async move { block })
            .collect()
            .await
    }

    pub(crate) fn extract_slack_permalinks(text: &str) -> Vec<SlackPermalinkRef> {
        let mut permalinks = Vec::new();
        let mut seen = HashSet::new();

        for token in text.split_whitespace() {
            if permalinks.len() >= SLACK_PERMALINK_MAX_LINKS_PER_MESSAGE {
                break;
            }

            let Some(url) = Self::extract_url_token(token) else {
                continue;
            };
            let Some(permalink) = Self::parse_slack_permalink(&url) else {
                continue;
            };
            if seen.insert((permalink.channel_id.clone(), permalink.message_ts.clone())) {
                permalinks.push(permalink);
            }
        }

        permalinks
    }

    pub(crate) fn extract_url_token(token: &str) -> Option<String> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }

        let candidate = if trimmed.starts_with('<') && trimmed.ends_with('>') {
            trimmed
                .trim_start_matches('<')
                .trim_end_matches('>')
                .split('|')
                .next()
                .unwrap_or_default()
                .trim()
        } else {
            trimmed.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | ',' | ';'
                )
            })
        };

        if candidate.starts_with("https://") || candidate.starts_with("http://") {
            Some(candidate.to_string())
        } else {
            None
        }
    }

    pub(crate) fn parse_slack_permalink(raw_url: &str) -> Option<SlackPermalinkRef> {
        let url = reqwest::Url::parse(raw_url).ok()?;
        let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        if host != "slack.com" && !host.ends_with(".slack.com") {
            return None;
        }

        let mut segments = url.path_segments()?;
        let first = segments.next()?;
        let second = segments.next()?;
        let third = segments.next()?;
        if first != "archives" || segments.next().is_some() {
            return None;
        }

        let channel_id = second.trim();
        if channel_id.is_empty() {
            return None;
        }

        let message_ts = Self::parse_slack_permalink_ts(third)?;
        let thread_ts_hint = url
            .query_pairs()
            .find(|(key, _)| key == "thread_ts")
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| Self::is_valid_slack_ts(value));

        Some(SlackPermalinkRef {
            url: raw_url.to_string(),
            channel_id: channel_id.to_string(),
            message_ts,
            thread_ts_hint,
        })
    }

    pub(crate) fn parse_slack_permalink_ts(segment: &str) -> Option<String> {
        let digits = segment.strip_prefix('p')?.trim();
        if digits.len() <= 6 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }

        let (secs, micros) = digits.split_at(digits.len() - 6);
        Some(format!("{secs}.{micros}"))
    }

    pub(crate) fn is_valid_slack_ts(ts: &str) -> bool {
        let Some((secs, micros)) = ts.split_once('.') else {
            return false;
        };
        !secs.is_empty()
            && micros.len() == 6
            && secs.chars().all(|ch| ch.is_ascii_digit())
            && micros.chars().all(|ch| ch.is_ascii_digit())
    }

    pub(crate) async fn resolve_slack_permalink(
        &self,
        permalink: &SlackPermalinkRef,
    ) -> Option<String> {
        let message_lookup = self
            .fetch_permalink_message(&permalink.channel_id, &permalink.message_ts)
            .await;
        let message = match message_lookup {
            SlackPermalinkLookup::Message(message) => message,
            SlackPermalinkLookup::AccessDenied(reason) => {
                return Some(Self::format_permalink_access_denied(permalink, &reason));
            }
            SlackPermalinkLookup::NotFound => {
                let thread_ts = permalink.thread_ts_hint.as_deref()?;
                let replies = self
                    .fetch_thread_messages_with_retry(&permalink.channel_id, thread_ts)
                    .await?;
                let target = replies.into_iter().find(|reply| {
                    reply.get("ts").and_then(|value| value.as_str())
                        == Some(permalink.message_ts.as_str())
                });
                let target = target?;
                return self
                    .format_permalink_context(permalink, target, Some(thread_ts))
                    .await;
            }
        };

        let thread_ts = message
            .get("thread_ts")
            .and_then(|value| value.as_str())
            .filter(|thread_ts| Self::is_valid_slack_ts(thread_ts))
            .map(str::to_string);

        self.format_permalink_context(permalink, message, thread_ts.as_deref())
            .await
    }

    pub(crate) async fn fetch_permalink_message(
        &self,
        channel_id: &str,
        message_ts: &str,
    ) -> SlackPermalinkLookup {
        let resp = match self
            .http_client()
            .get("https://slack.com/api/conversations.history")
            .bearer_auth(&self.bot_token)
            .query(&[
                ("channel", channel_id),
                ("oldest", message_ts),
                ("latest", message_ts),
                ("inclusive", "true"),
                ("limit", "1"),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    "Slack permalink resolver: conversations.history request failed for channel={} ts={}: {}",
                    channel_id,
                    message_ts,
                    err
                );
                return SlackPermalinkLookup::NotFound;
            }
        };

        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));
        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&body);
            tracing::warn!(
                "Slack permalink resolver: conversations.history failed for channel={} ts={} ({}): {}",
                channel_id,
                message_ts,
                status,
                sanitized
            );
            return SlackPermalinkLookup::NotFound;
        }

        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if payload.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = payload
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return match err {
                "not_in_channel" => SlackPermalinkLookup::AccessDenied(
                    "The Slack bot is not in that channel. Invite the app to the channel and try again."
                        .to_string(),
                ),
                "missing_scope" => SlackPermalinkLookup::AccessDenied(
                    "The Slack app is missing the scope needed to read that channel."
                        .to_string(),
                ),
                _ => {
                    tracing::warn!(
                        "Slack permalink resolver: conversations.history returned error for channel={} ts={}: {}",
                        channel_id, message_ts, err
                    );
                    SlackPermalinkLookup::NotFound
                }
            };
        }

        let messages = payload
            .get("messages")
            .and_then(|messages| messages.as_array())
            .cloned()
            .unwrap_or_default();
        messages
            .first()
            .cloned()
            .map(SlackPermalinkLookup::Message)
            .unwrap_or(SlackPermalinkLookup::NotFound)
    }

    pub(crate) fn format_permalink_access_denied(
        permalink: &SlackPermalinkRef,
        reason: &str,
    ) -> String {
        format!(
            "[Slack Link Access]\nURL: {}\nStatus: {}",
            permalink.url, reason
        )
    }

    pub(crate) async fn fetch_thread_messages_with_retry(
        &self,
        channel_id: &str,
        thread_ts: &str,
    ) -> Option<Vec<serde_json::Value>> {
        let payload = self
            .fetch_thread_replies_with_retry(channel_id, thread_ts, "0")
            .await?;
        let messages = payload
            .get("messages")
            .and_then(|messages| messages.as_array())
            .cloned()
            .unwrap_or_default();
        Some(messages)
    }

    pub(crate) async fn format_permalink_context(
        &self,
        permalink: &SlackPermalinkRef,
        message: serde_json::Value,
        thread_ts: Option<&str>,
    ) -> Option<String> {
        let mut lines = vec![
            "[Slack Link Context]".to_string(),
            format!("URL: {}", permalink.url),
        ];

        if let Some(thread_ts) = thread_ts {
            let replies = self
                .fetch_thread_messages_with_retry(&permalink.channel_id, thread_ts)
                .await
                .unwrap_or_else(|| vec![message.clone()]);
            let rendered = self
                .render_permalink_thread_messages(&replies, &permalink.message_ts)
                .await;
            if rendered.is_empty() {
                return None;
            }
            lines.push("Thread:".to_string());
            lines.extend(rendered);
        } else {
            let rendered = self.render_permalink_message_line(&message, true).await?;
            lines.push("Message:".to_string());
            lines.push(rendered);
        }

        Self::truncate_text(&lines.join("\n"), SLACK_PERMALINK_TEXT_MAX_CHARS)
    }

    pub(crate) async fn render_permalink_thread_messages(
        &self,
        messages: &[serde_json::Value],
        target_ts: &str,
    ) -> Vec<String> {
        let mut rendered = Vec::new();
        let total = messages.len();
        let start = total.saturating_sub(SLACK_PERMALINK_THREAD_MAX_REPLIES);

        if start > 0 {
            rendered.push(format!("… {} earlier thread messages omitted …", start));
        }

        for message in &messages[start..] {
            if let Some(line) = self
                .render_permalink_message_line(
                    message,
                    message.get("ts").and_then(|value| value.as_str()) == Some(target_ts),
                )
                .await
            {
                rendered.push(line);
            }
        }

        rendered
    }

    pub(crate) async fn render_permalink_message_line(
        &self,
        message: &serde_json::Value,
        highlight: bool,
    ) -> Option<String> {
        let user_id = message
            .get("user")
            .or_else(|| message.get("bot_id"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let sender = if user_id.is_empty() {
            "unknown".to_string()
        } else {
            self.resolve_sender_identity(user_id).await
        };

        let text = message
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("[no text]");
        let attachment_blocks = self.render_file_attachments(message).await;
        let content = Self::compose_incoming_content(text.to_string(), attachment_blocks)
            .unwrap_or_else(|| text.to_string())
            .replace('\n', " ");
        let prefix = if highlight { ">" } else { "-" };
        Some(format!("{prefix} {sender}: {content}"))
    }

    pub(crate) async fn render_file_attachments(&self, message: &serde_json::Value) -> Vec<String> {
        let Some(files) = message.get("files").and_then(|value| value.as_array()) else {
            return Vec::new();
        };

        if files.len() > SLACK_ATTACHMENT_MAX_FILES_PER_MESSAGE {
            tracing::warn!(
                "Slack message has {} files; processing first {} only",
                files.len(),
                SLACK_ATTACHMENT_MAX_FILES_PER_MESSAGE
            );
        }

        let limited_files = files
            .iter()
            .take(SLACK_ATTACHMENT_MAX_FILES_PER_MESSAGE)
            .cloned()
            .collect::<Vec<_>>();

        let tasks =
            limited_files
                .into_iter()
                .enumerate()
                .map(|(idx, raw_file)| async move {
                    (idx, self.render_file_attachment(&raw_file).await)
                });

        let mut rendered = futures_util::stream::iter(tasks)
            .buffer_unordered(SLACK_ATTACHMENT_RENDER_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        rendered.sort_by_key(|(idx, _)| *idx);
        rendered
            .into_iter()
            .filter_map(|(_, block)| block)
            .collect()
    }

    pub(crate) async fn render_file_attachment(
        &self,
        raw_file: &serde_json::Value,
    ) -> Option<String> {
        let file = self
            .hydrate_file_object(raw_file)
            .await
            .unwrap_or_else(|| raw_file.clone());

        // Voice / audio transcription: if transcription is configured and the
        // file looks like an audio attachment, download and transcribe it.
        if Self::is_audio_file(&file)
            && let Some(transcribed) = self.try_transcribe_audio_file(&file).await
        {
            return Some(transcribed);
        }
        if Self::is_image_file(&file)
            && let Some(marker) = self.fetch_image_marker(&file).await
        {
            return Some(marker);
        }

        let mut snippet = Self::file_text_preview(&file);
        if snippet.is_none() && Self::is_probably_text_file(&file) {
            snippet = self.download_text_snippet(&file).await;
        }

        if let Some(text) = snippet
            && !text.trim().is_empty()
        {
            return Some(Self::format_snippet_attachment(&file, &text));
        }

        Some(Self::format_attachment_summary(&file))
    }

    pub(crate) async fn hydrate_file_object(
        &self,
        file: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let file_id = Self::slack_file_id(file)?;
        let file_access = file
            .get("file_access")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let mode = Self::slack_file_mode(file).unwrap_or_default();

        let requires_lookup = file_access.eq_ignore_ascii_case("check_file_info")
            || Self::slack_file_download_url(file).is_none()
            || (Self::is_probably_text_file(file) && Self::file_text_preview(file).is_none())
            || (mode == "snippet" && file.get("preview").is_none());
        if !requires_lookup {
            return Some(file.clone());
        }

        self.fetch_file_info(file_id)
            .await
            .or_else(|| Some(file.clone()))
    }

    pub(crate) async fn fetch_file_info(&self, file_id: &str) -> Option<serde_json::Value> {
        let resp = match self
            .http_client()
            .get("https://slack.com/api/files.info")
            .bearer_auth(&self.bot_token)
            .query(&[("file", file_id)])
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!("Slack files.info request failed for {file_id}: {err}");
                return None;
            }
        };

        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));
        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&body);
            tracing::warn!("Slack files.info failed for {file_id} ({status}): {sanitized}");
            return None;
        }

        let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if payload.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = payload
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            tracing::warn!("Slack files.info returned error for {file_id}: {err}");
            return None;
        }

        payload.get("file").cloned()
    }

    pub(crate) fn slack_file_id(file: &serde_json::Value) -> Option<&str> {
        file.get("id").and_then(|value| value.as_str())
    }

    pub(crate) fn slack_file_name(file: &serde_json::Value) -> String {
        file.get("title")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| file.get("name").and_then(|value| value.as_str()))
            .unwrap_or("attachment")
            .trim()
            .to_string()
    }

    pub(crate) fn slack_file_mode(file: &serde_json::Value) -> Option<String> {
        file.get("mode")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
    }

    pub(crate) fn slack_file_mime(file: &serde_json::Value) -> Option<String> {
        file.get("mimetype")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
    }

    pub(crate) fn slack_file_download_url(file: &serde_json::Value) -> Option<&str> {
        file.get("url_private_download")
            .and_then(|value| value.as_str())
            .or_else(|| file.get("url_private").and_then(|value| value.as_str()))
    }

    pub(crate) fn slack_image_candidate_urls(file: &serde_json::Value) -> Vec<String> {
        let mut urls = Vec::new();
        let mut seen = HashSet::new();
        for key in [
            "thumb_1024",
            "thumb_960",
            "thumb_800",
            "thumb_720",
            "thumb_480",
            "thumb_360",
            "thumb_160",
            "url_private_download",
            "url_private",
        ] {
            if let Some(url) = file.get(key).and_then(|value| value.as_str()) {
                let trimmed = url.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if seen.insert(trimmed.to_string()) {
                    urls.push(trimmed.to_string());
                }
            }
        }
        urls
    }

    pub(crate) fn is_allowed_slack_media_hostname(host: &str) -> bool {
        let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        SLACK_ALLOWED_MEDIA_HOST_SUFFIXES
            .iter()
            .any(|suffix| normalized == *suffix || normalized.ends_with(&format!(".{suffix}")))
    }

    pub(crate) fn redact_slack_url(url: &reqwest::Url) -> String {
        let host = url.host_str().unwrap_or("unknown-host");
        let tail = url
            .path_segments()
            .and_then(|mut segments| {
                segments
                    .rfind(|segment| !segment.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "root".to_string());
        format!("{host}/.../{tail}")
    }

    pub(crate) fn redact_raw_slack_url(raw_url: &str) -> String {
        reqwest::Url::parse(raw_url)
            .map(|parsed| Self::redact_slack_url(&parsed))
            .unwrap_or_else(|_| "<invalid-url>".to_string())
    }

    pub(crate) fn redact_redirect_location(location: &str) -> String {
        match reqwest::Url::parse(location) {
            Ok(url) => Self::redact_slack_url(&url),
            Err(_) => {
                let tail = location
                    .split(['?', '#'])
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .filter(|segment| !segment.is_empty())
                    .unwrap_or("relative");
                format!("relative/.../{tail}")
            }
        }
    }

    pub(crate) fn validate_slack_private_file_url(raw_url: &str) -> Option<reqwest::Url> {
        let parsed = match reqwest::Url::parse(raw_url) {
            Ok(url) => url,
            Err(err) => {
                let redacted_raw = Self::redact_raw_slack_url(raw_url);
                tracing::warn!("Slack file URL parse failed for {redacted_raw}: {err}");
                return None;
            }
        };
        let redacted = Self::redact_slack_url(&parsed);

        if parsed.scheme() != "https" {
            tracing::warn!(
                "Slack file URL rejected due to non-HTTPS scheme for {}: {}",
                redacted,
                parsed.scheme()
            );
            return None;
        }

        let Some(host) = parsed.host_str() else {
            tracing::warn!("Slack file URL rejected due to missing host: {redacted}");
            return None;
        };
        if !Self::is_allowed_slack_media_hostname(host) {
            tracing::warn!("Slack file URL rejected due to non-Slack host: {redacted}");
            return None;
        }

        Some(parsed)
    }

    pub(crate) fn resolve_https_redirect_target(
        base: &reqwest::Url,
        location: &str,
    ) -> Option<reqwest::Url> {
        let redacted_base = Self::redact_slack_url(base);
        let redacted_location = Self::redact_redirect_location(location);
        let target = match base.join(location) {
            Ok(url) => url,
            Err(err) => {
                tracing::warn!(
                    "Slack file redirect URL parse failed for base {} and location {}: {}",
                    redacted_base,
                    redacted_location,
                    err
                );
                return None;
            }
        };
        let redacted_target = Self::redact_slack_url(&target);
        if target.scheme() != "https" {
            tracing::warn!(
                "Slack file redirect rejected due to non-HTTPS scheme for {}",
                redacted_target
            );
            return None;
        }
        let Some(host) = target.host_str() else {
            tracing::warn!(
                "Slack file redirect rejected due to missing host for {}",
                redacted_target
            );
            return None;
        };
        if !Self::is_allowed_slack_media_hostname(host) {
            tracing::warn!(
                "Slack file redirect rejected due to non-Slack host for {}",
                redacted_target
            );
            return None;
        }
        Some(target)
    }

    pub(crate) fn slack_media_http_client_no_redirect(&self) -> anyhow::Result<reqwest::Client> {
        let builder = operant_config::schema::apply_channel_proxy_to_builder(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10)),
            "channel.slack",
            self.proxy_url.as_deref(),
        );
        builder
            .build()
            .context("failed to build Slack media no-redirect HTTP client")
    }

    pub(crate) async fn fetch_slack_private_file(
        &self,
        raw_url: &str,
    ) -> Option<reqwest::Response> {
        let parsed = Self::validate_slack_private_file_url(raw_url)?;
        let redacted_parsed = Self::redact_slack_url(&parsed);
        let client = match self.slack_media_http_client_no_redirect() {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!("Slack file fetch failed for {}: {}", redacted_parsed, err);
                return None;
            }
        };
        let mut current_url = parsed;

        for redirect_hop in 0..=SLACK_MEDIA_REDIRECT_MAX_HOPS {
            let redacted_current = Self::redact_slack_url(&current_url);
            let mut req = client.get(current_url.clone());
            if redirect_hop == 0 {
                req = req.bearer_auth(&self.bot_token);
            }
            let response = match req.send().await {
                Ok(response) => response,
                Err(err) => {
                    tracing::warn!("Slack file fetch failed for {}: {}", redacted_current, err);
                    return None;
                }
            };

            if !response.status().is_redirection() {
                return Some(response);
            }

            if redirect_hop == SLACK_MEDIA_REDIRECT_MAX_HOPS {
                tracing::warn!(
                    "Slack file redirect limit exceeded for {} after {} hops",
                    redacted_current,
                    SLACK_MEDIA_REDIRECT_MAX_HOPS
                );
                return Some(response);
            }

            let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                return Some(response);
            };
            let Ok(location) = location.to_str() else {
                tracing::warn!(
                    "Slack file redirect location header is not valid UTF-8 for {}",
                    redacted_current
                );
                return Some(response);
            };
            let Some(next_url) = Self::resolve_https_redirect_target(&current_url, location) else {
                return Some(response);
            };
            current_url = next_url;
        }

        None
    }

    pub(crate) async fn fetch_image_marker(&self, file: &serde_json::Value) -> Option<String> {
        let file_name = Self::slack_file_name(file);
        let image_urls = Self::slack_image_candidate_urls(file);
        if image_urls.is_empty() {
            tracing::warn!(
                "Slack file attachment is image-like but has no downloadable URL: {}",
                file_name
            );
            return None;
        }

        for url in image_urls {
            if let Some(marker) = self.download_private_image_as_marker(&url, file).await {
                return Some(marker);
            }
        }

        tracing::warn!("Slack image attachment download failed for {file_name}");
        None
    }

    pub(crate) async fn download_private_image_as_marker(
        &self,
        url: &str,
        file: &serde_json::Value,
    ) -> Option<String> {
        let redacted_url = Self::redact_raw_slack_url(url);
        let resp = self.fetch_slack_private_file(url).await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));
            let sanitized = operant_providers::sanitize_api_error(&body);
            tracing::warn!(
                "Slack image fetch failed for {} ({status}): {sanitized}",
                redacted_url
            );
            return None;
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if let Some(content_length) = resp.content_length() {
            let content_length = usize::try_from(content_length).unwrap_or(usize::MAX);
            if content_length > SLACK_ATTACHMENT_IMAGE_MAX_BYTES {
                tracing::warn!(
                    "Slack image fetch skipped for {}: content-length {} exceeds {} bytes",
                    redacted_url,
                    content_length,
                    SLACK_ATTACHMENT_IMAGE_MAX_BYTES
                );
                return None;
            }
        }

        let bytes = match resp.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!("Slack image body read failed for {}: {err}", redacted_url);
                return None;
            }
        };
        if bytes.is_empty() {
            tracing::warn!("Slack image body is empty for {}", redacted_url);
            return None;
        }
        if bytes.len() > SLACK_ATTACHMENT_IMAGE_MAX_BYTES {
            tracing::warn!(
                "Slack image body too large for {}: {} bytes exceeds {} bytes",
                redacted_url,
                bytes.len(),
                SLACK_ATTACHMENT_IMAGE_MAX_BYTES
            );
            return None;
        }

        let Some(mime) =
            Self::detect_image_mime(content_type.as_deref(), file, bytes.as_ref(), url)
        else {
            tracing::warn!("Slack image MIME detection failed for {}", redacted_url);
            return None;
        };
        if !Self::is_supported_image_mime(&mime) {
            tracing::warn!(
                "Slack image MIME not supported for {}: {mime}",
                redacted_url
            );
            return None;
        }

        let file_name = Self::slack_file_name(file);
        if let Some(saved_path) = self
            .persist_image_attachment(file, &file_name, &mime, bytes.as_ref())
            .await
        {
            return Some(format!("[IMAGE:{}]", saved_path.display()));
        }

        if bytes.len() > SLACK_ATTACHMENT_IMAGE_INLINE_FALLBACK_MAX_BYTES {
            tracing::warn!(
                "Slack image inline fallback skipped for {}: {} bytes exceeds {} bytes",
                redacted_url,
                bytes.len(),
                SLACK_ATTACHMENT_IMAGE_INLINE_FALLBACK_MAX_BYTES
            );
            return None;
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("[IMAGE:data:{mime};base64,{encoded}]"))
    }

    pub(crate) fn detect_image_mime(
        content_type_header: Option<&str>,
        file: &serde_json::Value,
        bytes: &[u8],
        source_url: &str,
    ) -> Option<String> {
        let redacted_source = Self::redact_raw_slack_url(source_url);
        if let Some(magic_mime) = Self::mime_from_magic(bytes) {
            return Some(magic_mime.to_string());
        }

        if let Some(header_mime) = content_type_header
            .and_then(Self::normalized_content_type)
            .filter(|mime| mime.starts_with("image/"))
        {
            tracing::warn!(
                "Slack image MIME mismatch for {}: HTTP header claims {}, but bytes do not match a supported image signature",
                redacted_source,
                header_mime
            );
        }

        if let Some(file_mime) =
            Self::slack_file_mime(file).filter(|mime| mime.starts_with("image/"))
        {
            tracing::warn!(
                "Slack image MIME mismatch for {}: file metadata claims {}, but bytes do not match a supported image signature",
                redacted_source,
                file_mime
            );
        }

        if let Some(ext) = Self::file_extension(source_url)
            .or_else(|| Self::file_extension(&Self::slack_file_name(file)))
            && let Some(mime) = Self::mime_from_extension(&ext)
        {
            tracing::warn!(
                "Slack image MIME mismatch for {}: filename extension implies {}, but bytes do not match a supported image signature",
                redacted_source,
                mime
            );
        }

        None
    }

    pub(crate) fn normalized_content_type(content_type: &str) -> Option<String> {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if mime.is_empty() { None } else { Some(mime) }
    }

    pub(crate) fn is_supported_image_mime(mime: &str) -> bool {
        SLACK_SUPPORTED_IMAGE_MIME_TYPES.contains(&mime)
    }

    pub(crate) fn mime_from_extension(ext: &str) -> Option<&'static str> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "bmp" => Some("image/bmp"),
            _ => None,
        }
    }

    pub(crate) fn mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
        if bytes.len() >= 8
            && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'])
        {
            return Some("image/png");
        }
        if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return Some("image/jpeg");
        }
        if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
            return Some("image/gif");
        }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some("image/webp");
        }
        if bytes.len() >= 2 && bytes.starts_with(b"BM") {
            return Some("image/bmp");
        }
        None
    }

    pub(crate) async fn persist_image_attachment(
        &self,
        file: &serde_json::Value,
        file_name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Option<PathBuf> {
        let workspace = self.workspace_dir.as_ref()?;
        let safe_name = Self::sanitize_attachment_filename(file_name)
            .unwrap_or_else(|| "attachment".to_string());
        let ext = Self::image_extension_for_mime(mime).unwrap_or("png");
        let safe_name = Self::ensure_file_extension(&safe_name, ext);
        let file_id = Self::slack_file_id(file)
            .map(Self::sanitize_file_id)
            .unwrap_or_else(|| "file".to_string());
        let generated_name = format!(
            "slack_{}_{}_{}",
            Utc::now().timestamp_millis(),
            file_id,
            safe_name
        );

        let output_path = match Self::resolve_workspace_attachment_output_path(
            workspace,
            &generated_name,
        )
        .await
        {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(
                    "Slack image attachment path resolution failed for {}: {err}",
                    file_name
                );
                return None;
            }
        };

        let Some(parent_dir) = output_path.parent() else {
            tracing::warn!(
                "Slack image attachment write failed for {}: missing parent directory",
                output_path.display()
            );
            return None;
        };

        let file_tail = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        let temp_name = format!(
            ".{file_tail}.{}.part",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let temp_path = parent_dir.join(temp_name);

        let mut temp_file = match tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
        {
            Ok(file) => file,
            Err(err) => {
                tracing::warn!(
                    "Slack image attachment temp open failed for {}: {err}",
                    temp_path.display()
                );
                return None;
            }
        };

        if let Err(err) = temp_file.write_all(bytes).await {
            tracing::warn!(
                "Slack image attachment temp write failed for {}: {err}",
                temp_path.display()
            );
            let _ = tokio::fs::remove_file(&temp_path).await;
            return None;
        }
        if let Err(err) = temp_file.sync_all().await {
            tracing::warn!(
                "Slack image attachment temp sync failed for {}: {err}",
                temp_path.display()
            );
            let _ = tokio::fs::remove_file(&temp_path).await;
            return None;
        }
        drop(temp_file);

        // Reject symlinks at the destination to prevent a symlink-following attack
        // where an attacker places a symlink at the target path to redirect writes
        // outside the workspace.
        match tokio::fs::symlink_metadata(&output_path).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                tracing::warn!(
                    "Slack image attachment refused: output path is a symlink: {}",
                    output_path.display()
                );
                let _ = tokio::fs::remove_file(&temp_path).await;
                return None;
            }
            _ => {}
        }

        if let Err(err) = tokio::fs::rename(&temp_path, &output_path).await {
            tracing::warn!(
                "Slack image attachment finalize failed for {}: {err}",
                output_path.display()
            );
            let _ = tokio::fs::remove_file(&temp_path).await;
            return None;
        }

        Some(output_path)
    }

    pub(crate) async fn resolve_workspace_attachment_output_path(
        workspace: &Path,
        file_name: &str,
    ) -> anyhow::Result<PathBuf> {
        let safe_name = Self::sanitize_attachment_filename(file_name)
            .ok_or_else(|| anyhow::anyhow!("invalid attachment filename: {file_name}"))?;

        tokio::fs::create_dir_all(workspace).await?;
        let workspace_root = tokio::fs::canonicalize(workspace)
            .await
            .unwrap_or_else(|_| workspace.to_path_buf());

        let save_dir = workspace.join(SLACK_ATTACHMENT_SAVE_SUBDIR);
        tokio::fs::create_dir_all(&save_dir).await?;
        let resolved_save_dir = tokio::fs::canonicalize(&save_dir).await.with_context(|| {
            format!(
                "failed to resolve Slack attachment save directory: {}",
                save_dir.display()
            )
        })?;

        if !resolved_save_dir.starts_with(&workspace_root) {
            anyhow::bail!(
                "Slack attachment save directory escapes workspace: {}",
                resolved_save_dir.display()
            );
        }

        Ok(resolved_save_dir.join(safe_name))
    }

    pub(crate) fn sanitize_attachment_filename(file_name: &str) -> Option<String> {
        let basename = Path::new(file_name).file_name()?.to_str()?.trim();
        if basename.is_empty() || basename == "." || basename == ".." {
            return None;
        }

        let sanitized: String = basename
            .replace(['/', '\\'], "_")
            .chars()
            .take(SLACK_ATTACHMENT_FILENAME_MAX_CHARS)
            .collect();
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            None
        } else {
            Some(sanitized)
        }
    }

    pub(crate) fn sanitize_file_id(file_id: &str) -> String {
        let cleaned: String = file_id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
            .take(64)
            .collect();
        if cleaned.is_empty() {
            "file".to_string()
        } else {
            cleaned
        }
    }

    pub(crate) fn ensure_file_extension(file_name: &str, extension: &str) -> String {
        if Path::new(file_name).extension().is_some() {
            file_name.to_string()
        } else {
            format!("{file_name}.{extension}")
        }
    }

    pub(crate) fn image_extension_for_mime(mime: &str) -> Option<&'static str> {
        match mime {
            "image/png" => Some("png"),
            "image/jpeg" => Some("jpg"),
            "image/webp" => Some("webp"),
            "image/gif" => Some("gif"),
            "image/bmp" => Some("bmp"),
            _ => None,
        }
    }

    pub(crate) fn file_extension(value: &str) -> Option<String> {
        let before_query = value.split('?').next().unwrap_or(value);
        before_query
            .rsplit('/')
            .next()
            .unwrap_or(before_query)
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
    }

    pub(crate) fn file_text_preview(file: &serde_json::Value) -> Option<String> {
        let preview = file
            .get("preview")
            .and_then(|value| value.as_str())
            .or_else(|| {
                file.get("preview_highlight")
                    .and_then(|value| value.as_str())
            })
            .or_else(|| {
                file.get("initial_comment")
                    .and_then(|comment| comment.get("comment"))
                    .and_then(|value| value.as_str())
            })?;
        Self::truncate_text(preview, SLACK_ATTACHMENT_TEXT_INLINE_MAX_CHARS)
    }

    pub(crate) fn truncate_text(value: &str, max_chars: usize) -> Option<String> {
        let mut out = String::new();
        let mut count = 0usize;
        for ch in value.chars() {
            if count >= max_chars {
                break;
            }
            out.push(ch);
            count += 1;
        }
        let was_truncated = count >= max_chars && value.chars().nth(max_chars).is_some();
        let mut out = out.trim().to_string();
        if out.is_empty() {
            return None;
        }
        if was_truncated {
            out.push_str("\n…[truncated]");
        }
        Some(out)
    }

    pub(crate) fn is_probably_text_file(file: &serde_json::Value) -> bool {
        if matches!(
            Self::slack_file_mode(file).as_deref(),
            Some("snippet" | "post")
        ) {
            return true;
        }

        if Self::slack_file_mime(file)
            .as_deref()
            .is_some_and(|mime| mime.starts_with("text/"))
        {
            return true;
        }

        if file
            .get("filetype")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
            .is_some_and(Self::is_text_filetype)
        {
            return true;
        }

        Self::file_extension(&Self::slack_file_name(file))
            .as_deref()
            .is_some_and(Self::is_text_filetype)
    }

    pub(crate) fn is_text_filetype(filetype: &str) -> bool {
        matches!(
            filetype,
            "txt"
                | "text"
                | "md"
                | "markdown"
                | "csv"
                | "tsv"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "html"
                | "css"
                | "js"
                | "ts"
                | "jsx"
                | "tsx"
                | "py"
                | "rs"
                | "go"
                | "java"
                | "kt"
                | "c"
                | "cc"
                | "cpp"
                | "h"
                | "hpp"
                | "cs"
                | "php"
                | "rb"
                | "swift"
                | "sql"
                | "log"
                | "ini"
                | "conf"
                | "cfg"
                | "env"
                | "sh"
                | "bash"
                | "zsh"
        )
    }

    pub(crate) fn is_image_file(file: &serde_json::Value) -> bool {
        if Self::slack_file_mime(file)
            .as_deref()
            .is_some_and(|mime| mime.starts_with("image/"))
        {
            return true;
        }

        if file
            .get("filetype")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
            .is_some_and(|filetype| Self::mime_from_extension(filetype).is_some())
        {
            return true;
        }

        Self::file_extension(&Self::slack_file_name(file))
            .as_deref()
            .is_some_and(|ext| Self::mime_from_extension(ext).is_some())
    }

    /// Audio file extensions accepted for voice transcription.
    const AUDIO_EXTENSIONS: &[&str] = &[
        "flac", "mp3", "mpeg", "mpga", "mp4", "m4a", "ogg", "oga", "opus", "wav", "webm",
    ];

    /// Check whether a Slack file object looks like an audio attachment
    /// (voice memo, audio message, or uploaded audio file).
    pub(crate) fn is_audio_file(file: &serde_json::Value) -> bool {
        // Slack voice messages use subtype "slack_audio"
        if let Some(subtype) = file.get("subtype").and_then(|v| v.as_str())
            && subtype == "slack_audio"
        {
            return true;
        }

        if Self::slack_file_mime(file)
            .as_deref()
            .is_some_and(|mime| mime.starts_with("audio/"))
        {
            return true;
        }

        if let Some(ft) = file
            .get("filetype")
            .and_then(|v| v.as_str())
            .map(|v| v.to_ascii_lowercase())
            && Self::AUDIO_EXTENSIONS.contains(&ft.as_str())
        {
            return true;
        }

        Self::file_extension(&Self::slack_file_name(file))
            .as_deref()
            .is_some_and(|ext| Self::AUDIO_EXTENSIONS.contains(&ext))
    }

    /// Download an audio file attachment and transcribe it using the configured
    /// transcription provider. Returns `None` if transcription is not configured
    /// or if the download/transcription fails.
    pub(crate) async fn try_transcribe_audio_file(
        &self,
        file: &serde_json::Value,
    ) -> Option<String> {
        let manager = self.transcription_manager.as_deref()?;

        let url = Self::slack_file_download_url(file)?;
        let file_name = Self::slack_file_name(file);
        let redacted_url = Self::redact_raw_slack_url(url);

        let resp = self.fetch_slack_private_file(url).await?;
        let status = resp.status();
        if !status.is_success() {
            tracing::warn!(
                "Slack voice file download failed for {} ({status})",
                redacted_url
            );
            return None;
        }

        let audio_data = match resp.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => {
                tracing::warn!("Slack voice file read failed for {}: {e}", redacted_url);
                return None;
            }
        };

        // Determine a filename with extension for the transcription API.
        let transcription_filename = if Self::file_extension(&file_name).is_some() {
            file_name.clone()
        } else {
            // Fall back to extension from mimetype or default to .ogg
            let mime_ext = Self::slack_file_mime(file)
                .and_then(|mime| mime.rsplit('/').next().map(|s| s.to_string()))
                .unwrap_or_else(|| "ogg".to_string());
            format!("voice.{mime_ext}")
        };

        match manager
            .transcribe(&audio_data, &transcription_filename)
            .await
        {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    tracing::info!("Slack voice transcription returned empty text, skipping");
                    None
                } else {
                    tracing::info!(
                        "Slack: transcribed voice file {} ({} chars)",
                        file_name,
                        trimmed.len()
                    );
                    Some(format!("[Voice] {trimmed}"))
                }
            }
            Err(e) => {
                tracing::warn!("Slack voice transcription failed for {}: {e}", file_name);
                Some(Self::format_attachment_summary(file))
            }
        }
    }

    pub(crate) async fn download_text_snippet(&self, file: &serde_json::Value) -> Option<String> {
        let url = Self::slack_file_download_url(file)?;
        let redacted_url = Self::redact_raw_slack_url(url);
        let resp = self.fetch_slack_private_file(url).await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));
            let sanitized = operant_providers::sanitize_api_error(&body);
            tracing::warn!(
                "Slack snippet fetch failed for {} ({status}): {sanitized}",
                redacted_url
            );
            return None;
        }

        if let Some(content_length) = resp.content_length() {
            let content_length = usize::try_from(content_length).unwrap_or(usize::MAX);
            if content_length > SLACK_ATTACHMENT_TEXT_DOWNLOAD_MAX_BYTES {
                tracing::warn!(
                    "Slack snippet download skipped for {}: content-length {} exceeds {} bytes",
                    redacted_url,
                    content_length,
                    SLACK_ATTACHMENT_TEXT_DOWNLOAD_MAX_BYTES
                );
                return None;
            }
        }

        let bytes = match resp.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!("Slack snippet body read failed for {}: {err}", redacted_url);
                return None;
            }
        };
        if bytes.is_empty() {
            return None;
        }
        if bytes.len() > SLACK_ATTACHMENT_TEXT_DOWNLOAD_MAX_BYTES {
            tracing::warn!(
                "Slack snippet body too large for {}: {} bytes exceeds {} bytes",
                redacted_url,
                bytes.len(),
                SLACK_ATTACHMENT_TEXT_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
        if bytes.contains(&0) {
            tracing::warn!("Slack snippet body appears binary for {}", redacted_url);
            return None;
        }

        let text = String::from_utf8_lossy(&bytes);
        Self::truncate_text(&text, SLACK_ATTACHMENT_TEXT_INLINE_MAX_CHARS)
    }

    pub(crate) fn format_snippet_attachment(file: &serde_json::Value, snippet: &str) -> String {
        let file_name = Self::slack_file_name(file);
        let language = file
            .get("filetype")
            .and_then(|value| value.as_str())
            .map(Self::sanitize_code_fence_language)
            .unwrap_or_else(|| "text".to_string());

        let fence = if snippet.contains("```") {
            "````"
        } else {
            "```"
        };
        format!("[SNIPPET:{file_name}]\n{fence}{language}\n{snippet}\n{fence}")
    }

    pub(crate) fn sanitize_code_fence_language(input: &str) -> String {
        let normalized = input
            .trim()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+'))
            .collect::<String>();
        if normalized.is_empty() {
            "text".to_string()
        } else {
            normalized
        }
    }

    pub(crate) fn format_attachment_summary(file: &serde_json::Value) -> String {
        let file_name = Self::slack_file_name(file);
        let mime = Self::slack_file_mime(file).unwrap_or_else(|| "unknown".to_string());
        let size = file
            .get("size")
            .and_then(|value| value.as_u64())
            .map(|value| format!("{value} bytes"))
            .unwrap_or_else(|| "unknown size".to_string());
        format!("[ATTACHMENT:{file_name} | mime={mime} | size={size}]")
    }

    pub(crate) fn extract_channel_ids(list_payload: &serde_json::Value) -> Vec<String> {
        let mut ids = list_payload
            .get("channels")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .filter_map(|channel| {
                let id = channel.get("id").and_then(|id| id.as_str())?;
                let is_archived = channel
                    .get("is_archived")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_member = channel
                    .get("is_member")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if is_archived || !is_member {
                    return None;
                }
                Some(id.to_string())
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub(crate) async fn list_accessible_channels(&self) -> anyhow::Result<Vec<String>> {
        let mut channels = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut query_params = vec![
                ("exclude_archived", "true".to_string()),
                ("limit", "200".to_string()),
                (
                    "types",
                    "public_channel,private_channel,mpim,im".to_string(),
                ),
            ];
            if let Some(ref next) = cursor {
                query_params.push(("cursor", next.clone()));
            }

            let resp = self
                .http_client()
                .get("https://slack.com/api/conversations.list")
                .bearer_auth(&self.bot_token)
                .query(&query_params)
                .send()
                .await?;

            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

            if !status.is_success() {
                let sanitized = operant_providers::sanitize_api_error(&body);
                anyhow::bail!("Slack conversations.list failed ({status}): {sanitized}");
            }

            let data: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            if data.get("ok") == Some(&serde_json::Value::Bool(false)) {
                let err = data
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown");
                anyhow::bail!("Slack conversations.list failed: {err}");
            }

            channels.extend(Self::extract_channel_ids(&data));

            cursor = data
                .get("response_metadata")
                .and_then(|rm| rm.get("next_cursor"))
                .and_then(|c| c.as_str())
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(ToOwned::to_owned);

            if cursor.is_none() {
                break;
            }
        }

        channels.sort();
        channels.dedup();
        Ok(channels)
    }

    pub(crate) fn slack_now_ts() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("{}.{:06}", now.as_secs(), now.subsec_micros())
    }

    pub(crate) fn ensure_poll_cursor(
        cursors: &mut HashMap<String, String>,
        channel_id: &str,
        now_ts: &str,
    ) -> String {
        cursors
            .entry(channel_id.to_string())
            .or_insert_with(|| now_ts.to_string())
            .clone()
    }

    /// Try to parse a Socket Mode `interactive` envelope as an approval button tap.
    ///
    /// Returns `Some((token, response))` when the first action's `action_id` matches
    /// `"approval_{TOKEN}_{approve|deny|always}"`, `None` otherwise.
    pub(crate) fn try_parse_approval_block_action(
        envelope: &serde_json::Value,
    ) -> Option<(String, ChannelApprovalResponse)> {
        let payload = envelope.get("payload")?;
        if payload.get("type").and_then(|v| v.as_str())? != "block_actions" {
            return None;
        }
        let action_id = payload
            .get("actions")
            .and_then(|a| a.as_array())
            .and_then(|a| a.first())
            .and_then(|a| a.get("action_id"))
            .and_then(|v| v.as_str())?;
        let rest = action_id.strip_prefix("approval_")?;
        let (token, action) = rest.rsplit_once('_')?;
        if token.len() != 6 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        let response = match action {
            "approve" => ChannelApprovalResponse::Approve,
            "deny" => ChannelApprovalResponse::Deny,
            "always" => ChannelApprovalResponse::AlwaysApprove,
            _ => return None,
        };
        Some((token.to_string(), response))
    }

    /// Parse a Socket Mode `interactive` envelope containing a `block_actions`
    /// payload from the `/config` Block Kit UI.  Translates provider/model
    /// dropdown selections into synthetic `/models <provider>` or `/model <id>`
    /// commands so the existing runtime command handler can apply them.
    pub(crate) fn parse_block_action_as_command(
        envelope: &serde_json::Value,
        _bot_user_id: &str,
    ) -> Option<ChannelMessage> {
        let payload = envelope.get("payload")?;

        let payload_type = payload.get("type").and_then(|v| v.as_str())?;
        if payload_type != "block_actions" {
            return None;
        }

        let actions = payload.get("actions").and_then(|v| v.as_array())?;
        let action = actions.first()?;

        let action_id = action.get("action_id").and_then(|v| v.as_str())?;
        let selected_value = action
            .get("selected_option")
            .and_then(|o| o.get("value"))
            .and_then(|v| v.as_str())?;

        let command = match action_id {
            "operant_config_provider" => format!("/models {selected_value}"),
            "operant_config_model" => format!("/model {selected_value}"),
            _ => return None,
        };

        let user = payload
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let channel_id = payload
            .get("channel")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if channel_id.is_empty() {
            tracing::warn!("Slack block_actions: missing channel ID in interactive payload");
            return None;
        }

        let ts = payload
            .get("message")
            .and_then(|m| m.get("ts"))
            .and_then(|v| v.as_str())
            .unwrap_or("0");

        Some(ChannelMessage {
            id: format!("slack_{channel_id}_{ts}_action"),
            sender: user.to_string(),
            reply_target: channel_id.to_string(),
            content: command,
            channel: "slack".to_string(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: payload
                .get("message")
                .and_then(|m| m.get("thread_ts"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            interruption_scope_id: None,
            attachments: vec![],
        })
    }

    pub(crate) async fn open_socket_mode_url(&self) -> anyhow::Result<String> {
        let app_token = self
            .configured_app_token()
            .ok_or_else(|| anyhow::anyhow!("Slack Socket Mode requires app_token"))?;

        let resp = self
            .http_client()
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(app_token)
            .send()
            .await?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

        if !status.is_success() {
            let sanitized = operant_providers::sanitize_api_error(&body);
            anyhow::bail!("Slack apps.connections.open failed ({status}): {sanitized}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = parsed
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Slack apps.connections.open failed: {err}");
        }

        parsed
            .get("url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow::anyhow!("Slack apps.connections.open did not return url"))
    }

    pub(crate) async fn listen_socket_mode(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
        bot_user_id: &str,
        scoped_channels: Option<Vec<String>>,
    ) -> anyhow::Result<()> {
        let mut last_ts_by_channel: HashMap<String, String> = HashMap::new();
        let mut open_url_attempt: u32 = 0;
        let mut socket_reconnect_attempt: u32 = 0;

        loop {
            let ws_url = match self.open_socket_mode_url().await {
                Ok(url) => {
                    open_url_attempt = 0;
                    url
                }
                Err(e) => {
                    let wait = Self::compute_socket_mode_retry_delay(open_url_attempt);
                    tracing::warn!(
                        "Slack Socket Mode: failed to open websocket URL: {e}; retrying in {:.3}s (attempt #{})",
                        wait.as_secs_f64(),
                        open_url_attempt.saturating_add(1),
                    );
                    open_url_attempt = open_url_attempt.saturating_add(1);
                    tokio::time::sleep(wait).await;
                    continue;
                }
            };

            let (ws_stream, _) = match operant_config::schema::ws_connect_with_proxy(
                &ws_url,
                "channel.slack",
                self.proxy_url.as_deref(),
            )
            .await
            {
                Ok(connection) => {
                    socket_reconnect_attempt = 0;
                    connection
                }
                Err(e) => {
                    let wait = Self::compute_socket_mode_retry_delay(socket_reconnect_attempt);
                    tracing::warn!(
                        "Slack Socket Mode: websocket connect failed: {e}; retrying in {:.3}s (attempt #{})",
                        wait.as_secs_f64(),
                        socket_reconnect_attempt.saturating_add(1),
                    );
                    socket_reconnect_attempt = socket_reconnect_attempt.saturating_add(1);
                    tokio::time::sleep(wait).await;
                    continue;
                }
            };
            tracing::info!("Slack Socket Mode: websocket connected");

            let (mut write, mut read) = ws_stream.split();

            while let Some(frame) = read.next().await {
                let text = match frame {
                    Ok(WsMessage::Text(text)) => text,
                    Ok(WsMessage::Ping(payload)) => {
                        if let Err(e) = write.send(WsMessage::Pong(payload)).await {
                            tracing::warn!("Slack Socket Mode: pong send failed: {e}");
                            break;
                        }
                        continue;
                    }
                    Ok(WsMessage::Close(_)) => {
                        tracing::warn!("Slack Socket Mode: websocket closed by server");
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        tracing::warn!("Slack Socket Mode: websocket read failed: {e}");
                        break;
                    }
                };

                let envelope: serde_json::Value = match serde_json::from_str(text.as_ref()) {
                    Ok(value) => value,
                    Err(e) => {
                        tracing::warn!("Slack Socket Mode: invalid JSON payload: {e}");
                        continue;
                    }
                };

                if let Some(envelope_id) = envelope.get("envelope_id").and_then(|v| v.as_str()) {
                    let ack = serde_json::json!({ "envelope_id": envelope_id });
                    if let Err(e) = write.send(WsMessage::Text(ack.to_string().into())).await {
                        tracing::warn!("Slack Socket Mode: ack send failed: {e}");
                        break;
                    }
                }

                let envelope_type = envelope
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if envelope_type == "disconnect" {
                    tracing::warn!("Slack Socket Mode: received disconnect event");
                    break;
                }

                // Handle interactive payloads (block_actions from /config UI or approval buttons).
                if envelope_type == "interactive" {
                    if let Some((token, response)) =
                        Self::try_parse_approval_block_action(&envelope)
                    {
                        let mut map = self.pending_approvals.lock().await;
                        if let Some(sender) = map.remove(&token) {
                            let _ = sender.send(response);
                        }
                        continue;
                    }
                    if let Some(msg) = Self::parse_block_action_as_command(&envelope, bot_user_id)
                        && tx.send(msg).await.is_err()
                    {
                        return Ok(());
                    }
                    continue;
                }

                if envelope_type != "events_api" {
                    continue;
                }

                let Some(event) = envelope
                    .get("payload")
                    .and_then(|payload| payload.get("event"))
                else {
                    continue;
                };
                let event_type = event
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                // Track assistant thread context for Assistants API status indicators.
                if event_type == "assistant_thread_started"
                    || event_type == "assistant_thread_context_changed"
                {
                    if let Some(thread) = event.get("assistant_thread") {
                        let ch = thread
                            .get("channel_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let tts = thread
                            .get("thread_ts")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        if !ch.is_empty()
                            && !tts.is_empty()
                            && let Ok(mut map) = self.active_assistant_thread.lock()
                        {
                            map.insert(ch.to_string(), tts.to_string());
                        }
                    }
                    continue;
                }

                // Handle reaction-based cancellation.
                if event_type == "reaction_added" {
                    if let Some(ref cancel_emoji) = self.cancel_reaction {
                        let reaction = event
                            .get("reaction")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        if reaction == cancel_emoji.as_str() {
                            let user = event
                                .get("user")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if !user.is_empty() && self.is_user_allowed(user) {
                                let item = event.get("item");
                                let item_channel = item
                                    .and_then(|i| i.get("channel"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default();
                                let item_ts = item
                                    .and_then(|i| i.get("ts"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default();
                                if !item_channel.is_empty() && !item_ts.is_empty() {
                                    // Build a synthetic /stop message scoped to the
                                    // thread of the reacted message so the dispatch
                                    // loop cancels the correct in-flight task.
                                    let thread_ts = Some(item_ts.to_string());
                                    let scope_id = Some(item_ts.to_string());
                                    let sender = self.resolve_sender_identity(user).await;
                                    let cancel_msg = ChannelMessage {
                                        id: format!("slack_{item_channel}_{item_ts}_cancel"),
                                        sender,
                                        reply_target: item_channel.to_string(),
                                        content: "/stop".to_string(),
                                        channel: "slack".to_string(),
                                        timestamp: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        thread_ts,
                                        interruption_scope_id: scope_id,
                                        attachments: vec![],
                                    };
                                    tracing::info!(
                                        "Slack: :{cancel_emoji}: reaction from {user} \
                                         on {item_channel}/{item_ts} — sending /stop"
                                    );
                                    if tx.send(cancel_msg).await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }

                if event_type != "message" {
                    continue;
                }
                let subtype = event.get("subtype").and_then(|v| v.as_str());
                if !Self::is_supported_message_subtype(subtype) {
                    continue;
                }

                let channel_id = event
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_default();
                if channel_id.is_empty() {
                    continue;
                }
                if let Some(ref configured_channels) = scoped_channels
                    && !configured_channels.iter().any(|id| id == &channel_id)
                {
                    continue;
                }

                let user = event
                    .get("user")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if user.is_empty() || user == bot_user_id {
                    continue;
                }
                if !self.is_user_allowed(user) {
                    tracing::warn!("Slack: ignoring message from unauthorized user: {user}");
                    continue;
                }

                let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or_default();
                if ts.is_empty() {
                    continue;
                }
                let last_ts = last_ts_by_channel
                    .get(&channel_id)
                    .map(String::as_str)
                    .unwrap_or_default();
                if ts <= last_ts {
                    continue;
                }

                let is_group_message = Self::is_group_channel_id(&channel_id);
                let is_thread_reply = event.get("thread_ts").and_then(|v| v.as_str()).is_some();
                let allow_sender_without_mention =
                    is_group_message && self.is_group_sender_trigger_enabled(user);
                let require_mention = self.mention_only
                    && is_group_message
                    && !allow_sender_without_mention
                    && (!is_thread_reply || self.strict_mention_in_thread);

                let Some(normalized_text) = self
                    .build_incoming_content(event, require_mention, bot_user_id)
                    .await
                else {
                    continue;
                };

                if let Some((token, response)) = crate::util::parse_approval_reply(&normalized_text)
                {
                    let mut map = self.pending_approvals.lock().await;
                    if let Some(ap_sender) = map.remove(&token) {
                        let _ = ap_sender.send(response);
                        continue;
                    }
                }

                last_ts_by_channel.insert(channel_id.clone(), ts.to_string());
                let sender = self.resolve_sender_identity(user).await;

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
                        Self::inbound_thread_ts(event, ts)
                    } else {
                        Self::inbound_thread_ts_genuine_only(event)
                    },
                    interruption_scope_id: Self::inbound_interruption_scope_id(event, ts),
                    attachments: vec![],
                };

                // Track thread context so start_typing can set assistant status.
                if let Some(ref tts) = channel_msg.thread_ts
                    && let Ok(mut map) = self.active_assistant_thread.lock()
                {
                    map.insert(channel_id.clone(), tts.clone());
                }

                if tx.send(channel_msg).await.is_err() {
                    return Ok(());
                }
            }

            let wait = Self::compute_socket_mode_retry_delay(socket_reconnect_attempt);
            tracing::warn!(
                "Slack Socket Mode: reconnecting in {:.3}s (attempt #{})...",
                wait.as_secs_f64(),
                socket_reconnect_attempt.saturating_add(1),
            );
            socket_reconnect_attempt = socket_reconnect_attempt.saturating_add(1);
            tokio::time::sleep(wait).await;
        }
    }

    pub(crate) fn parse_retry_after_secs(headers: &HeaderMap) -> Option<u64> {
        let value = headers
            .get(reqwest::header::RETRY_AFTER)?
            .to_str()
            .ok()?
            .trim();
        Self::parse_retry_after_value(value)
    }

    pub(crate) fn parse_retry_after_value(value: &str) -> Option<u64> {
        if value.is_empty() {
            return None;
        }

        if let Ok(seconds) = value.parse::<u64>() {
            return Some(seconds);
        }

        let truncated = value
            .split_once('.')
            .map(|(whole, _)| whole)
            .unwrap_or(value);
        truncated.parse::<u64>().ok()
    }

    pub(crate) fn jitter_ms(max_jitter_ms: u64) -> u64 {
        if max_jitter_ms == 0 {
            return 0;
        }
        rand::random::<u64>() % (max_jitter_ms + 1)
    }

    pub(crate) fn compute_exponential_backoff_delay(
        base_retry_after_secs: u64,
        attempt: u32,
        max_backoff_secs: u64,
        jitter_ms: u64,
    ) -> Duration {
        let multiplier = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let backoff_secs = base_retry_after_secs
            .saturating_mul(multiplier)
            .min(max_backoff_secs);
        Duration::from_secs(backoff_secs) + Duration::from_millis(jitter_ms)
    }

    pub(crate) fn compute_retry_delay(
        base_retry_after_secs: u64,
        attempt: u32,
        jitter_ms: u64,
    ) -> Duration {
        Self::compute_exponential_backoff_delay(
            base_retry_after_secs,
            attempt,
            SLACK_HISTORY_MAX_BACKOFF_SECS,
            jitter_ms,
        )
    }

    pub(crate) fn compute_socket_mode_retry_delay(attempt: u32) -> Duration {
        let jitter_ms = Self::jitter_ms(SLACK_SOCKET_MODE_MAX_JITTER_MS);
        Self::compute_exponential_backoff_delay(
            SLACK_SOCKET_MODE_INITIAL_BACKOFF_SECS,
            attempt,
            SLACK_SOCKET_MODE_MAX_BACKOFF_SECS,
            jitter_ms,
        )
    }

    pub(crate) fn next_retry_timestamp(wait: Duration) -> String {
        match chrono::Duration::from_std(wait) {
            Ok(delta) => (Utc::now() + delta).to_rfc3339(),
            Err(_) => Utc::now().to_rfc3339(),
        }
    }

    pub(crate) fn evaluate_health(
        bot_ok: bool,
        socket_mode_enabled: bool,
        socket_mode_ok: bool,
    ) -> bool {
        if !bot_ok {
            return false;
        }
        if socket_mode_enabled {
            return socket_mode_ok;
        }
        true
    }

    pub(crate) fn slack_api_call_succeeded(status: reqwest::StatusCode, body: &str) -> bool {
        if !status.is_success() {
            return false;
        }

        let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        parsed
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    pub(crate) async fn fetch_history_with_retry(
        &self,
        channel_id: &str,
        params: &[(&str, String)],
    ) -> Option<serde_json::Value> {
        let mut total_wait = Duration::from_secs(0);

        for attempt in 0..=SLACK_HISTORY_MAX_RETRIES {
            let resp = match self
                .http_client()
                .get("https://slack.com/api/conversations.history")
                .bearer_auth(&self.bot_token)
                .query(params)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Slack poll error for channel {channel_id}: {e}");
                    return None;
                }
            };

            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

            let is_ratelimited_http = status == reqwest::StatusCode::TOO_MANY_REQUESTS;
            let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let is_ratelimited_payload = payload.get("ok") == Some(&serde_json::Value::Bool(false))
                && payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .is_some_and(|err| err == "ratelimited");

            if is_ratelimited_http || is_ratelimited_payload {
                if attempt >= SLACK_HISTORY_MAX_RETRIES {
                    tracing::error!(
                        "Slack rate limit retries exhausted for conversations.history on channel {}. Total wait: {}s across {} attempts. Proceeding without channel history.",
                        channel_id,
                        total_wait.as_secs(),
                        SLACK_HISTORY_MAX_RETRIES
                    );
                    return None;
                }

                let retry_after_secs = Self::parse_retry_after_secs(&headers)
                    .unwrap_or(SLACK_HISTORY_DEFAULT_RETRY_AFTER_SECS);
                let jitter_ms = Self::jitter_ms(SLACK_HISTORY_MAX_JITTER_MS);
                let wait = Self::compute_retry_delay(retry_after_secs, attempt, jitter_ms);
                total_wait += wait;
                let next_retry_at = Self::next_retry_timestamp(wait);
                tracing::warn!(
                    "Slack conversations.history rate limited for channel {}. Retry-After: {}s. Attempt {}/{}. Next retry at {}.",
                    channel_id,
                    retry_after_secs,
                    attempt + 1,
                    SLACK_HISTORY_MAX_RETRIES,
                    next_retry_at
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            if !status.is_success() {
                let sanitized = operant_providers::sanitize_api_error(&body);
                tracing::warn!(
                    "Slack history request failed for channel {} ({}): {}",
                    channel_id,
                    status,
                    sanitized
                );
                return None;
            }

            if payload.get("ok") == Some(&serde_json::Value::Bool(false)) {
                let err = payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown");
                tracing::warn!("Slack history error for channel {channel_id}: {err}");
                return None;
            }

            return Some(payload);
        }

        None
    }

    pub(crate) async fn fetch_thread_replies_with_retry(
        &self,
        channel_id: &str,
        thread_ts: &str,
        oldest: &str,
    ) -> Option<serde_json::Value> {
        let mut total_wait = Duration::from_secs(0);

        for attempt in 0..=SLACK_HISTORY_MAX_RETRIES {
            let resp = match self
                .http_client()
                .get("https://slack.com/api/conversations.replies")
                .bearer_auth(&self.bot_token)
                .query(&[
                    ("channel", channel_id),
                    ("ts", thread_ts),
                    ("oldest", oldest),
                    ("limit", "50"),
                ])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        "Slack conversations.replies error for thread {thread_ts} in {channel_id}: {e}"
                    );
                    return None;
                }
            };

            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

            let is_ratelimited_http = status == reqwest::StatusCode::TOO_MANY_REQUESTS;
            let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let is_ratelimited_payload = payload.get("ok") == Some(&serde_json::Value::Bool(false))
                && payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .is_some_and(|err| err == "ratelimited");

            if is_ratelimited_http || is_ratelimited_payload {
                if attempt >= SLACK_HISTORY_MAX_RETRIES {
                    tracing::error!(
                        "Slack rate limit retries exhausted for conversations.replies on thread {} in channel {}. Total wait: {}s across {} attempts.",
                        thread_ts,
                        channel_id,
                        total_wait.as_secs(),
                        SLACK_HISTORY_MAX_RETRIES
                    );
                    return None;
                }

                let retry_after_secs = Self::parse_retry_after_secs(&headers)
                    .unwrap_or(SLACK_HISTORY_DEFAULT_RETRY_AFTER_SECS);
                let jitter_ms = Self::jitter_ms(SLACK_HISTORY_MAX_JITTER_MS);
                let wait = Self::compute_retry_delay(retry_after_secs, attempt, jitter_ms);
                total_wait += wait;
                let next_retry_at = Self::next_retry_timestamp(wait);
                tracing::warn!(
                    "Slack conversations.replies rate limited for thread {} in channel {}. Retry-After: {}s. Attempt {}/{}. Next retry at {}.",
                    thread_ts,
                    channel_id,
                    retry_after_secs,
                    attempt + 1,
                    SLACK_HISTORY_MAX_RETRIES,
                    next_retry_at
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            if !status.is_success() {
                let sanitized = operant_providers::sanitize_api_error(&body);
                tracing::warn!(
                    "Slack conversations.replies failed for thread {} in channel {} ({}): {}",
                    thread_ts,
                    channel_id,
                    status,
                    sanitized
                );
                return None;
            }

            if payload.get("ok") == Some(&serde_json::Value::Bool(false)) {
                let err = payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown");
                tracing::warn!(
                    "Slack conversations.replies error for thread {} in channel {}: {}",
                    thread_ts,
                    channel_id,
                    err
                );
                return None;
            }

            return Some(payload);
        }

        None
    }

    /// Extract thread parent timestamps from channel history messages.
    /// Returns `(thread_ts, latest_reply_ts)` pairs for messages with active threads.
    pub(crate) fn extract_active_threads(messages: &[serde_json::Value]) -> Vec<(String, String)> {
        messages
            .iter()
            .filter_map(|msg| {
                let thread_ts = msg.get("thread_ts").and_then(|v| v.as_str())?;
                let ts = msg.get("ts").and_then(|v| v.as_str()).unwrap_or_default();
                // Only consider messages that are thread parents (ts == thread_ts)
                if ts != thread_ts {
                    return None;
                }
                let reply_count = msg.get("reply_count").and_then(|v| v.as_u64()).unwrap_or(0);
                if reply_count == 0 {
                    return None;
                }
                let latest_reply = msg
                    .get("latest_reply")
                    .and_then(|v| v.as_str())
                    .unwrap_or(thread_ts);
                Some((thread_ts.to_string(), latest_reply.to_string()))
            })
            .collect()
    }

    /// Evict expired or excess threads from the active-thread tracker.
    /// Each value is `(channel_id, last_seen_reply_ts, last_activity)`.
    pub(crate) fn evict_stale_threads(
        active_threads: &mut HashMap<String, (String, String, Instant)>,
        now: Instant,
    ) {
        let max_age = Duration::from_secs(SLACK_POLL_THREAD_EXPIRE_SECS);
        active_threads
            .retain(|_, (_, _, last_activity)| now.duration_since(*last_activity) < max_age);
        if active_threads.len() > SLACK_POLL_ACTIVE_THREAD_MAX {
            let overflow = active_threads.len() - SLACK_POLL_ACTIVE_THREAD_MAX;
            let mut entries: Vec<_> = active_threads
                .iter()
                .map(|(k, (_, _, t))| (k.clone(), *t))
                .collect();
            entries.sort_by_key(|(_, t)| *t);
            for (key, _) in entries.into_iter().take(overflow) {
                active_threads.remove(&key);
            }
        }
    }
}
