//! Inherent methods on `TelegramChannel` extracted verbatim.

use anyhow::Context;
use directories::UserDirs;
use operant_api::channel::{Channel, ChannelMessage, SendMessage};
use operant_config::schema::{Config, StreamMode};
use operant_runtime::security::pairing::PairingGuard;
use reqwest::multipart::{Form, Part};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::fs;

use super::*;

impl TelegramChannel {
    pub fn new(bot_token: String, allowed_users: Vec<String>, mention_only: bool) -> Self {
        let normalized_allowed = Self::normalize_allowed_users(allowed_users);
        let pairing = if normalized_allowed.is_empty() {
            let guard = PairingGuard::new(true, &[]);
            if let Some(code) = guard.pairing_code() {
                println!("  🔐 Telegram pairing required. One-time bind code: {code}");
                println!("     Send `{TELEGRAM_BIND_COMMAND} <code>` from your Telegram account.");
            }
            Some(guard)
        } else {
            None
        };

        Self {
            bot_token,
            allowed_users: Arc::new(RwLock::new(normalized_allowed)),
            pairing,
            client: reqwest::Client::new(),
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 1000,
            last_draft_edit: Mutex::new(std::collections::HashMap::new()),
            typing_handle: Mutex::new(None),
            mention_only,
            bot_username: Mutex::new(None),
            api_base: "https://api.telegram.org".to_string(),
            transcription: None,
            transcription_manager: None,
            voice_transcriptions: Mutex::new(std::collections::HashMap::new()),
            workspace_dir: None,
            ack_reactions: true,
            tts_config: None,
            voice_chats: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            pending_voice: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            proxy_url: None,
            tool_command_specs: Vec::new(),
            pending_approvals: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            pending_choices: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            approval_timeout_secs: 120,
            dm_topics_enabled: false,
            dm_topic_name: "General".to_string(),
            dm_topic_threads: std::sync::Mutex::new(std::collections::HashMap::new()),
            last_chat_id: std::sync::Mutex::new(None),
            link_previews_enabled: true,
            typing_cooldown_secs: 30.0,
            typing_cooldown_until: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            fallback_ips: Vec::new(),
            fallback_ip_index: Arc::new(parking_lot::Mutex::new(0)),
            poll_recovery: PollRecoveryState::new(),
            poll_watchdogs: Mutex::new(Vec::new()),
        }
    }

    /// Override the approval prompt timeout (default 120s).
    pub fn with_approval_timeout_secs(mut self, secs: u64) -> Self {
        self.approval_timeout_secs = secs;
        self
    }

    /// Configure whether Telegram-native acknowledgement reactions are sent.
    pub fn with_ack_reactions(mut self, enabled: bool) -> Self {
        self.ack_reactions = enabled;
        self
    }

    /// Set fallback IPs for the Bot API host (hermes `fallback_ips` parity).
    /// When set, `http_client()` pins the API host to one of these IPs,
    /// rotating on each rebuild, instead of relying on DNS.
    pub fn with_fallback_ips(mut self, ips: Vec<String>) -> Self {
        self.fallback_ips = ips;
        self
    }

    /// Set a per-channel proxy URL that overrides the global proxy config.
    pub fn with_proxy_url(mut self, proxy_url: Option<String>) -> Self {
        self.proxy_url = proxy_url;
        self
    }

    /// Store pre-computed tool command specs for bot command registration.
    pub fn with_tool_command_specs(mut self, specs: Vec<(String, String)>) -> Self {
        self.tool_command_specs = specs;
        self
    }

    /// Configure workspace directory for saving downloaded attachments.
    pub fn with_workspace_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.workspace_dir = Some(dir);
        self
    }

    /// Configure streaming mode for progressive draft updates.
    pub fn with_streaming(
        mut self,
        stream_mode: StreamMode,
        draft_update_interval_ms: u64,
    ) -> Self {
        self.stream_mode = stream_mode;
        self.draft_update_interval_ms = draft_update_interval_ms;
        self
    }

    /// Override the Telegram Bot API base URL.
    /// Useful for local Bot API servers or testing.
    pub fn with_api_base(mut self, api_base: String) -> Self {
        self.api_base = api_base;
        self
    }

    /// Configure voice transcription.
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

    /// Configure text-to-speech for outgoing voice replies.
    pub fn with_tts(mut self, config: operant_config::schema::TtsConfig) -> Self {
        if config.enabled {
            self.tts_config = Some(config);
        }
        self
    }

    /// Enable per-DM forum topics: each DM chat gets its own topic created
    /// on first contact and replies are routed into it. Thread ids are
    /// persisted across restarts (hermes `ensure_dm_topic` parity).
    pub fn with_dm_topics(mut self, enabled: bool, topic_name: String) -> Self {
        self.dm_topics_enabled = enabled;
        let name = topic_name.trim();
        if !name.is_empty() {
            self.dm_topic_name = name.to_string();
        }
        self
    }

    /// Toggle Telegram link previews on outbound messages (hermes
    /// `disable_link_previews` parity). Pass `true` to keep previews on.
    pub fn with_link_previews(mut self, enabled: bool) -> Self {
        self.link_previews_enabled = enabled;
        self
    }

    /// Configure the per-chat typing-indicator cooldown after transient
    /// send failures (hermes `typing_cooldown_seconds` parity). Default 30s.
    pub fn with_typing_cooldown_secs(mut self, secs: f64) -> Self {
        self.typing_cooldown_secs = if secs > 0.0 { secs } else { 1.0 };
        self
    }

    // ── DM topics (hermes `_setup_dm_topics` / `ensure_dm_topic` parity) ──

    /// Path to the DM-topic state file: `<config_dir>/telegram_dm_topics.json`
    /// with the same `OPERANT_CONFIG_DIR` > `$HOME/.operant` precedence the
    /// rest of the config uses.
    pub(crate) fn dm_topic_state_path() -> std::path::PathBuf {
        let dir = std::env::var("OPERANT_CONFIG_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|h| std::path::PathBuf::from(h).join(".operant"))
            })
            .unwrap_or_else(|| {
                directories::UserDirs::new()
                    .map(|u| u.home_dir().join(".operant"))
                    .unwrap_or_default()
            });
        dir.join("telegram_dm_topics.json")
    }

    /// Load the persisted `chat_id -> thread_id` map from the state file.
    pub(crate) fn load_dm_topic_state(&self) {
        let path = Self::dm_topic_state_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(map): Result<std::collections::HashMap<String, i64>, _> =
            serde_json::from_str(&text)
        else {
            return;
        };
        let mut cache = self.dm_topic_threads.lock().unwrap();
        for (chat, tid) in map {
            cache.insert(chat, tid);
        }
    }

    /// Persist the `chat_id -> thread_id` map to the state file.
    pub(crate) fn persist_dm_topic_state(&self) {
        let path = Self::dm_topic_state_path();
        let map = self.dm_topic_threads.lock().unwrap().clone();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Return the DM-topic thread id for a chat, creating the forum topic
    /// on first contact and persisting the id (hermes `ensure_dm_topic`
    /// parity). Returns `None` when topics are disabled, the chat is not a
    /// numeric DM id, or the API call fails (log-only — the reply still
    /// goes to the chat root).
    pub(crate) async fn ensure_dm_topic(&self, chat_id: &str) -> Option<i64> {
        if !self.dm_topics_enabled {
            return None;
        }
        let chat_id_int: i64 = chat_id.trim().parse().ok()?;

        if let Some(&tid) = self.dm_topic_threads.lock().unwrap().get(chat_id) {
            return Some(tid);
        }

        // Create the topic via createForumTopic. Icon color is derived from
        // the chat id so each user's topic is visually distinct.
        let icon_color = (chat_id_int as u32).wrapping_mul(2654435761) % 0x0F_FF_FF;
        let body = serde_json::json!({
            "chat_id": chat_id_int,
            "name": self.dm_topic_name,
            "icon_color": icon_color,
        });
        let resp = match self
            .http_client()
            .post(self.api_url("createForumTopic"))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(chat_id, error = %e, "dm-topic create request failed");
                return None;
            }
        };

        if !resp.status().is_success() {
            tracing::warn!(
                chat_id,
                status = %resp.status(),
                "dm-topic create failed (chat may not support forum topics)"
            );
            return None;
        }

        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(chat_id, error = %e, "dm-topic create parse failed");
                return None;
            }
        };
        let thread_id = data
            .get("result")
            .and_then(|r| r.get("message_thread_id"))
            .and_then(serde_json::Value::as_i64)?;

        self.dm_topic_threads
            .lock()
            .unwrap()
            .insert(chat_id.to_string(), thread_id);
        self.persist_dm_topic_state();
        tracing::info!(
            chat_id,
            thread_id,
            topic = %self.dm_topic_name,
            "Created DM topic"
        );
        Some(thread_id)
    }

    /// Parse reply_target into (chat_id, optional thread_id).
    pub(crate) fn parse_reply_target(reply_target: &str) -> (String, Option<String>) {
        if let Some((chat_id, thread_id)) = reply_target.split_once(':') {
            (chat_id.to_string(), Some(thread_id.to_string()))
        } else {
            (reply_target.to_string(), None)
        }
    }

    pub(crate) fn extract_update_message_target(
        update: &serde_json::Value,
    ) -> Option<(String, i64)> {
        let message = update.get("message")?;
        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)?
            .to_string();
        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)?;
        Some((chat_id, message_id))
    }

    pub(crate) fn try_add_ack_reaction_nonblocking(&self, chat_id: String, message_id: i64) {
        let client = self.http_client();
        let url = self.api_url("setMessageReaction");
        let emoji = random_telegram_ack_reaction().to_string();
        let body = build_telegram_ack_reaction_request(&chat_id, message_id, &emoji);

        tokio::spawn(async move {
            let response = match client.post(&url).json(&body).send().await {
                Ok(resp) => resp,
                Err(err) => {
                    tracing::warn!(
                        "Telegram: failed to add ACK reaction to chat_id={chat_id}, message_id={message_id}: {err}"
                    );
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let err_body = response.text().await.unwrap_or_default();
                tracing::warn!(
                    "Telegram: add ACK reaction failed for chat_id={chat_id}, message_id={message_id}: status={status}, body={err_body}"
                );
            }
        });
    }

    pub(crate) fn http_client(&self) -> reqwest::Client {
        build_telegram_api_client(
            &self.api_base,
            self.proxy_url.as_deref(),
            &self.fallback_ips,
            &self.fallback_ip_index,
        )
    }

    pub(crate) fn normalize_identity(value: &str) -> String {
        value.trim().trim_start_matches('@').to_string()
    }

    pub(crate) fn normalize_allowed_users(allowed_users: Vec<String>) -> Vec<String> {
        allowed_users
            .into_iter()
            .map(|entry| Self::normalize_identity(&entry))
            .filter(|entry| !entry.is_empty())
            .collect()
    }

    pub(crate) async fn load_config_without_env() -> anyhow::Result<Config> {
        let home = UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("Could not find home directory")?;
        let operant_dir = home.join(".operant");
        let config_path = operant_dir.join("config.toml");

        let contents = fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let mut config: Config = toml::from_str(&contents).context(
            "Failed to parse config.toml — check [channels.telegram] section for syntax errors",
        )?;
        config.config_path = config_path;
        config.workspace_dir = operant_dir.join("workspace");
        Ok(config)
    }

    pub(crate) async fn persist_allowed_identity(&self, identity: &str) -> anyhow::Result<()> {
        let mut config = Self::load_config_without_env().await?;
        let Some(telegram) = config.channels.telegram.as_mut() else {
            anyhow::bail!(
                "Missing [channels.telegram] section in config.toml. \
                Add bot_token and allowed_users under [channels.telegram], \
                or run `operant onboard --channels-only` to configure interactively"
            );
        };

        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            anyhow::bail!("Cannot persist empty Telegram identity");
        }

        if !telegram.allowed_users.iter().any(|u| u == &normalized) {
            telegram.allowed_users.push(normalized);
            config
                .save()
                .await
                .context("Failed to persist Telegram allowlist to config.toml")?;
        }

        Ok(())
    }

    pub(crate) fn add_allowed_identity_runtime(&self, identity: &str) {
        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            return;
        }
        if let Ok(mut users) = self.allowed_users.write()
            && !users.iter().any(|u| u == &normalized)
        {
            users.push(normalized);
        }
    }

    pub(crate) fn extract_bind_code(text: &str) -> Option<&str> {
        let mut parts = text.split_whitespace();
        let command = parts.next()?;
        let base_command = command.split('@').next().unwrap_or(command);
        if base_command != TELEGRAM_BIND_COMMAND {
            return None;
        }
        parts.next().map(str::trim).filter(|code| !code.is_empty())
    }

    pub(crate) fn pairing_code_active(&self) -> bool {
        self.pairing
            .as_ref()
            .and_then(PairingGuard::pairing_code)
            .is_some()
    }

    pub(crate) fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{method}", self.api_base, self.bot_token)
    }

    /// `link_preview_options` value for outbound sendMessage bodies when
    /// link previews are disabled (hermes `_link_preview_kwargs` parity).
    /// `None` when previews stay on, so the field is omitted entirely.
    pub(crate) fn link_preview_json(&self) -> Option<serde_json::Value> {
        if self.link_previews_enabled {
            None
        } else {
            Some(serde_json::json!({ "is_disabled": true }))
        }
    }

    /// Register the bot's slash commands with Telegram via `setMyCommands`.
    /// Called once at startup so that users see a command menu when pressing `/`.
    /// Includes built-in runtime commands, user-installed skill commands, and
    /// enabled tool commands from the configuration.
    pub(crate) async fn register_bot_commands(&self) {
        let mut commands: Vec<serde_json::Value> = vec![
            serde_json::json!({ "command": "new",    "description": "Start a new conversation session" }),
            serde_json::json!({ "command": "stop",   "description": "Cancel the current in-flight task" }),
            serde_json::json!({ "command": "model",  "description": "Show or switch the current model" }),
            serde_json::json!({ "command": "models", "description": "List available providers or switch provider" }),
            serde_json::json!({ "command": "config", "description": "Show current configuration" }),
        ];

        // Track registered names to deduplicate across skills and tools.
        let mut used_names: std::collections::HashSet<String> = commands
            .iter()
            .filter_map(|c| c.get("command").and_then(|v| v.as_str()).map(String::from))
            .collect();

        // Collect commands from installed skills.
        if let Some(ref workspace_dir) = self.workspace_dir {
            let skills = operant_runtime::skills::load_skills(workspace_dir);

            for skill in &skills {
                let sanitized = sanitize_telegram_command_name(&skill.name);
                if sanitized.is_empty() {
                    tracing::debug!(
                        "Skipping skill '{}': name produces empty Telegram command",
                        skill.name
                    );
                    continue;
                }
                if used_names.contains(&sanitized) {
                    tracing::debug!(
                        "Skipping skill '{}': command /{sanitized} conflicts with an existing command",
                        skill.name
                    );
                    continue;
                }
                let description = if skill.description.is_empty() {
                    format!("Run the {name} skill", name = skill.name)
                } else {
                    truncate_telegram_command_description(&skill.description)
                };
                used_names.insert(sanitized.clone());
                commands.push(serde_json::json!({
                    "command": sanitized,
                    "description": description,
                }));
            }
        }

        // Collect commands from enabled tools.
        for (name, description) in &self.tool_command_specs {
            let sanitized = sanitize_telegram_command_name(name);
            if sanitized.is_empty() || used_names.contains(&sanitized) {
                continue;
            }
            used_names.insert(sanitized.clone());
            commands.push(serde_json::json!({
                "command": sanitized,
                "description": truncate_telegram_command_description(description),
            }));
        }

        // Telegram allows at most 100 commands.
        let total_before_cap = commands.len();
        commands.truncate(TELEGRAM_MAX_BOT_COMMANDS);
        if total_before_cap > TELEGRAM_MAX_BOT_COMMANDS {
            tracing::warn!(
                "Telegram limits bots to {TELEGRAM_MAX_BOT_COMMANDS} commands; \
                 {total_before_cap} configured, registering first {TELEGRAM_MAX_BOT_COMMANDS}. \
                 Reduce installed skills to expose more commands."
            );
        }

        let url = self.api_url("setMyCommands");
        let body = serde_json::json!({ "commands": commands });

        match self.http_client().post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    "Telegram bot commands registered successfully ({} commands)",
                    commands.len()
                );
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                tracing::warn!("Failed to register Telegram bot commands: {status} — {text}");
            }
            Err(e) => {
                tracing::warn!("Failed to register Telegram bot commands: {e}");
            }
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Check whether a voice reply should be queued for the given recipient and
    /// content. Shared between `send()` and `finalize_draft()` so the TTS
    /// voice-reply path works regardless of `stream_mode`.
    ///
    /// When `immediate` is `true` (called from `finalize_draft`), the 10-second
    /// debounce is skipped and `synthesize_and_send_voice` is called directly,
    /// since the text is already the final response.
    pub(crate) fn try_queue_voice_reply(&self, recipient: &str, content: &str, immediate: bool) {
        let is_voice_chat = self
            .voice_chats
            .lock()
            .map(|vs| vs.contains(recipient))
            .unwrap_or(false);

        if !is_voice_chat || self.tts_config.is_none() {
            return;
        }

        // Only queue substantive natural-language replies for voice.
        // Skip tool outputs: URLs, JSON, code blocks, errors, short status.
        let is_substantive = content.len() > 40
            && !content.starts_with("http")
            && !content.starts_with('{')
            && !content.starts_with('[')
            && !content.starts_with("Error")
            && !content.contains("```")
            && !content.contains("tool_call")
            && !content.contains("wttr.in");

        if !is_substantive {
            return;
        }

        let (chat_id, thread_id) = Self::parse_reply_target(recipient);
        let voice_chats = self.voice_chats.clone();
        let api_base = self.api_base.clone();
        let bot_token = self.bot_token.clone();
        let tts_config = self.tts_config.clone().unwrap();

        if immediate {
            // Finalize path: text is already the final answer — no debounce.
            let text = content.to_string();
            let recipient = recipient.to_string();
            tokio::spawn(async move {
                if let Ok(mut vc) = voice_chats.lock() {
                    vc.remove(&recipient);
                }
                match Self::synthesize_and_send_voice(
                    &api_base,
                    &bot_token,
                    &chat_id,
                    thread_id.as_deref(),
                    &text,
                    &tts_config,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("Telegram: voice reply sent ({} chars)", text.len());
                    }
                    Err(e) => {
                        tracing::warn!("Telegram: TTS voice reply failed: {e}");
                    }
                }
            });
            return;
        }

        // Send path: debounce to coalesce multi-part tool-chain responses.
        if let Ok(mut pv) = self.pending_voice.lock() {
            pv.insert(
                recipient.to_string(),
                (content.to_string(), std::time::Instant::now()),
            );
        }

        let pending = self.pending_voice.clone();
        let recipient = recipient.to_string();
        tokio::spawn(async move {
            // Wait 10 seconds — long enough for the agent to finish its
            // full tool chain and send the final answer.
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            // Atomic check-and-remove: only one task gets the value
            let to_voice = pending.lock().ok().and_then(|mut pv| {
                if let Some((_, ts)) = pv.get(&recipient)
                    && ts.elapsed().as_secs() >= 8
                {
                    return pv.remove(&recipient).map(|(text, _)| text);
                }
                None
            });

            if let Some(text) = to_voice {
                if let Ok(mut vc) = voice_chats.lock() {
                    vc.remove(&recipient);
                }
                match Self::synthesize_and_send_voice(
                    &api_base,
                    &bot_token,
                    &chat_id,
                    thread_id.as_deref(),
                    &text,
                    &tts_config,
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!("Telegram: voice reply sent ({} chars)", text.len());
                    }
                    Err(e) => {
                        tracing::warn!("Telegram: TTS voice reply failed: {e}");
                    }
                }
            }
        });
    }

    /// Synthesize text to speech and send as a Telegram voice note (static version for spawned tasks).
    pub(crate) async fn synthesize_and_send_voice(
        api_base: &str,
        bot_token: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        text: &str,
        tts_config: &operant_config::schema::TtsConfig,
    ) -> anyhow::Result<()> {
        let tts_manager = crate::tts::TtsManager::new(tts_config)?;
        let audio_bytes = tts_manager.synthesize(text).await?;
        let audio_len = audio_bytes.len();
        tracing::info!("Telegram TTS: synthesized {audio_len} bytes of audio");

        if audio_bytes.is_empty() {
            anyhow::bail!("TTS returned empty audio");
        }

        let url = format!("{api_base}/bot{bot_token}/sendVoice");
        let client = operant_config::schema::build_runtime_proxy_client("channel.telegram");

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part(
                "voice",
                reqwest::multipart::Part::bytes(audio_bytes)
                    .file_name("voice.ogg")
                    .mime_str("audio/ogg")?,
            );

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        let resp = client.post(&url).multipart(form).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("sendVoice failed: status={status}, body={body}");
        }

        tracing::info!("Telegram TTS: sent voice note ({audio_len} bytes)");
        Ok(())
    }

    pub(crate) async fn classify_edit_message_response(
        resp: reqwest::Response,
    ) -> EditMessageResult {
        if resp.status().is_success() {
            return EditMessageResult::Success;
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("message is not modified") {
            return EditMessageResult::NotModified;
        }

        EditMessageResult::Failed(status)
    }

    pub(crate) async fn fetch_bot_username(&self) -> anyhow::Result<String> {
        let resp = self.http_client().get(self.api_url("getMe")).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch bot info: {}", resp.status());
        }

        let data: serde_json::Value = resp.json().await?;
        let username = data
            .get("result")
            .and_then(|r| r.get("username"))
            .and_then(|u| u.as_str())
            .context("Bot username not found in response")?;

        Ok(username.to_string())
    }

    pub(crate) async fn get_bot_username(&self) -> Option<String> {
        {
            let cache = self.bot_username.lock();
            if let Some(ref username) = *cache {
                return Some(username.clone());
            }
        }

        match self.fetch_bot_username().await {
            Ok(username) => {
                let mut cache = self.bot_username.lock();
                *cache = Some(username.clone());
                Some(username)
            }
            Err(e) => {
                tracing::warn!("Failed to fetch bot username: {e}");
                None
            }
        }
    }

    pub(crate) fn is_telegram_username_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    pub(crate) fn find_bot_mention_spans(text: &str, bot_username: &str) -> Vec<(usize, usize)> {
        let bot_username = bot_username.trim_start_matches('@');
        if bot_username.is_empty() {
            return Vec::new();
        }

        let mut spans = Vec::new();

        for (at_idx, ch) in text.char_indices() {
            if ch != '@' {
                continue;
            }

            if at_idx > 0 {
                let prev = text[..at_idx].chars().next_back().unwrap_or(' ');
                if Self::is_telegram_username_char(prev) {
                    continue;
                }
            }

            let username_start = at_idx + 1;
            let mut username_end = username_start;

            for (rel_idx, candidate_ch) in text[username_start..].char_indices() {
                if Self::is_telegram_username_char(candidate_ch) {
                    username_end = username_start + rel_idx + candidate_ch.len_utf8();
                } else {
                    break;
                }
            }

            if username_end == username_start {
                continue;
            }

            let mention_username = &text[username_start..username_end];
            if mention_username.eq_ignore_ascii_case(bot_username) {
                spans.push((at_idx, username_end));
            }
        }

        spans
    }

    pub(crate) fn contains_bot_mention(text: &str, bot_username: &str) -> bool {
        !Self::find_bot_mention_spans(text, bot_username).is_empty()
    }

    pub(crate) fn normalize_incoming_content(text: &str, bot_username: &str) -> Option<String> {
        let spans = Self::find_bot_mention_spans(text, bot_username);
        if spans.is_empty() {
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return (!normalized.is_empty()).then_some(normalized);
        }

        let mut normalized = String::with_capacity(text.len());
        let mut cursor = 0;
        for (start, end) in spans {
            normalized.push_str(&text[cursor..start]);
            cursor = end;
        }
        normalized.push_str(&text[cursor..]);

        let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
        (!normalized.is_empty()).then_some(normalized)
    }

    pub(crate) fn is_group_message(message: &serde_json::Value) -> bool {
        message
            .get("chat")
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "group" || t == "supergroup")
            .unwrap_or(false)
    }

    /// Apply the `mention_only` gate to a non-text update (photo / document /
    /// voice) using its caption as the channel for the mention.
    ///
    /// Returns:
    /// - `Some(None)` — gate does not apply (DM, or `mention_only = false`,
    ///   or the message is not in a group). The caller should use the raw
    ///   caption / transcript as-is.
    /// - `Some(Some(normalized))` — caption mentions the bot; the mention
    ///   has been stripped and the resulting text is suitable for use as
    ///   message content.
    /// - `None` — gated and rejected; the caller must drop the update
    ///   without performing any expensive work (no download, no
    ///   transcription).
    ///
    /// Voice notes typically arrive without a caption, so under
    /// `mention_only = true` they are rejected here before transcription
    /// runs. If a future change wants to honor a verbal mention inside the
    /// transcript, this gate would need to be split into a pre-download and
    /// a post-transcription stage. See #6229.
    pub(crate) fn check_media_mention_gate(
        &self,
        message: &serde_json::Value,
        caption: Option<&str>,
    ) -> Option<Option<String>> {
        let is_group = Self::is_group_message(message);
        if !self.mention_only || !is_group {
            return Some(caption.map(String::from));
        }
        let bot_username_guard = self.bot_username.lock();
        let bot_username = bot_username_guard.as_ref()?;
        let caption = caption?;
        if !Self::contains_bot_mention(caption, bot_username) {
            return None;
        }
        Some(Self::normalize_incoming_content(caption, bot_username))
    }

    pub(crate) fn is_user_allowed(&self, username: &str) -> bool {
        let identity = Self::normalize_identity(username);
        self.allowed_users
            .read()
            .map(|users| users.iter().any(|u| u == "*" || u == &identity))
            .unwrap_or(false)
    }

    pub(crate) fn is_any_user_allowed<'a, I>(&self, identities: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        identities.into_iter().any(|id| self.is_user_allowed(id))
    }

    pub(crate) async fn handle_unauthorized_message(&self, update: &serde_json::Value) {
        let Some(message) = update.get("message") else {
            return;
        };

        let Some(text) = message.get("text").and_then(serde_json::Value::as_str) else {
            return;
        };

        let username_opt = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str);
        let username = username_opt.unwrap_or("unknown");
        let normalized_username = Self::normalize_identity(username);

        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64);
        let sender_id_str = sender_id.map(|id| id.to_string());
        let normalized_sender_id = sender_id_str.as_deref().map(Self::normalize_identity);

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let Some(chat_id) = chat_id else {
            tracing::warn!("Telegram: missing chat_id in message, skipping");
            return;
        };

        let mut identities = vec![normalized_username.as_str()];
        if let Some(ref id) = normalized_sender_id {
            identities.push(id.as_str());
        }

        if self.is_any_user_allowed(identities.iter().copied()) {
            return;
        }

        if let Some(code) = Self::extract_bind_code(text) {
            if let Some(pairing) = self.pairing.as_ref() {
                match pairing.try_pair(code, &chat_id).await {
                    Ok(Some(_token)) => {
                        let bind_identity = normalized_sender_id.clone().or_else(|| {
                            if normalized_username.is_empty() || normalized_username == "unknown" {
                                None
                            } else {
                                Some(normalized_username.clone())
                            }
                        });

                        if let Some(identity) = bind_identity {
                            self.add_allowed_identity_runtime(&identity);
                            match Box::pin(self.persist_allowed_identity(&identity)).await {
                                Ok(()) => {
                                    let _ = self
                                        .send(&SendMessage::new(
                                            "✅ Telegram account bound successfully. You can talk to Operant now.",
                                            &chat_id,
                                        ))
                                        .await;
                                    tracing::info!(
                                        "Telegram: paired and allowlisted identity={identity}"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Telegram: failed to persist allowlist after bind: {e}"
                                    );
                                    let _ = self
                                        .send(&SendMessage::new(
                                            "⚠️ Bound for this runtime, but failed to persist config. Access may be lost after restart; check config file permissions.",
                                            &chat_id,
                                        ))
                                        .await;
                                }
                            }
                        } else {
                            let _ = self
                                .send(&SendMessage::new(
                                    "❌ Could not identify your Telegram account. Ensure your account has a username or stable user ID, then retry.",
                                    &chat_id,
                                ))
                                .await;
                        }
                    }
                    Ok(None) => {
                        let _ = self
                            .send(&SendMessage::new(
                                "❌ Invalid binding code. Ask operator for the latest code and retry.",
                                &chat_id,
                            ))
                            .await;
                    }
                    Err(lockout_secs) => {
                        let _ = self
                            .send(&SendMessage::new(
                                format!("⏳ Too many invalid attempts. Retry in {lockout_secs}s."),
                                &chat_id,
                            ))
                            .await;
                    }
                }
            } else {
                let _ = self
                    .send(&SendMessage::new(
                        "ℹ️ Telegram pairing is not active. Ask operator to add your user ID to channels.telegram.allowed_users in config.toml.",
                        &chat_id,
                    ))
                    .await;
            }
            return;
        }

        tracing::warn!(
            "Telegram: ignoring message from unauthorized user: username={username}, sender_id={}. \
Allowlist Telegram username (without '@') or numeric user ID.",
            sender_id_str.as_deref().unwrap_or("unknown")
        );

        let suggested_identity = normalized_sender_id
            .clone()
            .or_else(|| {
                if normalized_username.is_empty() || normalized_username == "unknown" {
                    None
                } else {
                    Some(normalized_username.clone())
                }
            })
            .unwrap_or_else(|| "YOUR_TELEGRAM_ID".to_string());

        let _ = self
            .send(&SendMessage::new(
                format!(
                    "🔐 This bot requires operator approval.\n\nCopy this command to operator terminal:\n`operant channel bind-telegram {suggested_identity}`\n\nAfter operator runs it, send your message again."
                ),
                &chat_id,
            ))
            .await;

        if self.pairing_code_active() {
            let _ = self
                .send(&SendMessage::new(
                    "ℹ️ If operator provides a one-time pairing code, you can also run `/bind <code>`.",
                    &chat_id,
                ))
                .await;
        }
    }

    /// Get the file path for a Telegram file ID via the Bot API.
    pub(crate) async fn get_file_path(&self, file_id: &str) -> anyhow::Result<String> {
        let url = self.api_url("getFile");
        let resp = self
            .http_client()
            .get(&url)
            .query(&[("file_id", file_id)])
            .send()
            .await
            .context("Failed to call Telegram getFile")?;

        let data: serde_json::Value = resp.json().await?;
        data.get("result")
            .and_then(|r| r.get("file_path"))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
            .context("Telegram getFile: missing file_path in response")
    }

    /// Download a file from the Telegram CDN.
    pub(crate) async fn download_file(&self, file_path: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!(
            "https://api.telegram.org/file/bot{}/{file_path}",
            self.bot_token
        );
        let resp = self
            .http_client()
            .get(&url)
            .send()
            .await
            .context("Failed to download Telegram file")?;

        if !resp.status().is_success() {
            anyhow::bail!("Telegram file download failed: {}", resp.status());
        }

        Ok(resp.bytes().await?.to_vec())
    }

    /// Extract (file_id, duration) from a voice or audio message.
    pub(crate) fn parse_voice_metadata(message: &serde_json::Value) -> Option<(String, u64)> {
        let voice = message.get("voice").or_else(|| message.get("audio"))?;
        let file_id = voice.get("file_id")?.as_str()?.to_string();
        let duration = voice
            .get("duration")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Some((file_id, duration))
    }

    /// Extract attachment metadata from an incoming Telegram message (document or photo).
    ///
    /// Returns `None` for text-only, voice, and other unsupported message types.
    pub(crate) fn parse_attachment_metadata(
        message: &serde_json::Value,
    ) -> Option<IncomingAttachment> {
        // Try document first
        if let Some(doc) = message.get("document") {
            let file_id = doc.get("file_id")?.as_str()?.to_string();
            let file_name = doc
                .get("file_name")
                .and_then(serde_json::Value::as_str)
                .map(String::from);
            let file_size = doc.get("file_size").and_then(serde_json::Value::as_u64);
            let caption = message
                .get("caption")
                .and_then(serde_json::Value::as_str)
                .map(String::from);
            return Some(IncomingAttachment {
                file_id,
                file_name,
                file_size,
                caption,
                kind: IncomingAttachmentKind::Document,
            });
        }

        // Try photo (array of PhotoSize, take last = highest resolution)
        if let Some(photos) = message.get("photo").and_then(serde_json::Value::as_array) {
            let best = photos.last()?;
            let file_id = best.get("file_id")?.as_str()?.to_string();
            let file_size = best.get("file_size").and_then(serde_json::Value::as_u64);
            let caption = message
                .get("caption")
                .and_then(serde_json::Value::as_str)
                .map(String::from);
            return Some(IncomingAttachment {
                file_id,
                file_name: None,
                file_size,
                caption,
                kind: IncomingAttachmentKind::Photo,
            });
        }

        None
    }

    /// Attempt to parse a Telegram update as a document/photo attachment.
    ///
    /// Downloads the file to `{workspace_dir}/telegram_files/` and returns a
    /// `ChannelMessage` with the local file path. Returns `None` if the message
    /// is not an attachment, workspace_dir is not configured, or the file exceeds
    /// size limits.
    pub(crate) async fn try_parse_attachment_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let message = update.get("message")?;
        let attachment = Self::parse_attachment_metadata(message)?;

        // Check file size limit
        if let Some(size) = attachment.file_size
            && size > TELEGRAM_MAX_FILE_DOWNLOAD_BYTES
        {
            tracing::info!(
                "Skipping attachment: file size {size} bytes exceeds {} MB limit",
                TELEGRAM_MAX_FILE_DOWNLOAD_BYTES / (1024 * 1024)
            );
            return None;
        }

        let (username, sender_id, sender_identity) = Self::extract_sender_info(message);

        let mut identities = vec![username.as_str()];
        if let Some(id) = sender_id.as_deref() {
            identities.push(id);
        }

        if !self.is_any_user_allowed(identities.iter().copied()) {
            return None;
        }

        // Apply mention_only gate before downloading. Photo / document
        // updates carry no `text` field, so the text-only gate in
        // `parse_update_message` can never see them and they used to slip
        // through unconditionally. See #6229.
        let gated_caption =
            self.check_media_mention_gate(message, attachment.caption.as_deref())?;

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let reply_target = if let Some(ref tid) = thread_id {
            format!("{}:{}", chat_id, tid)
        } else {
            chat_id.clone()
        };

        // Ensure workspace directory is configured
        let workspace = self.workspace_dir.as_ref().or_else(|| {
            tracing::warn!("Cannot save attachment: workspace_dir not configured");
            None
        })?;

        let save_dir = workspace.join("telegram_files");
        if let Err(e) = tokio::fs::create_dir_all(&save_dir).await {
            tracing::warn!("Failed to create telegram_files directory: {e}");
            return None;
        }

        // Download file from Telegram
        let tg_file_path = match self.get_file_path(&attachment.file_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get attachment file path: {e}");
                return None;
            }
        };

        let file_data = match self.download_file(&tg_file_path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to download attachment: {e}");
                return None;
            }
        };

        // Determine local filename
        let local_filename = match &attachment.file_name {
            Some(name) => name.clone(),
            None => {
                // For photos, derive extension from Telegram file path
                let ext = tg_file_path.rsplit('.').next().unwrap_or("jpg");
                format!("photo_{chat_id}_{message_id}.{ext}")
            }
        };

        let local_path = save_dir.join(&local_filename);
        if let Err(e) = tokio::fs::write(&local_path, &file_data).await {
            tracing::warn!("Failed to save attachment to {}: {e}", local_path.display());
            return None;
        }

        // Build message content.
        // Photos with image extensions use [IMAGE:] marker so the multimodal
        // pipeline validates vision capability. Non-image files always get
        // [Document:] format regardless of Telegram's classification.
        let mut content = format_attachment_content(attachment.kind, &local_filename, &local_path);
        // `gated_caption` is the caption with any bot mention stripped when
        // `mention_only` applied; otherwise the raw caption (or None).
        if let Some(caption) = gated_caption.as_deref()
            && !caption.is_empty()
        {
            use std::fmt::Write;
            let _ = write!(content, "\n\n{caption}");
        }

        // Prepend reply context if replying to another message
        if let Some(quote) = self.extract_reply_context(message) {
            content = format!("{quote}\n\n{content}");
        }

        // Prepend forwarding attribution when the message was forwarded
        if let Some(attr) = Self::format_forward_attribution(message) {
            content = format!("{attr}{content}");
        }

        Some(ChannelMessage {
            id: format!("telegram_{chat_id}_{message_id}"),
            sender: sender_identity,
            reply_target,
            content,
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: thread_id,
            interruption_scope_id: None,
            attachments: vec![],
        })
    }

    /// Attempt to parse a Telegram update as a voice message and transcribe it.
    ///
    /// Returns `None` if the message is not a voice message, transcription is disabled,
    /// or the message exceeds duration limits.
    pub(crate) async fn try_parse_voice_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let config = self.transcription.as_ref()?;
        let manager = self.transcription_manager.as_deref()?;
        let message = update.get("message")?;

        let (file_id, duration) = Self::parse_voice_metadata(message)?;

        if duration > config.max_duration_secs {
            tracing::info!(
                "Skipping voice message: duration {duration}s exceeds limit {}s",
                config.max_duration_secs
            );
            return None;
        }

        let (username, sender_id, sender_identity) = Self::extract_sender_info(message);

        let mut identities = vec![username.as_str()];
        if let Some(id) = sender_id.as_deref() {
            identities.push(id);
        }

        if !self.is_any_user_allowed(identities.iter().copied()) {
            return None;
        }

        // Apply mention_only gate before downloading + transcribing. Voice
        // notes typically have no caption, so under `mention_only = true`
        // they are rejected here — the bot has no reliable way to know it
        // was mentioned without first transcribing, and we don't want to
        // pay that cost for messages that will likely be dropped. See #6229.
        // The transcription itself is discarded; we only care whether the
        // gate returns Some (allowed) vs None (rejected).
        let voice_caption = message.get("caption").and_then(serde_json::Value::as_str);
        self.check_media_mention_gate(message, voice_caption)?;

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let reply_target = if let Some(ref tid) = thread_id {
            format!("{}:{}", chat_id, tid)
        } else {
            chat_id.clone()
        };

        // Download and transcribe
        let file_path = match self.get_file_path(&file_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get voice file path: {e}");
                return None;
            }
        };

        let file_name = file_path
            .rsplit('/')
            .next()
            .unwrap_or("voice.ogg")
            .to_string();

        let audio_data = match self.download_file(&file_path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to download voice file: {e}");
                return None;
            }
        };

        let text = match manager.transcribe(&audio_data, &file_name).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Voice transcription failed: {e}");
                return None;
            }
        };

        if text.trim().is_empty() {
            tracing::info!("Voice transcription returned empty text, skipping");
            return None;
        }

        // Enter voice-chat mode so outgoing replies get a TTS voice note
        if let Ok(mut vc) = self.voice_chats.lock() {
            vc.insert(reply_target.clone());
        }

        // Cache transcription for reply-context lookups
        {
            let mut cache = self.voice_transcriptions.lock();
            if cache.len() >= 100 {
                cache.clear();
            }
            cache.insert(format!("{chat_id}:{message_id}"), text.clone());
        }

        let content = if let Some(quote) = self.extract_reply_context(message) {
            format!("{quote}\n\n[Voice] {text}")
        } else {
            format!("[Voice] {text}")
        };

        // Prepend forwarding attribution when the message was forwarded
        let content = if let Some(attr) = Self::format_forward_attribution(message) {
            format!("{attr}{content}")
        } else {
            content
        };

        Some(ChannelMessage {
            id: format!("telegram_{chat_id}_{message_id}"),
            sender: sender_identity,
            reply_target,
            content,
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: thread_id,
            interruption_scope_id: None,
            attachments: vec![],
        })
    }

    /// Extract sender username and display identity from a Telegram message object.
    pub(crate) fn extract_sender_info(
        message: &serde_json::Value,
    ) -> (String, Option<String>, String) {
        let username = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());
        let sender_identity = if username == "unknown" {
            sender_id.clone().unwrap_or_else(|| "unknown".to_string())
        } else {
            username.clone()
        };
        (username, sender_id, sender_identity)
    }

    /// Build a forwarding attribution prefix from Telegram forward fields.
    ///
    /// Returns `Some("[Forwarded from ...] ")` when the message is forwarded,
    /// `None` otherwise.
    pub(crate) fn format_forward_attribution(message: &serde_json::Value) -> Option<String> {
        if let Some(from_chat) = message.get("forward_from_chat") {
            // Forwarded from a channel or group
            let title = from_chat
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown channel");
            Some(format!("[Forwarded from channel: {title}] "))
        } else if let Some(from_user) = message.get("forward_from") {
            // Forwarded from a user (privacy allows identity)
            let label = from_user
                .get("username")
                .and_then(serde_json::Value::as_str)
                .map(|u| format!("@{u}"))
                .or_else(|| {
                    from_user
                        .get("first_name")
                        .and_then(serde_json::Value::as_str)
                        .map(String::from)
                })
                .unwrap_or_else(|| "unknown".to_string());
            Some(format!("[Forwarded from {label}] "))
        } else {
            // Forwarded from a user who hides their identity
            message
                .get("forward_sender_name")
                .and_then(serde_json::Value::as_str)
                .map(|name| format!("[Forwarded from {name}] "))
        }
    }

    /// Extract reply context from a Telegram `reply_to_message`, if present.
    pub(crate) fn extract_reply_context(&self, message: &serde_json::Value) -> Option<String> {
        let reply = message.get("reply_to_message")?;

        // Skip the auto-injected topic-root reference Telegram adds to every
        // message in a non-General forum topic. Its message_id equals the
        // parent message's message_thread_id. Treating it as a real reply
        // produces a spurious `> @user:\n> [Message]` blockquote prefix that
        // downstream reply-intent classification reads as "user is replying
        // to someone else" and rejects.
        let reply_mid = reply.get("message_id").and_then(serde_json::Value::as_i64);
        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64);
        if let (Some(rmid), Some(tid)) = (reply_mid, thread_id)
            && rmid == tid
        {
            return None;
        }

        let reply_sender = reply
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                reply
                    .get("from")
                    .and_then(|from| from.get("first_name"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("unknown");

        let reply_text = if let Some(text) = reply.get("text").and_then(serde_json::Value::as_str) {
            text.to_string()
        } else if reply.get("voice").is_some() || reply.get("audio").is_some() {
            let reply_mid = reply.get("message_id").and_then(serde_json::Value::as_i64);
            let chat_id = message
                .get("chat")
                .and_then(|c| c.get("id"))
                .and_then(serde_json::Value::as_i64);
            if let (Some(mid), Some(cid)) = (reply_mid, chat_id) {
                self.voice_transcriptions
                    .lock()
                    .get(&format!("{cid}:{mid}"))
                    .map(|t| format!("[Voice] {t}"))
                    .unwrap_or_else(|| "[Voice message]".to_string())
            } else {
                "[Voice message]".to_string()
            }
        } else if reply.get("photo").is_some() {
            "[Photo]".to_string()
        } else if reply.get("document").is_some() {
            "[Document]".to_string()
        } else if reply.get("video").is_some() {
            "[Video]".to_string()
        } else if reply.get("sticker").is_some() {
            "[Sticker]".to_string()
        } else {
            "[Message]".to_string()
        };

        // Format as blockquote with sender attribution
        let quoted_lines: String = reply_text
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        Some(format!("> @{reply_sender}:\n{quoted_lines}"))
    }

    pub(crate) fn parse_update_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let message = update.get("message")?;

        let text = message.get("text").and_then(serde_json::Value::as_str)?;

        let (username, sender_id, sender_identity) = Self::extract_sender_info(message);

        let mut identities = vec![username.as_str()];
        if let Some(id) = sender_id.as_deref() {
            identities.push(id);
        }

        if !self.is_any_user_allowed(identities.iter().copied()) {
            return None;
        }

        let is_group = Self::is_group_message(message);
        if self.mention_only && is_group {
            let bot_username = self.bot_username.lock();
            let bot_username = bot_username.as_deref()?;
            if !Self::contains_bot_mention(text, bot_username) {
                return None;
            }
        }

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        // Extract thread/topic ID for forum support
        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        // reply_target: chat_id or chat_id:thread_id format
        let reply_target = if let Some(ref tid) = thread_id {
            format!("{}:{}", chat_id, tid)
        } else {
            chat_id.clone()
        };

        let content = if self.mention_only && is_group {
            let bot_username = self.bot_username.lock();
            let bot_username = bot_username.as_ref()?;
            Self::normalize_incoming_content(text, bot_username)?
        } else {
            text.to_string()
        };

        let content = if let Some(quote) = self.extract_reply_context(message) {
            format!("{quote}\n\n{content}")
        } else {
            content
        };

        // Prepend forwarding attribution when the message was forwarded
        let content = if let Some(attr) = Self::format_forward_attribution(message) {
            format!("{attr}{content}")
        } else {
            content
        };

        // Exit voice-chat mode when user switches back to typing
        if let Ok(mut vc) = self.voice_chats.lock() {
            vc.remove(&reply_target);
        }

        Some(ChannelMessage {
            id: format!("telegram_{chat_id}_{message_id}"),
            sender: sender_identity,
            reply_target,
            content,
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: thread_id,
            interruption_scope_id: None,
            attachments: vec![],
        })
    }

    /// Download a Telegram photo by file_id, resize to fit within 1024px, and return as base64 data URI.
    #[allow(dead_code)] // WIP: will be used for photo attachment support
    pub(crate) async fn resolve_photo_data_uri(&self, file_id: &str) -> anyhow::Result<String> {
        use base64::Engine as _;

        // Step 1: call getFile to get file_path
        let get_file_url = self.api_url(&format!("getFile?file_id={}", file_id));
        let resp = self.http_client().get(&get_file_url).send().await?;
        let json: serde_json::Value = resp.json().await?;
        let file_path = json
            .get("result")
            .and_then(|r| r.get("file_path"))
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("getFile: no file_path in response"))?
            .to_string();

        // Step 2: download the actual file
        let download_url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token, file_path
        );
        let img_resp = self.http_client().get(&download_url).send().await?;
        let bytes = img_resp.bytes().await?;

        // Step 3: resize to max 1024px on longest side to fit within model context
        let resized_bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let img = image::load_from_memory(&bytes)?;
            let (w, h) = (img.width(), img.height());
            let max_dim = 512u32;
            let resized = if w > max_dim || h > max_dim {
                img.thumbnail(max_dim, max_dim)
            } else {
                img
            };
            let mut buf = Vec::new();
            resized.write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::Jpeg,
            )?;
            Ok(buf)
        })
        .await??;

        let b64 = base64::engine::general_purpose::STANDARD.encode(&resized_bytes);
        Ok(format!("data:image/jpeg;base64,{}", b64))
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Convert Markdown to Telegram HTML format.
    /// Telegram HTML supports: <b>, <i>, <u>, <s>, <code>, <pre>, <a href="...">
    /// This mirrors OpenClaw's markdownToTelegramHtml approach.
    pub(crate) fn markdown_to_telegram_html(text: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut result_lines: Vec<String> = Vec::new();

        for line in &lines {
            let trimmed_line = line.trim_start();
            if trimmed_line.starts_with("```") {
                // Preserve fence lines so the second-pass block parser can consume them
                // without interference from inline backtick handling.
                result_lines.push(trimmed_line.to_string());
                continue;
            }

            let mut line_out = String::new();

            // Handle code blocks (``` ... ```) - handled at text level below
            // Handle headers: ## Title → <b>Title</b>
            let stripped = line.trim_start_matches('#');
            let header_level = line.len() - stripped.len();
            if header_level > 0 && line.starts_with('#') && stripped.starts_with(' ') {
                let title = Self::escape_html(stripped.trim());
                result_lines.push(format!("<b>{title}</b>"));
                continue;
            }

            // Inline formatting
            let mut i = 0;
            let bytes = line.as_bytes();
            let len = bytes.len();
            while i < len {
                // Bold: **text** or __text__
                if i + 1 < len
                    && bytes[i] == b'*'
                    && bytes[i + 1] == b'*'
                    && let Some(end) = line[i + 2..].find("**")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<b>{inner}</b>");
                    i += 4 + end;
                    continue;
                }
                if i + 1 < len
                    && bytes[i] == b'_'
                    && bytes[i + 1] == b'_'
                    && let Some(end) = line[i + 2..].find("__")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<b>{inner}</b>");
                    i += 4 + end;
                    continue;
                }
                // Italic: *text* or _text_ (single)
                if bytes[i] == b'*'
                    && (i == 0 || bytes[i - 1] != b'*')
                    && let Some(end) = line[i + 1..].find('*')
                    && end > 0
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<i>{inner}</i>");
                    i += 2 + end;
                    continue;
                }
                // Inline code: `code`
                if bytes[i] == b'`'
                    && (i == 0 || bytes[i - 1] != b'`')
                    && let Some(end) = line[i + 1..].find('`')
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<code>{inner}</code>");
                    i += 2 + end;
                    continue;
                }
                // Markdown link: [text](url)
                if bytes[i] == b'['
                    && let Some(bracket_end) = line[i + 1..].find(']')
                {
                    let text_part = &line[i + 1..i + 1 + bracket_end];
                    let after_bracket = i + 1 + bracket_end + 1; // position after ']'
                    if after_bracket < len
                        && bytes[after_bracket] == b'('
                        && let Some(paren_end) = line[after_bracket + 1..].find(')')
                    {
                        let url = &line[after_bracket + 1..after_bracket + 1 + paren_end];
                        if url.starts_with("http://") || url.starts_with("https://") {
                            let text_html = Self::escape_html(text_part);
                            let url_html = Self::escape_html(url);
                            let _ = write!(line_out, "<a href=\"{url_html}\">{text_html}</a>");
                            i = after_bracket + 1 + paren_end + 1;
                            continue;
                        }
                    }
                }
                // Strikethrough: ~~text~~
                if i + 1 < len
                    && bytes[i] == b'~'
                    && bytes[i + 1] == b'~'
                    && let Some(end) = line[i + 2..].find("~~")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<s>{inner}</s>");
                    i += 4 + end;
                    continue;
                }
                // Default: escape HTML entities
                let ch = line[i..].chars().next().unwrap();
                match ch {
                    '<' => line_out.push_str("&lt;"),
                    '>' => line_out.push_str("&gt;"),
                    '&' => line_out.push_str("&amp;"),
                    '"' => line_out.push_str("&quot;"),
                    '\'' => line_out.push_str("&#39;"),
                    _ => line_out.push(ch),
                }
                i += ch.len_utf8();
            }
            result_lines.push(line_out);
        }

        // Second pass: handle ``` code blocks across lines
        let joined = result_lines.join("\n");
        let mut final_out = String::with_capacity(joined.len());
        let mut in_code_block = false;
        let mut code_buf = String::new();

        for line in joined.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_code_block {
                    in_code_block = false;
                    let escaped = code_buf.trim_end_matches('\n');
                    // Telegram HTML parse mode supports <pre> and <code>, but not class attributes.
                    let _ = writeln!(final_out, "<pre><code>{escaped}</code></pre>");
                    code_buf.clear();
                } else {
                    in_code_block = true;
                    code_buf.clear();
                }
            } else if in_code_block {
                code_buf.push_str(line);
                code_buf.push('\n');
            } else {
                final_out.push_str(line);
                final_out.push('\n');
            }
        }
        if in_code_block && !code_buf.is_empty() {
            let _ = writeln!(final_out, "<pre><code>{}</code></pre>", code_buf.trim_end());
        }

        final_out.trim_end_matches('\n').to_string()
    }

    pub(crate) fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    pub(crate) async fn send_text_chunks(
        &self,
        message: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let chunks = split_message_for_telegram(message);

        for (index, chunk) in chunks.iter().enumerate() {
            let text = if chunks.len() > 1 {
                if index == 0 {
                    format!("{chunk}\n\n(continues...)")
                } else if index == chunks.len() - 1 {
                    format!("(continued)\n\n{chunk}")
                } else {
                    format!("(continued)\n\n{chunk}\n\n(continues...)")
                }
            } else {
                chunk.to_string()
            };

            let mut markdown_body = serde_json::json!({
                "chat_id": chat_id,
                "text": Self::markdown_to_telegram_html(&text),
                "parse_mode": "HTML"
            });

            // Add message_thread_id for forum topic support
            if let Some(tid) = thread_id {
                markdown_body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }
            if let Some(lp) = self.link_preview_json() {
                markdown_body["link_preview_options"] = lp;
            }

            let markdown_resp = self
                .http_client()
                .post(self.api_url("sendMessage"))
                .json(&markdown_body)
                .send()
                .await?;

            if markdown_resp.status().is_success() {
                if index < chunks.len() - 1 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }

            let markdown_status = markdown_resp.status();
            let markdown_err = markdown_resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = ?markdown_status,
                "Telegram sendMessage with Markdown failed; retrying without parse_mode"
            );

            let mut plain_body = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            });

            // Add message_thread_id for forum topic support
            if let Some(tid) = thread_id {
                plain_body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }
            if let Some(lp) = self.link_preview_json() {
                plain_body["link_preview_options"] = lp;
            }
            let plain_resp = self
                .http_client()
                .post(self.api_url("sendMessage"))
                .json(&plain_body)
                .send()
                .await?;

            if !plain_resp.status().is_success() {
                let plain_status = plain_resp.status();
                let plain_err = plain_resp.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Telegram sendMessage failed (markdown {}: {}; plain {}: {})",
                    markdown_status,
                    markdown_err,
                    plain_status,
                    plain_err
                );
            }

            if index < chunks.len() - 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    pub(crate) async fn send_media_by_url(
        &self,
        method: &str,
        media_field: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
        });
        body[media_field] = serde_json::Value::String(url.to_string());

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url(method))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram {method} by URL failed: {err}");
        }

        tracing::info!("Telegram {method} sent to {chat_id}: {url}");
        Ok(())
    }

    pub(crate) async fn send_attachment(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        attachment: &TelegramAttachment,
    ) -> anyhow::Result<()> {
        let target = attachment.target.trim();

        if is_http_url(target) {
            let result = match attachment.kind {
                TelegramAttachmentKind::Image => {
                    self.send_photo_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Document => {
                    self.send_document_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Video => {
                    self.send_video_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Audio => {
                    self.send_audio_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Voice => {
                    self.send_voice_by_url(chat_id, thread_id, target, None)
                        .await
                }
            };

            // If sending media by URL failed (e.g. Telegram can't fetch the URL,
            // wrong content type, etc.), fall back to sending the URL as a text link
            // instead of losing the reply entirely.
            if let Err(e) = result {
                tracing::warn!(
                    url = target,
                    error = %e,
                    "Telegram send media by URL failed; falling back to text link"
                );
                let kind_label = match attachment.kind {
                    TelegramAttachmentKind::Image => "Image",
                    TelegramAttachmentKind::Document => "Document",
                    TelegramAttachmentKind::Video => "Video",
                    TelegramAttachmentKind::Audio => "Audio",
                    TelegramAttachmentKind::Voice => "Voice",
                };
                let fallback_text = format!("{kind_label}: {target}");
                self.send_text_chunks(&fallback_text, chat_id, thread_id)
                    .await?;
            }

            return Ok(());
        }

        // Remap Docker container workspace path (/workspace/...) to the host
        // workspace directory so files written by the containerised runtime
        // can be found and sent by the host-side Telegram sender.
        let remapped;
        let target = if let Some(rel) = target.strip_prefix("/workspace/") {
            if let Some(ws) = &self.workspace_dir {
                remapped = ws.join(rel);
                remapped.to_str().unwrap_or(target)
            } else {
                target
            }
        } else {
            target
        };

        let path = Path::new(target);
        if !path.exists() {
            anyhow::bail!("Telegram attachment path not found: {target}");
        }

        match attachment.kind {
            TelegramAttachmentKind::Image => self.send_photo(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Document => {
                self.send_document(chat_id, thread_id, path, None).await
            }
            TelegramAttachmentKind::Video => self.send_video(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Audio => self.send_audio(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Voice => self.send_voice(chat_id, thread_id, path, None).await,
        }
    }

    /// Send a document/file to a Telegram chat
    pub async fn send_document(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument failed: {err}");
        }

        tracing::info!("Telegram document sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a document from bytes (in-memory) to a Telegram chat
    pub async fn send_document_bytes(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument failed: {err}");
        }

        tracing::info!("Telegram document sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a photo to a Telegram chat
    pub async fn send_photo(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("photo.jpg");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto failed: {err}");
        }

        tracing::info!("Telegram photo sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a photo from bytes (in-memory) to a Telegram chat
    pub async fn send_photo_bytes(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto failed: {err}");
        }

        tracing::info!("Telegram photo sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a video to a Telegram chat
    pub async fn send_video(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("video.mp4");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("video", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendVideo"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendVideo failed: {err}");
        }

        tracing::info!("Telegram video sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send an audio file to a Telegram chat
    pub async fn send_audio(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.mp3");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("audio", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendAudio"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendAudio failed: {err}");
        }

        tracing::info!("Telegram audio sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a voice message to a Telegram chat
    pub async fn send_voice(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("voice.ogg");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendVoice"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendVoice failed: {err}");
        }

        tracing::info!("Telegram voice sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a file by URL (Telegram will download it)
    pub async fn send_document_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "document": url
        });

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument by URL failed: {err}");
        }

        tracing::info!("Telegram document (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    /// Send a photo by URL (Telegram will download it)
    pub async fn send_photo_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": url
        });

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto by URL failed: {err}");
        }

        tracing::info!("Telegram photo (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    /// Send a video by URL (Telegram will download it)
    pub async fn send_video_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendVideo", "video", chat_id, thread_id, url, caption)
            .await
    }

    /// Send an audio file by URL (Telegram will download it)
    pub async fn send_audio_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendAudio", "audio", chat_id, thread_id, url, caption)
            .await
    }

    /// Send a voice message by URL (Telegram will download it)
    pub async fn send_voice_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendVoice", "voice", chat_id, thread_id, url, caption)
            .await
    }

    /// Spawn the T4 watchdog loops (heartbeat getMe probe + pending-update
    /// probe). They own clones of the recovery state and the API connection
    /// parameters, so they can run independently of `listen()`.
    pub(crate) fn spawn_poll_watchdogs(&self) {
        let recovery = self.poll_recovery.clone();
        let api_base = self.api_base.clone();
        let token = self.bot_token.clone();
        let proxy = self.proxy_url.clone();
        let fallbacks = self.fallback_ips.clone();
        let index = Arc::clone(&self.fallback_ip_index);
        let heartbeat = tokio::spawn(async move {
            poll_heartbeat_loop(
                &recovery,
                &api_base,
                &token,
                proxy.as_deref(),
                &fallbacks,
                &index,
            )
            .await;
        });

        let recovery = self.poll_recovery.clone();
        let api_base = self.api_base.clone();
        let token = self.bot_token.clone();
        let proxy = self.proxy_url.clone();
        let fallbacks = self.fallback_ips.clone();
        let index = Arc::clone(&self.fallback_ip_index);
        let pending_probe = tokio::spawn(async move {
            probe_pending_updates_loop(
                &recovery,
                &api_base,
                &token,
                proxy.as_deref(),
                &fallbacks,
                &index,
            )
            .await;
        });

        self.poll_watchdogs
            .lock()
            .extend([heartbeat, pending_probe]);
    }

    /// Abort the spawned watchdog loops. Called when `listen()` returns so a
    /// dead channel does not keep probing the API; also wired into `Drop` for
    /// externally-aborted tasks.
    pub(crate) fn abort_poll_watchdogs(&self) {
        let mut handles = self.poll_watchdogs.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    /// Claim the getUpdates slot (startup) or re-verify it after a recovery
    /// trigger / transient failure (hermes `_verify_polling_after_reconnect`
    /// parity). Uses a `timeout=0` request so the slot is confirmed without
    /// entering a long-poll. Drains any queued updates into `offset`.
    ///
    /// `Fatal` errors (401/403) stop the channel; `Conflict` backs off past
    /// Telegram's 30-second poll window; transient network errors retry with a
    /// short backoff. Returns `Err` only for fatal errors.
    pub(crate) async fn verify_polling_slot(&self, offset: &mut i64) -> anyhow::Result<()> {
        loop {
            let url = self.api_url("getUpdates");
            let probe = serde_json::json!({
                "offset": *offset,
                "timeout": 0,
                "allowed_updates": ["message", "callback_query"]
            });
            match self.http_client().post(&url).json(&probe).send().await {
                Err(e) => {
                    tracing::warn!("Telegram slot probe network error: {e}; retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Err(e) => {
                        tracing::warn!("Telegram slot probe parse error: {e}; retrying in 5s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    Ok(data) => {
                        let ok = data
                            .get("ok")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if ok {
                            // Slot claimed — advance offset past any queued updates.
                            if let Some(results) =
                                data.get("result").and_then(serde_json::Value::as_array)
                            {
                                for update in results {
                                    if let Some(uid) =
                                        update.get("update_id").and_then(serde_json::Value::as_i64)
                                    {
                                        *offset = uid + 1;
                                    }
                                }
                            }
                            return Ok(());
                        }

                        let error_code = data
                            .get("error_code")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or_default();
                        let description = data
                            .get("description")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown");
                        match PollErrorClass::from_error_code(error_code) {
                            PollErrorClass::Fatal => {
                                tracing::error!(
                                    "Telegram polling slot probe fatal error \
({error_code}): {description}"
                                );
                                anyhow::bail!(
                                    "telegram polling fatal error {error_code}: {description}"
                                );
                            }
                            PollErrorClass::Conflict => {
                                tracing::debug!(
                                    "Telegram slot busy (409): {description}; backing off 35s"
                                );
                                tokio::time::sleep(Duration::from_secs(35)).await;
                            }
                            PollErrorClass::Network => {
                                tracing::warn!(
                                    "Telegram slot probe API error {error_code}: {description}; \
retrying in 5s"
                                );
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                },
            }
        }
    }
}
