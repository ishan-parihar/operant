//! Gateway Runner
//!
//! Manages a singleton Gateway instance that persists across CLI commands
//! within a single process. Provides start/stop/restart lifecycle management
//! backed by configuration and the `operant_core::gateway` module.

use anyhow::{Context, Result};
use base64::Engine;
use operant_core::agent::{AgentEvent, OperantAgent};
use operant_core::config::runtime_config;
use operant_core::config::AppConfig;
use operant_core::gateway::{
    DiscordAdapter, Gateway, GatewayConfig, IncomingMessage, MessageHandler, OutgoingMessage,
    PlatformAdapter, SlackAdapter, TelegramAdapter, WebhookAdapter,
};
use operant_core::gateway_pipeline::{MessagePipeline, PipelineAction};
use operant_core::memory_provider::{build_memory_provider, MemoryProvider};

use crate::gateway_commands::{
    handle_command, resolve_command, telegram_bot_commands, CommandContext,
};
use operant_core::mcp::McpManager;
use operant_core::tools::{OperantTool, ToolContext, TranscriptionTool};
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Returns the path to the gateway PID file used for cross-process status checks.
fn pid_file_path() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("gateway.pid")
}
/// Message handler that processes incoming gateway messages through the Operant agent.
struct GatewayMessageHandler {
    agent: Arc<OperantAgent>,
    memory_provider: Arc<dyn MemoryProvider>,
}

#[async_trait::async_trait]
impl MessageHandler for GatewayMessageHandler {
    async fn handle(&self, message: IncomingMessage) -> operant_core::Result<OutgoingMessage> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Build session key: per-user for DMs, per-channel for groups
        let session_key = if message.is_group_chat {
            format!("{}:{}", message.platform, message.channel_id)
        } else {
            format!(
                "{}:{}:{}",
                message.platform, message.channel_id, message.user_id
            )
        };

        // Derive stable session_id from key hash
        let mut hasher = DefaultHasher::new();
        session_key.hash(&mut hasher);
        let session_id = format!("gw_{:x}", hasher.finish());

        // Ensure session exists in DB
        let now = chrono::Utc::now().to_rfc3339();
        let _ = self
            .agent
            .db()
            .save_session(&session_id, None, "gateway", &now, &now);

        // Load conversation history (last 20 messages) instead of clearing
        self.agent.clear_history().await;
        if let Ok(history) = self.agent.db().get_session_messages(&session_id) {
            let skip = history.len().saturating_sub(20);
            for msg in history.into_iter().skip(skip) {
                let m = match msg.role.as_str() {
                    "user" => operant_core::client::Message::user(msg.content),
                    "assistant" => operant_core::client::Message::assistant(msg.content),
                    _ => continue,
                };
                self.agent.add_message(m).await;
            }
        }

        // Prefetch relevant memories from the configured provider
        let memory_context = self.memory_provider.prefetch(&message.content).await;
        let query = if memory_context.is_empty() {
            message.content.clone()
        } else {
            format!(
                "[retrieved_memory]\n{}\n\n[/retrieved_memory]\n\n{}",
                memory_context, message.content
            )
        };

        let user_content = message.content.clone();

        match self.agent.run(query).await {
            Ok(response) => {
                let content = if response.content.trim().is_empty() {
                    if let Some(ref reasoning) = response.reasoning {
                        if !reasoning.trim().is_empty() {
                            tracing::info!(
                                "Content empty but reasoning available, using as fallback (len={})",
                                reasoning.len()
                            );
                            reasoning.clone()
                        } else {
                            tracing::warn!("Agent returned empty response, no reasoning available");
                            format!("I've completed the tool calls you requested.")
                        }
                    } else {
                        tracing::warn!("Agent returned empty response, no reasoning available");
                        format!("I've completed the tool calls you requested.")
                    }
                } else {
                    response.content
                };

                // Sync this turn to the memory provider
                let _ = self
                    .memory_provider
                    .sync_turn(&user_content, &content)
                    .await;

                // Save user message and assistant response to DB
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "user", &user_content, &now);
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "assistant", &content, &now);

                Ok(OutgoingMessage::new(message.channel_id, content))
            }
            Err(e) => {
                // Still save the user message on error
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "user", &user_content, &now);
                Ok(OutgoingMessage::new(
                    &message.channel_id,
                    format!("Error: {}", e),
                ))
            }
        }
    }
}

/// Global gateway instance managed by the runner
static RUNNER: OnceLock<Mutex<Option<Arc<Gateway>>>> = OnceLock::new();

fn runner() -> &'static Mutex<Option<Arc<Gateway>>> {
    RUNNER.get_or_init(|| Mutex::new(None))
}

/// Build platform adapters from config
fn build_adapters(config: &GatewayConfig) -> Vec<Arc<dyn PlatformAdapter>> {
    let mut adapters: Vec<Arc<dyn PlatformAdapter>> = Vec::new();

    // Telegram
    if config.telegram_enabled {
        adapters.push(Arc::new(TelegramAdapter::with_config(
            config.telegram_token.clone(),
            config.telegram_bot_username.clone(),
            config.telegram_dm_topics_enabled,
            config.telegram_proxy.as_deref(),
        )));
    }

    // Discord
    if config.discord_enabled {
        adapters.push(Arc::new(DiscordAdapter::new(config.discord_token.clone())));
    }

    // Slack
    if config.slack_enabled {
        adapters.push(Arc::new(SlackAdapter::new(
            config.slack_token.clone(),
            None,
        )));
    }

    // Webhook adapter
    if config.webhooks_enabled {
        adapters.push(Arc::new(WebhookAdapter::new(config.webhooks_enabled)));
    }

    adapters
}

fn build_session_context(platform: &str, channel_id: &str, app_config: &AppConfig) -> String {
    let mut ctx = String::new();
    ctx.push_str(&format!("Connected platform: {}\n", platform));
    ctx.push_str(&format!(
        "Channel: {}\n",
        operant_core::pii::redact_chat_id(channel_id)
    ));
    if app_config.gateway.telegram_enabled {
        ctx.push_str("Available: Telegram\n");
    }
    if app_config.gateway.discord_enabled {
        ctx.push_str("Available: Discord\n");
    }
    if app_config.gateway.slack_enabled {
        ctx.push_str("Available: Slack\n");
    }
    ctx
}

/// Start the gateway with the provided application configuration.
///
/// Constructs a `GatewayConfig` from `app_config.gateway.*` fields,
/// builds the enabled platform adapters, creates the gateway session
/// handler, and starts polling for incoming messages.
pub async fn start_gateway(app_config: &AppConfig) -> Result<String> {
    let mut guard = runner().lock().await;

    if guard.is_some() {
        if let Some(gw) = guard.as_ref() {
            if gw.is_running().await {
                return Ok("Gateway is already running.".to_string());
            }
        }
    }

    let gw_config = GatewayConfig {
        telegram_enabled: app_config.gateway.telegram_enabled,
        telegram_token: app_config.gateway.telegram_token.clone(),
        discord_enabled: app_config.gateway.discord_enabled,
        discord_token: app_config.gateway.discord_token.clone(),
        slack_enabled: app_config.gateway.slack_enabled,
        slack_token: app_config.gateway.slack_token.clone(),
        webhooks_enabled: app_config.gateway.webhooks_enabled,
        webhooks_addr: app_config.gateway.webhooks_addr.clone(),
        admins: app_config.gateway.admins.clone(),
        streaming_transport: app_config.gateway.streaming_transport.clone(),
        telegram_proxy: app_config.gateway.telegram_proxy.clone(),
        telegram_bot_username: app_config.gateway.telegram_bot_username.clone(),
        telegram_dm_topics_enabled: app_config.gateway.telegram_dm_topics_enabled,
    };
    let dispatch_config = gw_config.clone();
    let adapters = build_adapters(&gw_config);

    let mut gateway = Gateway::new(gw_config);
    for adapter in adapters {
        gateway = gateway.with_adapter(adapter);
    }

    // Attach persistent session store for cross-restart session tracking
    let session_db_path = operant_core::platform::operant_home().join("database.db");
    if let Ok(store) = operant_core::PersistentSessionStore::open(session_db_path.to_str().unwrap())
    {
        gateway = gateway.with_persistent_sessions(std::sync::Arc::new(store));
    }

    let mcp_manager = McpManager::new();

    // Create event channel for tool progress previews
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let current_channel: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));

    let agent = crate::create_runtime_agent(
        app_config,
        &app_config.agent,
        None,
        event_tx,
        &mcp_manager,
        &app_config.skills.root_dir,
    )
    .await?;
    let agent = Arc::new(agent);
    let cron_agent = agent.clone();

    // Initialize memory provider from config
    let mem_cfg = runtime_config().memory;
    let storage_dir = operant_core::platform::operant_home();
    let memory_provider =
        if mem_cfg.enabled && mem_cfg.provider != "builtin" && mem_cfg.provider != "disabled" {
            build_memory_provider(&mem_cfg.provider, storage_dir)
        } else {
            build_memory_provider("builtin", storage_dir)
        };

    let handler = Arc::new(GatewayMessageHandler {
        agent,
        memory_provider,
    });
    gateway = gateway.with_handler(handler);

    let gateway = Arc::new(gateway);

    // Check for interrupted turns from previous session
    check_interrupted_turns();
    gateway.start().await.context("Failed to start gateway")?;

    let (message_tx, mut message_rx) = mpsc::unbounded_channel::<IncomingMessage>();
    gateway
        .start_with_channel(message_tx)
        .await
        .context("Failed to start gateway channel")?;

    let gw = gateway.clone();
    let admins = dispatch_config.admins.clone();
    let telegram_token = dispatch_config.telegram_token.clone();

    if let Some(ref token) = telegram_token {
        let api_base = "https://api.telegram.org";
        let cmd_url = format!("{}/bot{}/setMyCommands", api_base, token);
        let cmd_body = format!(r#"{{"commands":{}}}"#, telegram_bot_commands());
        let client = reqwest::Client::new();
        match client
            .post(&cmd_url)
            .header("Content-Type", "application/json")
            .body(cmd_body)
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    tracing::info!("Telegram bot commands registered successfully");
                } else {
                    tracing::warn!(
                        "Failed to register Telegram commands: HTTP {}",
                        resp.status()
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Failed to register Telegram commands: {}", e);
            }
        }
    }

    // Tool preview helpers
    use std::collections::HashMap as Hm;

    fn tool_emoji(name: &str) -> &'static str {
        match name {
            "terminal" | "bash" | "shell" => "\u{1F4BB}",
            "web_search" | "web_search_" | "tavily" | "tavily_search" => "\u{1F50D}",
            "web_fetch" | "web_scrape" | "tavily_extract" | "tavily_crawl" => "\u{1F310}",
            "read" | "glob" | "grep" | "ast_grep" | "look_at" => "\u{1F4D6}",
            "write" | "edit" | "create" => "\u{270F}\u{FE0F}",
            "memory" | "memory_search" | "memory_store" => "\u{1F9E0}",
            "github" | "gh" | "git" => "\u{1F5A5}\u{FE0F}",
            "think" | "reason" | "sequentialthinking" => "\u{1F4AD}",
            "plan" | "strategy" => "\u{1F9F0}",
            "sql" | "database" | "db" | "postgres" => "\u{1F4BE}",
            "image" | "screenshot" | "draw" => "\u{1F5BC}\u{FE0F}",
            "chat" | "send_message" | "message" | "notify" => "\u{1F4AC}",
            "error" | "fail" => "\u{274C}",
            "done" | "complete" | "finish" => "\u{2705}",
            "search" | "find" => "\u{1F50E}",
            _ => "\u{2699}\u{FE0F}",
        }
    }

    fn extract_tool_arg(name: &str, args: &str) -> Option<String> {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(args) {
            let keys: &[&str] = match name {
                "terminal" | "bash" | "shell" => &["command"],
                "web_search" | "web_search_" | "tavily" | "tavily_search" => &["query"],
                "tavily_extract" | "web_fetch" | "tavily_crawl" => &["url"],
                "read" | "look_at" => &["file_path", "path", "pattern"],
                "glob" => &["pattern", "file_path"],
                "grep" | "search" | "ast_grep" => &["pattern", "query", "file_path"],
                "write" | "create" => &["file_path", "path", "content"],
                "edit" => &["file_path", "path", "old_string"],
                "memory" | "memory_search" => &["query", "text"],
                "memory_store" => &["content", "text"],
                "image" | "screenshot" => &["path", "url"],
                "chat" | "send_message" | "notify" => &["message", "content", "text"],
                "github" | "gh" => &["query", "command", "repo"],
                "think" | "sequentialthinking" | "reason" => &["thought"],
                "sql" | "database" | "postgres" => &["query", "sql"],
                _ => &[
                    "query",
                    "command",
                    "text",
                    "url",
                    "path",
                    "file_path",
                    "input",
                    "content",
                    "message",
                ],
            };
            for key in keys {
                if let Some(val) = json.get(*key).and_then(|v| v.as_str()) {
                    let truncated = if val.len() > 100 {
                        format!("{}...", &val[..100])
                    } else {
                        val.to_string()
                    };
                    return Some(truncated.replace('\n', "\\n"));
                }
            }
            // Fallback: show first string value from any key
            if let Some(obj) = json.as_object() {
                for (_key, val) in obj.iter() {
                    if let Some(s) = val.as_str() {
                        let truncated = if s.len() > 100 {
                            format!("{}...", &s[..100])
                        } else {
                            s.to_string()
                        };
                        return Some(truncated.replace('\n', "\\n"));
                    }
                }
            }
        }
        None
    }

    fn tool_preview_line(name: &str, args: &str) -> String {
        let emoji = tool_emoji(name);
        match extract_tool_arg(name, args) {
            Some(arg) => format!("{} {}: {}", emoji, name, arg),
            None => format!("{} {}...", emoji, name),
        }
    }

    // Spawn tool progress event receiver — appends tool calls to a single
    // chronological message by editing in-place. This gives users a clean
    // single-message timeline of all tool calls (like the operant-agent does).
    let gw_for_events = gw.clone();
    let current_channel_for_events = current_channel.clone();
    tokio::spawn(async move {
        // (platform, channel_id) -> (message_id, tool_lines)
        let mut progress_msgs: Hm<(String, String), (String, Vec<String>)> = Hm::new();
        tracing::info!("Tool progress event receiver started");
        while let Some(event) = event_rx.recv().await {
            let (platform, channel_id) = match current_channel_for_events.lock().await.as_ref() {
                Some((p, c)) => (p.clone(), c.clone()),
                None => {
                    tracing::debug!(
                        "Dropping tool event (no active channel): {:?}",
                        std::mem::discriminant(&event)
                    );
                    continue;
                }
            };

            match event {
                AgentEvent::ToolStart { name, arguments } => {
                    let line = tool_preview_line(&name, &arguments);
                    let key = (platform.clone(), channel_id.clone());

                    if let Some((msg_id, lines)) = progress_msgs.get_mut(&key) {
                        // Append line to existing chronological message
                        lines.push(line);
                        let body = lines.join("\n");
                        let msg = OutgoingMessage::new(&channel_id, &body).no_markdown();
                        let _ = gw_for_events
                            .edit_message(&platform, &channel_id, msg_id, msg)
                            .await;
                    } else {
                        // First tool — send new message
                        let body = line.clone();
                        let msg = OutgoingMessage::new(&channel_id, &body).no_markdown();
                        match gw_for_events.send_message_return_id(&platform, msg).await {
                            Ok(msg_id) => {
                                progress_msgs.insert(key, (msg_id, vec![line]));
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to send first progress message");
                            }
                        }
                    }
                }
                AgentEvent::ToolComplete { result } => {
                    // Intercept TTS audio results and send as voice message
                    if result.name == "text_to_speech" && result.success {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&result.content)
                        {
                            if let Some(audio_b64) = data.get("audio").and_then(|a| a.as_str()) {
                                let format =
                                    data.get("format").and_then(|f| f.as_str()).unwrap_or("wav");
                                if let Ok(audio_bytes) =
                                    base64::engine::general_purpose::STANDARD.decode(audio_b64)
                                {
                                    if let Err(e) = gw_for_events
                                        .send_voice(&platform, &channel_id, &audio_bytes, format)
                                        .await
                                    {
                                        tracing::warn!(error = %e, "Failed to send TTS voice message");
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        tracing::warn!("Tool progress event receiver exited (event_rx closed)");
    });

    let app_config_clone = app_config.clone();
    tokio::spawn(async move {
        while let Some(mut msg) = message_rx.recv().await {
            tracing::info!(
                "Gateway received message from {} on {}",
                msg.user_id,
                msg.platform
            );
            let platform = msg.platform.clone();
            let channel_id = msg.channel_id.clone();

            // Wrap entire processing body for error resilience — a single
            // message processing error will not crash the whole gateway.
            let result: anyhow::Result<()> = async {
                // ── 1. Admin allowlist ────────────────────────────────────────
                if !admins.is_empty() && !admins.contains(&msg.user_id) {
                    tracing::warn!(
                        "Message from unauthorized user {} on {} rejected",
                        msg.user_id,
                        msg.platform
                    );
                    let response = OutgoingMessage::new(
                        &msg.channel_id,
                        "You are not authorized to use this bot.",
                    );
                    gw.send_to_platform(&platform, response).await?;
                    return Ok(());
                }
                tracing::info!("Message from {} passed admin allowlist check", msg.user_id);

                // ── 1.5 Command interception ──────────────────────────────────
                let is_admin = admins.is_empty() || admins.contains(&msg.user_id);
                if let Some((cmd_def, cmd_args)) = resolve_command(&msg.content) {
                    tracing::info!("User {} ran command /{}", msg.user_id, cmd_def.name);
                    if cmd_def.admin_only && !is_admin {
                        let response =
                            OutgoingMessage::new(&msg.channel_id, "This command is admin-only.");
                        gw.send_to_platform(&platform, response).await?;
                    } else {
                        let ctx = CommandContext::new(
                            Some(&gw),
                            &app_config_clone,
                            is_admin,
                            &msg.user_id,
                            &platform,
                            &msg.channel_id,
                        );
                        if let Some(response_text) = handle_command(cmd_def.name, cmd_args, &ctx) {
                            let response = OutgoingMessage::new(&msg.channel_id, response_text);
                            gw.send_to_platform(&platform, response).await?;
                        }
                    }
                    return Ok(());
                }

                // ── 2. Message enrichment ─────────────────────────────────────
                if platform == "telegram" {
                    if let Some(token) = &telegram_token {
                        // Photo enrichment: download largest photo and note it
                        if let Some(desc) = enrich_photo(&msg.raw, token).await {
                            msg.content = format!("{}\n{}", desc, msg.content);
                        }
                        // Voice enrichment: download, transcribe, prepend
                        if let Some(transcript) = enrich_voice(&msg.raw, token).await {
                            msg.content = format!("{}\n{}", transcript, msg.content);
                        }
                        // Document enrichment: extract filename + caption
                        if let Some(info) = enrich_document(&msg.raw) {
                            msg.content = if msg.content.is_empty() {
                                info
                            } else {
                                format!("{}\n{}", msg.content, info)
                            };
                        }
                    }
                }
                tracing::info!(
                    "Message enrichment done, final content length: {}",
                    msg.content.len()
                );

                // ── 3. Message pipeline ───────────────────────────────────────
                let pipeline = MessagePipeline::new();
                let action = pipeline.process(&msg);
                tracing::info!("Message pipeline action: {:?}", action);
                match action {
                    PipelineAction::Block(reason) => {
                        let response = OutgoingMessage::new(&msg.channel_id, reason);
                        gw.send_to_platform(&platform, response).await?;
                        return Ok(());
                    }
                    PipelineAction::Queue => {
                        // Queue not yet wired — allow through for now.
                    }
                    PipelineAction::Allow => {}
                }

                // ── 4. Session management ─────────────────────────────────────
                if msg.is_group_chat {
                    match gw
                        .get_session_store()
                        .find_or_create_shared_session(&msg.platform, &msg.channel_id)
                    {
                        Ok(s) => tracing::info!(
                            "Shared session {} for channel {}",
                            s.session_id,
                            msg.channel_id
                        ),
                        Err(e) => tracing::warn!("Failed to get shared session: {}", e),
                    }
                    msg.content = format!("[{}]: {}", msg.username, msg.content);
                } else {
                    let existing = gw.get_session_store().find_session(
                        &msg.platform,
                        &msg.user_id,
                        &msg.channel_id,
                    );
                    if let Some(s) = existing {
                        tracing::info!(
                            "Found existing session {} for {}@{}",
                            s.session_id,
                            msg.user_id,
                            msg.platform
                        );
                        let _ = gw.get_session_store().update_activity(&s.session_id);
                    } else {
                        match gw.get_session_store().create_session(
                            &msg.platform,
                            &msg.user_id,
                            &msg.channel_id,
                        ) {
                            Ok(s) => tracing::info!(
                                "Created new session {} for {}@{}",
                                s.session_id,
                                msg.user_id,
                                msg.platform
                            ),
                            Err(e) => tracing::warn!("Failed to create session: {}", e),
                        }
                    }
                }

                // ── 5. Typing indicator ───────────────────────────────────────
                let typing_handle = if platform == "telegram" {
                    let gw = gw.clone();
                    let ch = channel_id.clone();
                    Some(tokio::spawn(async move {
                        let mut interval = tokio::time::interval(std::time::Duration::from_secs(4));
                        // Skip first immediate tick, send first after 4s
                        interval.tick().await;
                        loop {
                            interval.tick().await;
                            if gw.send_typing("telegram", &ch).is_err() {
                                break;
                            }
                        }
                    }))
                } else {
                    None
                };

                // ── 5.5 Keepalive notification ─────────────────────────────────
                // Send "Still working..." periodically for long-running operations
                let keepalive_handle = if platform == "telegram" {
                    let gw = gw.clone();
                    let ch = channel_id.clone();
                    Some(tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        // Wait 3 minutes before first notification (matches Python behavior)
                        tokio::time::sleep(std::time::Duration::from_secs(180)).await;
                        loop {
                            let elapsed = start.elapsed().as_secs();
                            let minutes = elapsed / 60;
                            let body =
                                format!("\u{23F3} Still working... ({}m elapsed...)", minutes);
                            let msg = OutgoingMessage::new(&ch, &body).no_markdown();
                            if gw.send_to_platform("telegram", msg).await.is_err() {
                                break;
                            }
                            // Wait 3 minutes between notifications
                            tokio::time::sleep(std::time::Duration::from_secs(180)).await;
                        }
                    }))
                } else {
                    None
                };

                // ── 5.5 Set current channel for tool progress ────────────────
                tracing::debug!(
                    "Setting current_channel to {}@{} for tool progress",
                    platform,
                    channel_id
                );
                *current_channel.lock().await = Some((platform.clone(), channel_id.clone()));

                // ── 5.6 Per-turn .env reload for credential rotation ─────────
                operant_core::env_passthrough::reload_dotenv();

                // ── 5.7 Turn state: mark pending ─────────────────────────────
                save_turn_state(&channel_id, "pending");

                // ── 5.8 Session context injection ────────────────────────────
                let ctx = build_session_context(&platform, &channel_id, &app_config_clone);
                msg.content = format!("[context: {}]\n{}", ctx.trim(), msg.content);

                // ── 6. Route message ──────────────────────────────────────────
                match gw.route_message(msg).await {
                    Ok(Some(response)) => {
                        tracing::info!(
                            "Message routed successfully, response length: {}",
                            response.content.len()
                        );
                        if let Err(e) = gw.send_to_platform(&platform, response).await {
                            tracing::error!(
                                "Failed to send response on {} to {}: {}",
                                platform,
                                channel_id,
                                e
                            );
                        }
                        save_turn_state(&channel_id, "complete");
                    }
                    Ok(None) => {
                        save_turn_state(&channel_id, "complete");
                    }
                    Err(e) => {
                        tracing::error!("Failed to route message on {}: {}", platform, e);
                        save_turn_state(&channel_id, "failed");
                    }
                }

                if let Some(handle) = typing_handle {
                    handle.abort();
                }
                if let Some(handle) = keepalive_handle {
                    handle.abort();
                }

                Ok(())
            }
            .await;

            if let Err(e) = result {
                tracing::error!(
                    "Error processing message from {} on {}: {}",
                    channel_id,
                    platform,
                    e
                );
            }
        }
    });

    let platform_count = gateway.status().await.len();
    *guard = Some(gateway.clone());

    // ── Spawn Cron Scheduler ─────────────────────────────────────────────
    let cron_db_path = operant_core::platform::operant_home().join("operant_cron.db");
    if let Ok(cron_db) = operant_core::cronjobs::CronDb::init(cron_db_path) {
        let cron_db = Arc::new(cron_db);
        let (cron_tx, mut cron_rx) =
            tokio::sync::mpsc::unbounded_channel::<operant_core::cronjobs::CronDelivery>();

        let scheduler = operant_core::cronjobs::CronScheduler::new(cron_db, cron_agent.clone())
            .with_delivery(cron_tx);
        tokio::spawn(async move { scheduler.start().await });

        // Delivery receiver — sends cron results to platforms
        let gw_for_cron = gateway.clone();
        tokio::spawn(async move {
            while let Some(delivery) = cron_rx.recv().await {
                let msg = OutgoingMessage::new(&delivery.chat_id, &delivery.content);
                if let Err(e) = gw_for_cron.send_to_platform(&delivery.platform, msg).await {
                    tracing::warn!(error = %e, "Failed to deliver cron result");
                }
            }
        });
        tracing::info!("Cron scheduler started");
    }

    // Write PID file for cross-process status checks
    if let Ok(_pid) = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
    {
        // Use current PID as content (not timestamp)
        let pid_str = std::process::id().to_string();
        let pid_path = pid_file_path();
        if let Some(parent) = pid_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&pid_path, &pid_str);
    }

    Ok(format!(
        "Gateway started with {} platform(s).",
        platform_count
    ))
}

/// Stop the gateway
pub async fn stop_gateway() -> Result<String> {
    let mut guard = runner().lock().await;

    match guard.take() {
        Some(gateway) => {
            gateway.stop().await.context("Failed to stop gateway")?;
            let _ = std::fs::remove_file(pid_file_path());
            Ok("Gateway stopped.".to_string())
        }
        None => Ok("Gateway is not running.".to_string()),
    }
}

/// Restart the gateway
pub async fn restart_gateway(app_config: &AppConfig) -> Result<String> {
    stop_gateway().await?;
    start_gateway(app_config).await
}

/// Check if gateway is running
pub async fn is_running() -> bool {
    let guard = runner().lock().await;
    match guard.as_ref() {
        Some(gw) => gw.is_running().await,
        None => false,
    }
}

/// Get reference to running gateway (for other modules)
pub async fn get_gateway() -> Option<Arc<Gateway>> {
    let guard = runner().lock().await;
    guard.clone()
}

// ── Message enrichment helpers ──────────────────────────────────────────────

/// Download a file from Telegram by `file_id` to a temporary location.
async fn download_telegram_file(token: &str, file_id: &str) -> Result<std::path::PathBuf> {
    let base = runtime_config().gateway.telegram_api_base;
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::new();

    // Step 1: resolve file_id → file_path via getFile API
    let resp: serde_json::Value = client
        .post(format!("{}/bot{}/getFile", base, token))
        .json(&serde_json::json!({"file_id": file_id}))
        .send()
        .await
        .context("Telegram getFile request failed")?
        .json()
        .await
        .context("Telegram getFile response parse failed")?;

    let file_path = resp["result"]["file_path"]
        .as_str()
        .context("Telegram getFile returned no file_path")?
        .to_string();

    // Step 2: download the actual file bytes
    let download_url = format!("{}/file/bot{}/{}", base, token, file_path);
    let bytes = client
        .get(&download_url)
        .send()
        .await
        .context("Telegram file download request failed")?
        .bytes()
        .await
        .context("Telegram file download body read failed")?;

    // Step 3: persist to a temp file (cleaned up by the OS on next boot)
    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("dat");
    let temp_dir = std::env::temp_dir();
    let short_id: String = file_id.chars().take(12).collect();
    let temp_file = temp_dir.join(format!("operant_tg_{}.{}", short_id, ext));
    tokio::fs::write(&temp_file, &bytes)
        .await
        .context("Failed to write Telegram download to temp file")?;

    Ok(temp_file)
}

/// If `raw` contains a Telegram photo array, download the largest image and
/// return a description string.  Returns `None` when no photo is present.
async fn enrich_photo(raw: &serde_json::Value, token: &str) -> Option<String> {
    let message = raw.get("message")?;
    let photos = message.get("photo")?.as_array()?;
    if photos.is_empty() {
        return None;
    }

    // Pick the photo variant with the largest file_size
    let largest = photos
        .iter()
        .max_by_key(|p| p.get("file_size").and_then(|s| s.as_i64()).unwrap_or(0))?;

    let file_id = largest.get("file_id")?.as_str()?;
    let width = largest.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
    let height = largest.get("height").and_then(|h| h.as_i64()).unwrap_or(0);

    match download_telegram_file(token, file_id).await {
        Ok(path) => {
            tracing::info!(
                "Downloaded photo to {:?} for analysis ({}×{})",
                path,
                width,
                height
            );
            Some(format!("[Image: {}×{} pixels]", width, height))
        }
        Err(e) => {
            tracing::warn!("Failed to download Telegram photo: {}", e);
            // Still report the dimensions even when download fails
            Some(format!("[Image: {}×{} pixels]", width, height))
        }
    }
}

/// If `raw` contains a Telegram voice message, download and transcribe it.
/// Returns the transcription text or a fallback placeholder on failure.
async fn enrich_voice(raw: &serde_json::Value, token: &str) -> Option<String> {
    let message = raw.get("message")?;
    let voice = message.get("voice")?;
    let file_id = voice.get("file_id")?.as_str()?;
    let duration = voice.get("duration").and_then(|d| d.as_i64()).unwrap_or(0);
    let minutes = duration / 60;
    let seconds = duration % 60;

    // Download the OGG/audio file to a temporary location
    let path = match download_telegram_file(token, file_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to download voice message: {}", e);
            return Some(format!("[Voice message: {}:{:02}]", minutes, seconds));
        }
    };

    // Transcribe via the existing TranscriptionTool (reads from local path)
    let tool = TranscriptionTool::new();
    let args = serde_json::json!({
        "filePath": path.to_str().unwrap_or(""),
        "provider": "groq",
    });
    let result = tool.execute(args, ToolContext::default()).await;

    if result.success {
        if let Ok(data) = result.parse_content::<serde_json::Value>() {
            if let Some(transcript) = data.get("transcript").and_then(|v| v.as_str()) {
                let trimmed = transcript.trim();
                if !trimmed.is_empty() {
                    tracing::info!(
                        "Transcribed voice message ({}:{:02}): {}",
                        minutes,
                        seconds,
                        trimmed
                    );
                    return Some(format!("[Transcription: {}]", trimmed));
                }
            }
        }
    } else {
        tracing::warn!(
            "Voice transcription failed: {}",
            result.error.as_deref().unwrap_or("unknown error")
        );
    }

    Some(format!("[Voice message: {}:{:02}]", minutes, seconds))
}

/// If `raw` contains a Telegram document, return its filename / mime / caption.
fn enrich_document(raw: &serde_json::Value) -> Option<String> {
    let message = raw.get("message")?;
    let doc = message.get("document")?;

    let filename = doc
        .get("file_name")
        .and_then(|f| f.as_str())
        .unwrap_or("unknown");
    let mime = doc
        .get("mime_type")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    let size = doc.get("file_size").and_then(|s| s.as_i64()).unwrap_or(0);
    let size_kb = size as f64 / 1024.0;

    let caption = message.get("caption").and_then(|c| c.as_str());

    let info = if let Some(cap) = caption {
        format!(
            "[Document: {} ({:.1} KB, {}) with caption: {}]",
            filename, size_kb, mime, cap
        )
    } else {
        format!("[Document: {} ({:.1} KB, {})]", filename, size_kb, mime)
    };

    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::config::GatewaySettings;

    #[tokio::test]
    async fn test_build_adapters_all_disabled() {
        let config = GatewayConfig {
            telegram_enabled: false,
            telegram_token: None,
            discord_enabled: false,
            discord_token: None,
            slack_enabled: false,
            slack_token: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
        };
        let adapters = build_adapters(&config);
        assert_eq!(adapters.len(), 0);
    }

    #[tokio::test]
    async fn test_build_adapters_telegram_enabled() {
        let config = GatewayConfig {
            telegram_enabled: true,
            telegram_token: Some("test-token".to_string()),
            discord_enabled: false,
            discord_token: None,
            slack_enabled: false,
            slack_token: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
        };
        let adapters = build_adapters(&config);
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].name(), "telegram");
    }

    #[tokio::test]
    async fn test_build_adapters_multi_platform() {
        let config = GatewayConfig {
            telegram_enabled: true,
            telegram_token: Some("t-token".to_string()),
            discord_enabled: true,
            discord_token: Some("d-token".to_string()),
            slack_enabled: true,
            slack_token: Some("s-token".to_string()),
            webhooks_enabled: true,
            webhooks_addr: Some("0.0.0.0:9090".to_string()),
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
        };
        let adapters = build_adapters(&config);
        assert_eq!(adapters.len(), 4);
    }

    #[test]
    fn test_gateway_config_from_appconfig_fields() {
        let app_config = AppConfig {
            gateway: GatewaySettings {
                telegram_enabled: true,
                telegram_token: Some("tok".to_string()),
                discord_enabled: false,
                discord_token: None,
                slack_enabled: true,
                slack_token: Some("stok".to_string()),
                webhooks_enabled: false,
                webhooks_addr: None,
                admins: vec!["admin1".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };

        let gw_config = GatewayConfig {
            telegram_enabled: app_config.gateway.telegram_enabled,
            telegram_token: app_config.gateway.telegram_token.clone(),
            discord_enabled: app_config.gateway.discord_enabled,
            discord_token: app_config.gateway.discord_token.clone(),
            slack_enabled: app_config.gateway.slack_enabled,
            slack_token: app_config.gateway.slack_token.clone(),
            webhooks_enabled: app_config.gateway.webhooks_enabled,
            webhooks_addr: app_config.gateway.webhooks_addr.clone(),
            admins: app_config.gateway.admins.clone(),
            streaming_transport: "auto".to_string(),
            telegram_proxy: app_config.gateway.telegram_proxy.clone(),
            telegram_bot_username: app_config.gateway.telegram_bot_username.clone(),
            telegram_dm_topics_enabled: app_config.gateway.telegram_dm_topics_enabled,
        };

        assert!(gw_config.telegram_enabled);
        assert!(!gw_config.discord_enabled);
        assert!(gw_config.slack_enabled);
        assert!(!gw_config.webhooks_enabled);
        assert_eq!(gw_config.admins, vec!["admin1"]);
    }

    #[tokio::test]
    async fn test_gateway_start_stop_with_disabled_platforms() {
        stop_gateway().await.ok();

        let tmp = tempfile::tempdir().unwrap();
        let _guard = operant_core::profile::set_operant_home_override(tmp.path().to_path_buf());

        // Ensure a fresh DB exists with the full schema before gateway touches it
        let db_path = tmp.path().join("database.db");
        let _db = operant_core::database::Database::init(db_path.clone()).expect("init test db");

        let config = AppConfig {
            database_path: db_path,
            gateway: GatewaySettings {
                telegram_enabled: false,
                discord_enabled: false,
                slack_enabled: false,
                webhooks_enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let msg = start_gateway(&config).await.unwrap();
        assert!(msg.contains("0 platform(s)"));

        let running = is_running().await;
        assert!(running);

        let stop_msg = stop_gateway().await.unwrap();
        assert!(stop_msg.contains("stopped"));

        let not_running = is_running().await;
        assert!(!not_running);
    }
}

// ── Turn state tracking for auto-continue / interruption recovery ────────

/// Persist turn state so interrupted sessions can be detected on restart.
///
/// Writes to `<operant_home>/.turn_state/<channel_id>.json` — one file per
/// channel — so concurrent turns on different channels don't overwrite each
/// other. Previously this wrote a single `.turn_state.json` that got
/// clobbered by whichever channel wrote last, so a crash during channel A's
/// turn would be invisible if channel B completed its turn before the
/// restart.
fn save_turn_state(channel_id: &str, status: &str) {
    save_turn_state_in(&operant_core::platform::operant_home(), channel_id, status);
}

/// Same as `save_turn_state` but writes to a caller-provided base directory.
/// Exposed for tests so they don't pollute the real operant home.
fn save_turn_state_in(base_dir: &std::path::Path, channel_id: &str, status: &str) {
    let dir = base_dir.join(".turn_state");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", sanitize_channel_id(channel_id)));
    let ts = chrono::Utc::now().to_rfc3339();
    let json = serde_json::json!({
        "channel_id": channel_id,
        "status": status,
        "timestamp": ts,
    });
    let _ = std::fs::write(path, json.to_string());
}

/// Check for interrupted turns on startup and log a warning for each.
///
/// Scans every file under `<operant_home>/.turn_state/` so all concurrent
/// channels are reported, not just whichever one happened to win the
/// last-write race on the old single-file scheme.
pub fn check_interrupted_turns() {
    check_interrupted_turns_in(&operant_core::platform::operant_home());
}

/// Same as `check_interrupted_turns` but reads from a caller-provided base
/// directory. Exposed for tests.
fn check_interrupted_turns_in(base_dir: &std::path::Path) -> Vec<(String, String)> {
    let dir = base_dir.join(".turn_state");
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return found,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let state = match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if state.get("status").and_then(|s| s.as_str()) == Some("pending") {
            let channel_id = state["channel_id"].as_str().unwrap_or("").to_string();
            let timestamp = state["timestamp"].as_str().unwrap_or("").to_string();
            tracing::warn!(
                channel_id = %channel_id,
                timestamp = %timestamp,
                "Detected interrupted turn from previous session"
            );
            found.push((channel_id, timestamp));
        }
    }
    found
}

/// Sanitize a channel ID for use as a filename. Channel IDs are typically
/// alphanumeric (Slack channel IDs, Discord channel IDs, etc.) but we
/// defensively strip anything that isn't alphanumeric, dash, or underscore
/// to prevent path traversal if a platform ever sends a hostile ID.
fn sanitize_channel_id(channel_id: &str) -> String {
    channel_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod turn_state_tests {
    use super::*;

    #[test]
    fn save_turn_state_writes_one_file_per_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "channel_a", "pending");
        save_turn_state_in(base, "channel_b", "pending");
        save_turn_state_in(base, "channel_a", "complete");

        let dir = base.join(".turn_state");
        let a_path = dir.join("channel_a.json");
        let b_path = dir.join("channel_b.json");
        assert!(a_path.exists(), "channel_a file should exist");
        assert!(b_path.exists(), "channel_b file should exist");

        // channel_a was written pending then complete — final state should
        // be "complete".
        let a_content = std::fs::read_to_string(&a_path).unwrap();
        let a_json: serde_json::Value = serde_json::from_str(&a_content).unwrap();
        assert_eq!(a_json["status"], "complete");
        assert_eq!(a_json["channel_id"], "channel_a");

        // channel_b was only written pending.
        let b_content = std::fs::read_to_string(&b_path).unwrap();
        let b_json: serde_json::Value = serde_json::from_str(&b_content).unwrap();
        assert_eq!(b_json["status"], "pending");
        assert_eq!(b_json["channel_id"], "channel_b");
    }

    #[test]
    fn concurrent_channels_do_not_overwrite_each_other() {
        // This is the core regression test for the old single-file bug:
        // two channels with concurrent pending turns must both be visible
        // to check_interrupted_turns_in, not just whichever one wrote last.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "slack_channel_1", "pending");
        save_turn_state_in(base, "discord_channel_2", "pending");

        let interrupted = check_interrupted_turns_in(base);
        assert_eq!(
            interrupted.len(),
            2,
            "both channels should be reported as interrupted, got {:?}",
            interrupted
        );

        let channel_ids: Vec<&str> = interrupted.iter().map(|(id, _)| id.as_str()).collect();
        assert!(channel_ids.contains(&"slack_channel_1"));
        assert!(channel_ids.contains(&"discord_channel_2"));
    }

    #[test]
    fn check_interrupted_turns_skips_complete_status() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        save_turn_state_in(base, "completed_chan", "pending");
        save_turn_state_in(base, "completed_chan", "complete");
        save_turn_state_in(base, "still_pending", "pending");

        let interrupted = check_interrupted_turns_in(base);
        // Only "still_pending" should be reported — "completed_chan" was
        // marked complete after its pending state.
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].0, "still_pending");
    }

    #[test]
    fn check_interrupted_turns_returns_empty_when_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No save_turn_state_in calls — directory doesn't exist.
        let interrupted = check_interrupted_turns_in(tmp.path());
        assert!(interrupted.is_empty());
    }

    #[test]
    fn sanitize_channel_id_strips_path_separators() {
        // Defensive: a hostile platform sending "../../etc/passwd" as a
        // channel ID must not escape the .turn_state directory.
        assert_eq!(sanitize_channel_id("../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_channel_id("normal-channel_123"), "normal-channel_123");
        assert_eq!(sanitize_channel_id(""), "");
    }
}
