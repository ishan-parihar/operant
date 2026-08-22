//! `telegram` — extracted verbatim from gateway/mod.rs.

use crate::config::runtime_config;
use crate::error::Result;
use crate::gateway_markdown::markdown_to_telegram_html;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::*;

/// Telegram adapter
pub struct TelegramAdapter {
    token: Option<String>,
    enabled: bool,
    running: Arc<AtomicBool>,
    client: reqwest::Client,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter
    pub fn new(token: Option<String>) -> Self {
        let enabled = token.is_some();
        Self {
            token,
            enabled,
            running: Arc::new(AtomicBool::new(false)),
            // Bounded timeouts: a wedged TCP connection must surface as an
            // error (and hit the poll-loop retry) instead of hanging
            // `getUpdates`/`sendMessage` forever. 30s long-poll fits in 60s.
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Create a Telegram adapter with full configuration
    pub fn with_config(
        token: Option<String>,
        _bot_username: Option<String>,
        _dm_topics_enabled: bool,
        proxy_url: Option<&str>,
    ) -> Self {
        let enabled = token.is_some();
        let mut client_builder = reqwest::Client::builder();
        if let Some(proxy) = proxy_url {
            if let Ok(proxy_obj) = reqwest::Proxy::all(proxy) {
                client_builder = client_builder.proxy(proxy_obj);
                tracing::info!("Telegram adapter using proxy: {}", proxy);
            } else {
                tracing::warn!("Invalid proxy URL, ignoring: {}", proxy);
            }
        }
        let client = client_builder
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            token,
            enabled,
            running: Arc::new(AtomicBool::new(false)),
            client,
        }
    }

    pub(crate) fn api_url(&self) -> String {
        let base = runtime_config().gateway.telegram_api_base;
        format!(
            "{}/bot{}",
            base.trim_end_matches('/'),
            self.token.as_ref().unwrap_or(&String::new())
        )
    }

    /// Send a message to a Telegram chat and return the message_id.
    /// Uses HTML parse_mode; falls back to plain text on 400 Bad Request.
    pub(crate) async fn send_telegram_inner(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<String> {
        let html = markdown_to_telegram_html(text);
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": html,
            "parse_mode": "HTML",
        });

        if let Some(reply) = reply_to {
            body["reply_to_message_id"] = serde_json::json!(reply);
        }
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        tracing::debug!(
            "Sending message to chat {} ({} chars)",
            channel_id,
            text.len()
        );
        let response = self
            .client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;

        let status = response.status();

        // If HTML parsing fails (400 Bad Request), retry as plain text
        if status.as_u16() == 400 {
            let html_err = serde_json::from_str::<serde_json::Value>(&response.text().await?)
                .ok()
                .and_then(|d| d["description"].as_str().map(String::from))
                .unwrap_or_else(|| "unknown".to_string());
            warn!(
                error = %html_err,
                len = text.len(),
                "HTML send failed (400), falling back to plain text"
            );
            let mut plain_body = serde_json::json!({
                "chat_id": channel_id,
                "text": text,
            });
            if let Some(reply) = reply_to {
                plain_body["reply_to_message_id"] = serde_json::json!(reply);
            }
            if let Some(tid) = thread_id {
                plain_body["message_thread_id"] = serde_json::json!(tid);
            }
            let resp = self
                .client
                .post(format!("{}/sendMessage", self.api_url()))
                .json(&plain_body)
                .send()
                .await?;
            if !resp.status().is_success() {
                let plain_status = resp.status();
                let plain_err = serde_json::from_str::<serde_json::Value>(&resp.text().await?)
                    .ok()
                    .and_then(|d| d["description"].as_str().map(String::from))
                    .unwrap_or_default();
                tracing::error!(
                    error = %plain_err,
                    "Plain-text send also failed for chat {}: HTTP {}",
                    channel_id,
                    plain_status
                );
                // Last resort (usually message-too-long): deliver a
                // truncated head so the answer is never fully lost.
                const MAX_TELEGRAM: usize = 4096;
                if text.len() > MAX_TELEGRAM - 100 {
                    let cut = text
                        .char_indices()
                        .nth(MAX_TELEGRAM - 200)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    let mut truncated = text[..cut].to_string();
                    truncated.push_str("\n… [truncated]");
                    let resp2 = self
                        .client
                        .post(format!("{}/sendMessage", self.api_url()))
                        .json(&serde_json::json!({
                            "chat_id": channel_id,
                            "text": truncated,
                        }))
                        .send()
                        .await;
                    match resp2 {
                        Ok(r) if r.status().is_success() => {
                            tracing::info!("Truncated fallback delivered for chat {}", channel_id);
                            return Ok(String::new());
                        }
                        _ => {
                            tracing::error!("All send attempts failed for chat {}", channel_id);
                            return Err(crate::error::Error::Agent(format!(
                                "Telegram rejected all send attempts ({})",
                                plain_err
                            )));
                        }
                    }
                }
                return Err(crate::error::Error::Agent(format!(
                    "Telegram rejected send: {}",
                    plain_err
                )));
            }
            let data: serde_json::Value = resp.json().await?;
            tracing::info!(
                "Message sent to chat {} via plain text, message_id: {:?}",
                channel_id,
                data["result"]["message_id"].as_i64()
            );
            return Ok(data["result"]["message_id"]
                .as_i64()
                .map(|id| id.to_string())
                .unwrap_or_default());
        }

        tracing::debug!("Sending message to chat {}", channel_id);
        let data: serde_json::Value = response.json().await?;
        tracing::info!(
            "Message sent to chat {}, message_id: {:?}",
            channel_id,
            data["result"]["message_id"].as_i64()
        );
        Ok(data["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default())
    }
}

/// Count UTF-16 code units in a string (Telegram's length metric).
pub(crate) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Split text into chunks that respect Telegram's 4096 UTF-16 code unit limit.
///
/// - Measures length using UTF-16 code units, not bytes or chars.
/// - Adds `(X/Y)` suffix indicators when multiple chunks are produced.
/// - Reserves 14 UTF-16 code units for the suffix ` (NNN/NNN)`.
/// - Splits at natural boundaries: prefers `\n\n`, then `\n`, then spaces.
/// - Code-block aware: avoids splitting inside ``` fences; if a split would
///   fall inside a code block, closes the fence and reopens it in the next chunk.
pub(crate) fn chunk_text(text: &str, max_chunk_size: usize) -> Vec<String> {
    const SUFFIX_RESERVE: usize = 14; // room for " (NNN/NNN)"
    const FENCE_CLOSE: &str = "\n```";

    if utf16_len(text) <= max_chunk_size {
        return vec![text.to_string()];
    }

    // First pass: estimate total chunks to know Y in (X/Y).
    // This is a rough estimate — we refine during actual splitting.
    let body_budget = max_chunk_size - SUFFIX_RESERVE;
    let estimated_chunks = utf16_len(text).div_ceil(body_budget);
    let estimated_chunks = estimated_chunks.max(1);

    // Second pass: actual splitting with code-block awareness.
    let mut chunks: Vec<String> = Vec::with_capacity(estimated_chunks);
    let mut remaining = text;
    // When continuing from a code block opened in the previous chunk,
    // holds the language tag so we can reopen the fence.
    let mut carry_lang: Option<String> = None;

    while !remaining.is_empty() {
        let prefix = if let Some(ref lang) = carry_lang {
            format!("```{}\n", lang)
        } else {
            String::new()
        };
        let prefix_utf16 = utf16_len(&prefix);
        let fence_close_utf16 = utf16_len(FENCE_CLOSE);

        // If everything remaining fits in one final chunk
        if prefix_utf16 + utf16_len(remaining) + SUFFIX_RESERVE <= max_chunk_size {
            chunks.push(format!("{}{}", prefix, remaining));
            break;
        }

        // How much body text we can fit after accounting for prefix,
        // a potential closing fence, and the suffix indicator.
        let headroom = max_chunk_size
            .saturating_sub(SUFFIX_RESERVE)
            .saturating_sub(prefix_utf16)
            .saturating_sub(fence_close_utf16);
        let headroom = if headroom < 1 {
            max_chunk_size / 2
        } else {
            headroom
        };

        // Find the largest codepoint prefix of `remaining` whose UTF-16
        // length is ≤ headroom.
        let cp_limit = utf16_char_limit(remaining, headroom);
        let region = &remaining[..cp_limit];

        // Find a natural split point: prefer \n\n, then \n, then space.
        let split_at = find_split_point(region, cp_limit);

        let chunk_body = &remaining[..split_at];
        // Skip leading whitespace on remaining for next iteration
        remaining = remaining[split_at..].trim_start();

        let mut full_chunk = prefix.clone();
        full_chunk.push_str(chunk_body);

        // Determine if we end inside an open code block.
        let (in_code, lang) = scan_code_blocks(chunk_body, carry_lang.as_deref());

        if in_code {
            full_chunk.push_str(FENCE_CLOSE);
            carry_lang = Some(lang);
        } else {
            carry_lang = None;
        }

        chunks.push(full_chunk);
    }

    // Append (X/Y) indicators when multiple chunks.
    if chunks.len() > 1 {
        let total = chunks.len();
        chunks = chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| format!("{} ({}/{})", chunk, i + 1, total))
            .collect();
    }

    chunks
}

/// Find the largest codepoint index such that `s[..index]` has UTF-16 length ≤ limit.
pub(crate) fn utf16_char_limit(s: &str, limit: usize) -> usize {
    let mut count = 0;
    let mut byte_pos = 0;
    for ch in s.chars() {
        let ch_utf16 = ch.len_utf16();
        if count + ch_utf16 > limit {
            break;
        }
        count += ch_utf16;
        byte_pos += ch.len_utf8();
    }
    byte_pos
}

/// Find a natural split point in `region` (a string slice of `remaining`).
/// Prefers double newlines, then single newlines, then spaces.
/// Falls back to `cp_limit` if no natural boundary found.
pub(crate) fn find_split_point(region: &str, cp_limit: usize) -> usize {
    // Prefer \n\n
    if let Some(pos) = region.rfind("\n\n") {
        let split = pos + 2; // include both newlines
        if split > cp_limit / 4 {
            return split;
        }
    }
    // Then \n
    if let Some(pos) = region.rfind('\n') {
        let split = pos + 1; // include the newline
        if split > cp_limit / 4 {
            return split;
        }
    }
    // Then space
    if let Some(pos) = region.rfind(' ')
        && pos > cp_limit / 4
    {
        return pos;
    }
    // Fallback: hard split at the limit
    cp_limit
}

/// Scan `chunk_body` for code block fences, starting from `carry_lang` state.
/// Returns (in_code_block, language_tag) at the end of the body.
pub(crate) fn scan_code_blocks(chunk_body: &str, carry_lang: Option<&str>) -> (bool, String) {
    let mut in_code = carry_lang.is_some();
    let mut lang = carry_lang.unwrap_or("").to_string();

    for line in chunk_body.lines() {
        let stripped = line.trim();
        if let Some(rest) = stripped.strip_prefix("```") {
            if in_code {
                in_code = false;
                lang = String::new();
            } else {
                in_code = true;
                let tag = rest.trim();
                lang = tag.split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }

    (in_code, lang)
}

/// Path used to persist the Telegram polling offset across restarts.
pub(crate) fn get_offset_path() -> PathBuf {
    std::env::current_dir()
        .ok()
        .map(|p| p.join("telegram_offset.txt"))
        .unwrap_or_else(|| PathBuf::from("telegram_offset.txt"))
}

#[async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        // Verify the token by getting bot info
        let response = self
            .client
            .get(format!("{}/getMe", self.api_url()))
            .send()
            .await?;

        if response.status().is_success() {
            info!("Telegram bot started successfully");
            Ok(())
        } else {
            Err(crate::error::Error::Agent(
                "Failed to verify Telegram bot token".to_string(),
            ))
        }
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        info!("Telegram adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let chunks = chunk_text(&message.content, 4000);
        for (i, chunk) in chunks.iter().enumerate() {
            let reply_to = if i == 0 {
                message.reply_to.as_deref()
            } else {
                None
            };
            self.send_telegram_inner(&message.channel_id, chunk, reply_to, message.thread_id)
                .await?;
        }
        Ok(())
    }

    fn send_typing(&self, channel_id: &str, thread_id: Option<i64>) -> Result<()> {
        let url = format!("{}/sendChatAction", self.api_url());
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "action": "typing",
        });
        // Route the typing indicator into the same forum topic the user
        // sent from — otherwise it shows in the general chat.
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
        tracing::debug!("Sent typing indicator to chat {}", channel_id);
        Ok(())
    }

    async fn send_message_return_id(&self, message: OutgoingMessage) -> Result<String> {
        let chunks = chunk_text(&message.content, 4000);
        let id = self
            .send_telegram_inner(
                &message.channel_id,
                &chunks[0],
                message.reply_to.as_deref(),
                message.thread_id,
            )
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(&message.channel_id, chunk, None, message.thread_id)
                .await?;
        }
        Ok(id)
    }

    async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let url = format!("{}/editMessageText", self.api_url());
        let html = markdown_to_telegram_html(&message.content);
        // An explicit empty inline keyboard clears any buttons on the edited
        // message — this is how approval/clarify prompts lose their buttons
        // once resolved (hermes `query.edit_message_text(reply_markup=None)`
        // parity). Harmless for plain stream edits (no keyboard to clear).
        let body = serde_json::json!({
            "chat_id": channel_id,
            "message_id": message_id,
            "text": html,
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": [] },
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if resp.status().as_u16() == 400 {
            // "message is not modified" — return existing ID.
            return Ok(message_id.to_string());
        }
        if !resp.status().is_success() {
            return Err(crate::error::Error::Agent(format!(
                "Telegram edit_message failed: HTTP {}",
                resp.status()
            )));
        }
        Ok(message_id.to_string())
    }

    fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let url = format!("{}/deleteMessage", self.api_url());
        let body = serde_json::json!({
            "chat_id": channel_id,
            "message_id": message_id,
        });
        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::error!("Telegram delete_message error: {}", e);
            }
        });
        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Shared callback-query routing (inline keyboard taps) first, then
        // the regular message parser. The polling loop uses the same static
        // helper so button taps work on every Telegram transport.
        if let Some(msg) = Self::handle_callback_update(
            &self.client,
            self.token.as_deref().unwrap_or_default(),
            &runtime_config().gateway.telegram_api_base,
            update.clone(),
        )
        .await
        {
            return Ok(Some(msg));
        }

        // Parse Telegram update — delegates to the static parse_update
        TelegramAdapter::parse_update(update)
    }

    async fn send_approval_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        tool_name: &str,
        description: &str,
    ) -> Result<Option<String>> {
        let prompt = format!(
            "🔧 Permission required: {tool_name} — {description}\nTap a button to allow (once / always) or cancel (60s timeout), or reply /approve, /approve always, /deny."
        );
        // Inline keyboard with tappable approve / always / deny buttons
        // (hermes `send_exec_approval` parity — `always` is the permanent
        // allowlist choice). Callback data uses the `approval:` prefix
        // handled in `handle_callback_update` below.
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": prompt,
            "reply_markup": {
                "inline_keyboard": [[
                    { "text": "✅ Approve", "callback_data": "approval:approve" },
                    { "text": "✅✅ Always", "callback_data": "approval:always" },
                    { "text": "❌ Deny", "callback_data": "approval:deny" }
                ]]
            }
        });
        // Route the prompt into the same forum topic the discussion is in —
        // without message_thread_id it lands in the general chat instead.
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp: serde_json::Value = self
            .client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        let msg_id = resp
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|m| m.as_i64())
            .map(|id| id.to_string());
        Ok(msg_id)
    }

    async fn send_choice_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        question: &str,
        choices: &[String],
    ) -> Result<()> {
        // One tappable button per choice (hermes `send_clarify` parity).
        // Index-based callback_data (`choice:<idx>`) stays under Telegram's
        // 64-byte payload limit; the dispatch layer maps the index back to
        // the option text when resolving the pending question.
        let rows: Vec<Vec<serde_json::Value>> = choices
            .iter()
            .enumerate()
            .map(|(i, c)| {
                vec![serde_json::json!({
                    "text": c,
                    "callback_data": format!("choice:{i}"),
                })]
            })
            .collect();
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": format!("❓ {question}"),
            "reply_markup": { "inline_keyboard": rows },
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        self.client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "telegram",
            "enabled": self.enabled,
            "has_token": self.token.is_some()
        })
    }

    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        self.start().await?;
        tracing::info!("Telegram token verified via getMe");

        let token = self.token.clone().unwrap_or_default();
        let base = runtime_config().gateway.telegram_api_base;
        let url = format!("{}/bot{}/getUpdates", base.trim_end_matches('/'), token);
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        let client = self.client.clone();
        let media_base = base.clone();
        let media_token = token.clone();

        tracing::info!("Telegram polling task spawned");
        // Panic-supervisor wrapper: a polling epoch that panics must be
        // respawned — a dead poll task inside a live gateway process is the
        // silent-death failure mode this adapter must never have. Clean
        // epochs (shutdown) exit here; 409-style recovery stays inside the
        // epoch's own 'restart loop.
        tokio::spawn(async move {
            loop {
                if !running.load(Ordering::SeqCst) || message_tx.is_closed() {
                    return;
                }
                let handle = tokio::spawn({
                    let running = running.clone();
                    let client = client.clone();
                    let url = url.clone();
                    let message_tx = message_tx.clone();
                    let media_base = media_base.clone();
                    let media_token = media_token.clone();
                    async move {
                        // ── OUTER SUPERVISED RESTART LOOP ──
                        // On 409 Conflict the inner loop breaks here, triggering a fresh
                        // startup probe (timeout=0) before re-entering the polling loop.
                        'restart: while running.load(Ordering::SeqCst) {
                            let mut offset: i64 = 0;

                            // === STARTUP PROBE: claim any pending updates before long-poll starts ===
                            if let Ok(resp) = client
                                .post(&url)
                                .json(&serde_json::json!({
                                    "offset": 0,
                                    "timeout": 0,
                                }))
                                .send()
                                .await
                                && let Ok(data) = resp.json::<serde_json::Value>().await
                                && let Some(updates) = data["result"].as_array()
                            {
                                for update in updates {
                                    if let Some(update_id) = update["update_id"].as_i64() {
                                        offset = update_id + 1;
                                    }
                                }
                            }
                            tracing::info!("Startup probe completed, initial offset: {}", offset);

                            // === LOAD SAVED OFFSET (persist across restarts) ===
                            let offset_path = get_offset_path();
                            if offset_path.exists()
                                && let Ok(saved) = tokio::fs::read_to_string(&offset_path).await
                                && let Ok(n) = saved.trim().parse::<i64>()
                                && n > offset
                            {
                                offset = n;
                            }
                            tracing::info!("Loaded saved offset: {}", offset);

                            // ── INNER POLLING LOOP ──
                            let mut retry_delay: u64 = 1;
                            let mut last_heartbeat = Instant::now();

                            // Defensive dedup window: remember recently processed
                            // update_ids so a duplicate delivery (offset-file race,
                            // restart edge case) can never double-process the same
                            // update and double-reply. Telegram guarantees no dupes
                            // when offset handling is correct — this is belt-and-braces.
                            const RECENT_UPDATE_WINDOW: usize = 256;
                            let mut recent_updates: std::collections::VecDeque<i64> =
                                std::collections::VecDeque::new();

                            tracing::info!("Entering main polling loop");
                            while running.load(Ordering::SeqCst) {
                                // Early exit if the gateway receiver has been dropped (clean shutdown).
                                if message_tx.is_closed() {
                                    info!("Telegram: message channel closed, stopping poll loop");
                                    running.store(false, Ordering::SeqCst);
                                    break;
                                }

                                // Heartbeat: log every 60s without receiving updates
                                if last_heartbeat.elapsed() >= Duration::from_secs(60) {
                                    info!("Polling active, last update offset: {}", offset);
                                    last_heartbeat = Instant::now();
                                }

                                let response = client
                                    .post(&url)
                                    .json(&serde_json::json!({
                                        "offset": offset,
                                        "timeout": 30,
                                    }))
                                    .send()
                                    .await;

                                let mut had_updates = false;

                                match response {
                                    Ok(resp) => {
                                        let status = resp.status();

                                        // Handle 409 Conflict — break to outer loop for a clean re-probe
                                        if status.as_u16() == 409 {
                                            tracing::warn!(
                                                "Telegram 409 Conflict (another instance?), restarting from probe in 35s"
                                            );
                                            tokio::time::sleep(Duration::from_secs(35)).await;
                                            break;
                                        }

                                        // Any other successful HTTP response resets the retry delay.
                                        retry_delay = 1;

                                        if let Ok(data) = resp.json::<serde_json::Value>().await
                                            && let Some(updates) = data["result"].as_array()
                                        {
                                            had_updates = !updates.is_empty();
                                            if had_updates {
                                                tracing::info!(
                                                    "Received {} update(s) from Telegram",
                                                    updates.len()
                                                );
                                                last_heartbeat = Instant::now();
                                            }
                                            for update in updates {
                                                let Some(update_id) = update["update_id"].as_i64()
                                                else {
                                                    continue;
                                                };
                                                // Skip anything already processed (defense
                                                // against duplicate delivery).
                                                if recent_updates.contains(&update_id) {
                                                    tracing::warn!(
                                                        update_id,
                                                        "Skipping duplicate Telegram update (already processed)"
                                                    );
                                                    continue;
                                                }
                                                offset = update_id + 1;
                                                // Route inline-keyboard taps (approval /
                                                // clarify buttons) before the regular
                                                // message parser — parse_update drops
                                                // callback_query updates entirely.
                                                let parsed = if let Some(m) =
                                                    TelegramAdapter::handle_callback_update(
                                                        &client,
                                                        &media_token,
                                                        &media_base,
                                                        update.clone(),
                                                    )
                                                    .await
                                                {
                                                    Some(m)
                                                } else {
                                                    TelegramAdapter::parse_update(update.clone())
                                                        .ok()
                                                        .flatten()
                                                };
                                                if let Some(mut msg) = parsed {
                                                    // Download attachments (photos, documents,
                                                    // voice/audio/video) into the local media
                                                    // cache so the agent can inspect them with
                                                    // native tools (hermes parity). Best-effort:
                                                    // a failed download keeps the placeholder
                                                    // text and still delivers the message.
                                                    let media = download_telegram_attachments(
                                                        &client,
                                                        &media_token,
                                                        &media_base,
                                                        update,
                                                    )
                                                    .await;
                                                    if !media.is_empty() {
                                                        msg = msg.with_media_urls(media);
                                                    }
                                                    tracing::info!(
                                                        "Sent message to gateway handler (chat: {}, content: {:.50}, media: {})",
                                                        msg.channel_id,
                                                        msg.content,
                                                        msg.media_urls.len()
                                                    );
                                                    if let Err(e) = message_tx.send(msg) {
                                                        tracing::error!(
                                                            "Failed to send message to gateway handler: {}",
                                                            e
                                                        );
                                                        // Receiver dropped — likely shutting down.
                                                        running.store(false, Ordering::SeqCst);
                                                        break;
                                                    }
                                                }
                                                recent_updates.push_back(update_id);
                                                if recent_updates.len() > RECENT_UPDATE_WINDOW {
                                                    recent_updates.pop_front();
                                                }
                                            }
                                        }

                                        // Persist offset to disk when updates were received
                                        if had_updates {
                                            let _ =
                                                std::fs::write(&offset_path, offset.to_string());
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Telegram polling error (retrying in {}s): {}",
                                            retry_delay,
                                            e
                                        );
                                        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
                                        retry_delay = (retry_delay * 2).min(30);
                                        continue; // Stay in inner loop, skip the 2s pause below
                                    }
                                }

                                // Only sleep when no updates arrived (long-poll timed out)
                                // so we don't add latency between receiving updates and polling again.
                                if !had_updates {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            }

                            // Before re-probing, check if we should shut down cleanly.
                            if !running.load(Ordering::SeqCst) || message_tx.is_closed() {
                                break 'restart;
                            }
                        }
                    } // async move (polling epoch)
                });
                match handle.await {
                    // Clean epoch exit — gateway is shutting down.
                    Ok(()) => return,
                    // Panicked epoch — log loudly and respawn.
                    Err(join_err) => {
                        error!("Telegram polling task panicked: {join_err} — respawning in 5s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });

        info!("Telegram bot started with polling");
        Ok(())
    }

    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let chunks = chunk_text(&message.content, 4000);
        let first_id = self
            .send_telegram_inner(
                channel_id,
                &chunks[0],
                message.reply_to.as_deref(),
                message.thread_id,
            )
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(channel_id, chunk, None, message.thread_id)
                .await?;
        }
        Ok(first_id)
    }

    async fn send_voice(&self, channel_id: &str, audio_data: &[u8], format: &str) -> Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendVoice",
            self.token.as_deref().unwrap_or("")
        );
        let filename = format!("voice.{}", format);
        let mime = match format {
            "ogg" | "opus" => "audio/ogg",
            "mp3" => "audio/mpeg",
            _ => "audio/wav",
        };
        let part = reqwest::multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename)
            .mime_str(mime)
            .map_err(|e| crate::error::Error::Agent(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", channel_id.to_string())
            .part("voice", part);
        let resp = self.client.post(&url).multipart(form).send().await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::error::Error::Agent(format!(
                "sendVoice failed: {}",
                body
            )));
        }
        Ok(())
    }
}

impl TelegramAdapter {
    /// Process a Telegram `callback_query` update (inline keyboard tap) and
    /// synthesize the equivalent inbound message, if any.
    ///
    /// Two tap families are supported, mirroring hermes' button contract:
    ///
    /// 1. `approval:<action>` — tool-permission Approve/Deny buttons. The tap
    ///    is answered (spinner dismissed) and synthesized into a `/approve`
    ///    or `/deny` text command that flows through the shared command
    ///    resolver — identical to a typed reply. The prompt message's
    ///    chat/message ids ride in `IncomingMessage.raw["approval_callback"]`
    ///    so the dispatch layer can edit the prompt to show the outcome
    ///    (hermes `resolve_gateway_approval` + `query.edit_message_text`
    ///    parity).
    /// 2. `choice:<idx>` — clarify() multiple-choice buttons. Synthesized
    ///    with a `choice_callback` raw marker; the dispatch layer resolves
    ///    the pending user question with the selected option text and edits
    ///    the prompt message.
    ///
    /// Returns `None` for non-callback updates (delegate to `parse_update`).
    /// Shared by `handle_update` and the Telegram polling loop so button
    /// taps work on every transport.
    pub(crate) async fn handle_callback_update(
        client: &reqwest::Client,
        token: &str,
        base: &str,
        update: serde_json::Value,
    ) -> Option<IncomingMessage> {
        let cb = update.get("callback_query")?;
        let data = cb.get("data").and_then(serde_json::Value::as_str)?;
        let cb_id = cb
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let api = format!("{}/bot{}", base.trim_end_matches('/'), token);

        // ── Tool-permission approve / always / deny taps ─────────────────
        if let Some(action) = data.strip_prefix("approval:") {
            let label = match action {
                "approve" => "✅ Approved",
                "always" => "✅✅ Always allowed",
                "deny" => "❌ Denied",
                _ => "⚠️ Unknown action",
            };
            // Answer the callback so the inline keyboard stops its spinner
            // (best-effort; a failure must not drop the tap).
            let _ = client
                .post(format!("{api}/answerCallbackQuery"))
                .json(&serde_json::json!({ "callback_query_id": cb_id, "text": label }))
                .send()
                .await;
            // Synthesize the equivalent text command so the shared
            // gateway_commands resolver handles the tap identically to a
            // typed reply.
            return Self::approval_message_from_callback(cb);
        }

        // ── clarify() multiple-choice taps ───────────────────────────────
        if data.starts_with("choice:") {
            let _ = client
                .post(format!("{api}/answerCallbackQuery"))
                .json(&serde_json::json!({
                    "callback_query_id": cb_id,
                    "text": "✅ Selected",
                }))
                .send()
                .await;
            return Self::choice_message_from_callback(cb);
        }

        None
    }

    /// Synthesize the `/approve` / `/deny` text command that an inline-keyboard
    /// tap resolves to, so taps and text replies share the gateway_commands
    /// resolution path (hermes `send_exec_approval` parity). Pure — no I/O —
    /// so it is unit-testable without a live client. Carries the thread_id
    /// and an `approval_callback` raw marker (prompt chat/message ids, tap
    /// user and action) so the dispatch layer can edit the prompt message to
    /// reflect the outcome.
    pub(crate) fn approval_message_from_callback(
        cb: &serde_json::Value,
    ) -> Option<IncomingMessage> {
        let action = cb
            .get("data")
            .and_then(serde_json::Value::as_str)
            .and_then(|d| d.strip_prefix("approval:"))?;
        let message = cb.get("message")?;
        let chat = message.get("chat")?;
        let channel_id = chat
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .map(|i| i.to_string())?;
        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());
        let is_group = matches!(
            chat.get("type").and_then(|t| t.as_str()),
            Some("group" | "supergroup")
        );
        let user_name = cb
            .get("from")
            .and_then(|f| f.get("first_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("User")
            .to_string();
        Some(
            IncomingMessage::new(
                "telegram",
                cb.get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                cb.get("from")
                    .and_then(|f| f.get("username"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                channel_id.clone(),
                match action {
                    "approve" => "/approve".to_string(),
                    "always" => "/approve always".to_string(),
                    "deny" => "/deny".to_string(),
                    other => format!("/approve-unknown-{other}"),
                },
            )
            .with_thread_id(thread_id)
            .with_group_chat(is_group)
            .with_raw(serde_json::json!({
                "approval_callback": {
                    "chat_id": channel_id,
                    "message_id": message.get("message_id").and_then(serde_json::Value::as_i64),
                    "thread_id": thread_id,
                    "user": user_name,
                    "action": action,
                }
            })),
        )
    }

    /// Synthesize the inbound message a clarify() button tap produces. Pure —
    /// no I/O — so it is unit-testable. The message carries a `choice_callback`
    /// raw marker (prompt chat/message ids, tap user and selected index) so
    /// the dispatch layer can resolve the pending question with the option
    /// text and edit the prompt message.
    pub(crate) fn choice_message_from_callback(cb: &serde_json::Value) -> Option<IncomingMessage> {
        let idx_str = cb
            .get("data")
            .and_then(serde_json::Value::as_str)
            .and_then(|d| d.strip_prefix("choice:"))?;
        let message = cb.get("message")?;
        let chat = message.get("chat")?;
        let channel_id = chat
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .map(|i| i.to_string())?;
        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());
        let is_group = matches!(
            chat.get("type").and_then(|t| t.as_str()),
            Some("group" | "supergroup")
        );
        let user_name = cb
            .get("from")
            .and_then(|f| f.get("first_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("User")
            .to_string();
        let idx = idx_str.parse::<usize>().unwrap_or(0);
        Some(
            IncomingMessage::new(
                "telegram",
                cb.get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                user_name.clone(),
                channel_id.clone(),
                format!("__choice__{idx}"),
            )
            .with_thread_id(thread_id)
            .with_group_chat(is_group)
            .with_raw(serde_json::json!({
                "choice_callback": {
                    "chat_id": channel_id,
                    "message_id": message.get("message_id").and_then(serde_json::Value::as_i64),
                    "thread_id": thread_id,
                    "user": user_name,
                    "idx": idx,
                }
            })),
        )
    }
}

/// Telegram bot API file path for a message attachment, if any.
///
/// Returns the `file_id` for the largest photo size, or the document/voice/
/// video/audio/sticker file — mirroring hermes' `_media_message_type`
/// extraction. `None` for plain text messages.
pub(crate) fn telegram_attachment_file_id(message: &serde_json::Value) -> Option<String> {
    if let Some(photos) = message.get("photo").and_then(|p| p.as_array()) {
        // PhotoSize list is sorted by size ascending — take the largest.
        return photos
            .last()
            .and_then(|p| p.get("file_id"))
            .and_then(|f| f.as_str())
            .map(|s| s.to_string());
    }
    for key in [
        "document",
        "voice",
        "video",
        "audio",
        "video_note",
        "sticker",
    ] {
        if let Some(fid) = message
            .get(key)
            .and_then(|v| v.get("file_id"))
            .and_then(|f| f.as_str())
        {
            return Some(fid.to_string());
        }
    }
    None
}

/// Guess a file extension for a Telegram file path / mime-ish hint.
pub(crate) fn telegram_media_extension(file_path: &str, fallback: &str) -> String {
    let lower = file_path.to_lowercase();
    let known = [
        ".jpg", ".jpeg", ".png", ".webp", ".gif", ".mp4", ".ogg", ".oga", ".mp3", ".m4a", ".wav",
        ".opus", ".pdf", ".txt", ".md", ".docx", ".xlsx", ".csv", ".json", ".zip", ".tar", ".gz",
        ".py", ".rs", ".toml", ".yaml", ".yml", ".html",
    ];
    for ext in known {
        if lower.ends_with(ext) {
            return ext.to_string();
        }
    }
    fallback.to_string()
}

/// Download a Telegram attachment (photo/document/voice/video) to the local
/// media cache (`~/.operant/media/`) so the agent can inspect it with native
/// tools. Hermes `cache_image_from_bytes`/`cache_audio_from_bytes` parity —
/// Telegram file URLs are ephemeral (~1h), so the gateway must persist them.
///
/// Returns the local file path on success. Failures log a warning and return
/// None — an attachment that can't be cached degrades to the placeholder text
/// rather than dropping the message.
pub(crate) async fn download_telegram_attachment(
    client: &reqwest::Client,
    token: &str,
    base: &str,
    file_id: &str,
    file_name_hint: Option<&str>,
) -> Option<String> {
    // 1. Resolve file path via getFile
    let api_base = base.trim_end_matches('/');
    let get_file_url = format!("{api_base}/bot{token}/getFile");
    let resp = client
        .post(&get_file_url)
        .json(&serde_json::json!({ "file_id": file_id }))
        .send()
        .await
        .ok()?;
    let data: serde_json::Value = resp.json().await.ok()?;
    let file_path = data["result"]["file_path"].as_str()?;

    // 2. Download the bytes
    let download_url = format!("{api_base}/file/bot{token}/{file_path}");
    let bytes = client
        .get(&download_url)
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    if bytes.is_empty() {
        return None;
    }

    // 3. Persist to ~/.operant/media/ with a stable, descriptive name
    let media_dir = crate::platform::operant_home().join("media");
    if std::fs::create_dir_all(&media_dir).is_err() {
        return None;
    }
    let ext = telegram_media_extension(
        file_path,
        file_name_hint
            .and_then(|f| std::path::Path::new(f).extension().and_then(|e| e.to_str()))
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_else(|| ".bin".to_string())
            .as_str(),
    );
    let file_name = file_name_hint
        .and_then(|f| {
            let stem = std::path::Path::new(f)
                .file_stem()
                .and_then(|s| s.to_str())?
                .to_string();
            Some(stem)
        })
        .unwrap_or_else(|| "attachment".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Sanitize the stem so it can't escape the media dir.
    let safe_stem: String = file_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dest = media_dir.join(format!("{safe_stem}_{ts}{ext}"));
    if std::fs::write(&dest, &bytes).is_err() {
        return None;
    }
    tracing::info!(path = %dest.display(), file_id = %file_id, "Cached Telegram attachment");
    Some(dest.to_string_lossy().to_string())
}

/// Download all attachments on a Telegram update message into the local media
/// cache. Returns the list of cached local paths (empty for text-only
/// messages).
pub(crate) async fn download_telegram_attachments(
    client: &reqwest::Client,
    token: &str,
    base: &str,
    update: &serde_json::Value,
) -> Vec<String> {
    let message = match update.get("message") {
        Some(m) => m,
        None => return Vec::new(),
    };
    let file_id = match telegram_attachment_file_id(message) {
        Some(fid) => fid,
        None => return Vec::new(),
    };
    let file_name_hint = message
        .get("document")
        .and_then(|d| d.get("file_name"))
        .and_then(|f| f.as_str());
    match download_telegram_attachment(client, token, base, &file_id, file_name_hint).await {
        Some(path) => vec![path],
        None => {
            tracing::warn!(
                file_id = %file_id,
                "Failed to cache Telegram attachment — degraded to placeholder text"
            );
            Vec::new()
        }
    }
}

/// Telegram update message keys that indicate a *service* message — no user
/// content: topic lifecycle, membership changes, pins, migrations, payments,
/// etc. These must be filtered in `parse_update` before the attachment
/// fallbacks, otherwise they surface to the agent as "[sent an attachment]"
/// and spawn a bogus agent turn. That was the double-reply bug when a user
/// creates a forum topic and types a message: the `forum_topic_created`
/// service message got its own turn (→ "I received another attachment")
/// *in addition to* the real text turn.
pub(crate) const TELEGRAM_SERVICE_MESSAGE_KEYS: &[&str] = &[
    "forum_topic_created",
    "forum_topic_closed",
    "forum_topic_reopened",
    "forum_topic_edited",
    "general_forum_topic_hidden",
    "general_forum_topic_unhidden",
    "new_chat_members",
    "left_chat_member",
    "new_chat_title",
    "new_chat_photo",
    "delete_chat_photo",
    "group_chat_created",
    "supergroup_chat_created",
    "channel_chat_created",
    "message_auto_delete_timer_changed",
    "migrate_to_chat_id",
    "migrate_from_chat_id",
    "pinned_message",
    "chat_background_set",
    "video_chat_started",
    "video_chat_ended",
    "video_chat_scheduled",
    "video_chat_participants_invited",
    "proximity_alert_triggered",
    "boost_added",
    "user_shared",
    "chat_shared",
    "write_access_allowed",
    "connected_website",
    "passport_data",
    "successful_payment",
    "refunded_payment",
    "invoice",
    "giveaway_created",
    "giveaway_completed",
    "giveaway_winners",
];

/// True when a Telegram update message is a service message (topic created,
/// member joined/left, pinned, migrated, …) carrying no user content.
pub(crate) fn telegram_message_is_service(message: &serde_json::Value) -> bool {
    TELEGRAM_SERVICE_MESSAGE_KEYS
        .iter()
        .any(|k| message.get(*k).is_some())
}

impl TelegramAdapter {
    /// Parse a Telegram update into an IncomingMessage.
    /// This is the same logic used by handle_update but callable without a trait object.
    pub(crate) fn parse_update(update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        let message = match update.get("message") {
            Some(m) => m,
            None => return Ok(None),
        };

        // Filter out messages from bots
        if let Some(from) = message.get("from")
            && from
                .get("is_bot")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
        {
            return Ok(None);
        }

        // Service messages (forum topic created, member joined/left, pinned,
        // migrated, …) carry no user content — skip them entirely instead of
        // fabricating an "[sent an attachment]" placeholder that spawns a
        // bogus agent turn.
        if telegram_message_is_service(message) {
            return Ok(None);
        }

        let chat = match message.get("chat") {
            Some(c) => c,
            None => return Ok(None),
        };

        let from = message.get("from");

        let content = if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
            text.to_string()
        } else if message.get("photo").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a photo]")
                .to_string()
        } else if let Some(doc) = message.get("document") {
            let filename = doc
                .get("file_name")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown");
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("[sent a document: {}]", filename))
        } else if message.get("voice").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a voice message]")
                .to_string()
        } else if message.get("video").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a video]")
                .to_string()
        } else if message.get("video_note").is_some() {
            "[sent a video note]".to_string()
        } else if message.get("animation").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent an animation]")
                .to_string()
        } else if message.get("audio").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent an audio message]")
                .to_string()
        } else if message.get("sticker").is_some() {
            "[sent a sticker]".to_string()
        } else {
            // No text, no caption, and no recognized media — not a user
            // message (unknown service updates fall here). Skip rather than
            // fabricate an attachment the agent can't act on.
            return Ok(None);
        };

        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());

        if content.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            IncomingMessage::new(
                "telegram",
                from.and_then(|f| f.get("id"))
                    .and_then(|id| id.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                from.and_then(|f| f.get("username"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("unknown"),
                chat.get("id")
                    .and_then(|id| id.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                content,
            )
            .with_group_chat(matches!(
                chat.get("type").and_then(|t| t.as_str()),
                Some("group" | "supergroup")
            ))
            .with_raw(update)
            .with_thread_id(thread_id),
        ))
    }
}
