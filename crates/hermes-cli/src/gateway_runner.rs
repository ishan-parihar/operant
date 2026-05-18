//! Gateway Runner
//!
//! Manages a singleton Gateway instance that persists across CLI commands
//! within a single process. Provides start/stop/restart lifecycle management
//! backed by configuration and the `hermes_core::gateway` module.

use anyhow::{Context, Result};
use hermes_core::agent::{AgentEvent, HermesAgent};
use hermes_core::config::runtime_config;
use hermes_core::config::AppConfig;
use hermes_core::gateway::{
    DiscordAdapter, Gateway, GatewayConfig, IncomingMessage, MessageHandler, OutgoingMessage,
    PlatformAdapter, SlackAdapter, TelegramAdapter, WebhookAdapter,
};
use hermes_core::gateway_pipeline::{MessagePipeline, PipelineAction};

use crate::gateway_commands::{handle_command, resolve_command, telegram_bot_commands, CommandContext};
use hermes_core::mcp::McpManager;
use hermes_core::tools::{HermesTool, ToolContext, TranscriptionTool};
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Returns the path to the gateway PID file used for cross-process status checks.
fn pid_file_path() -> std::path::PathBuf {
    hermes_core::platform::hermes_home().join("gateway.pid")
}
/// Message handler that processes incoming gateway messages through the Hermes agent.
struct GatewayMessageHandler {
    agent: Arc<HermesAgent>,
}

#[async_trait::async_trait]
impl MessageHandler for GatewayMessageHandler {
    async fn handle(&self, message: IncomingMessage) -> hermes_core::Result<OutgoingMessage> {
        match self.agent.run(message.content).await {
            Ok(response) => {
                let content = if response.content.trim().is_empty() {
                    // If content is empty but we have reasoning content, use that instead.
                    // Reasoning models (DeepSeek R1, etc.) may put the final answer in
                    // reasoning_content and leave content empty.
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
                Ok(OutgoingMessage::new(message.channel_id, content))
            }
            Err(e) => Ok(OutgoingMessage::new(
                &message.channel_id,
                format!("Error: {}", e),
            )),
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
        adapters.push(Arc::new(TelegramAdapter::new(
            config.telegram_token.clone(),
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
    };
    let dispatch_config = gw_config.clone();
    let adapters = build_adapters(&gw_config);

    let mut gateway = Gateway::new(gw_config);
    for adapter in adapters {
        gateway = gateway.with_adapter(adapter);
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
    let handler = Arc::new(GatewayMessageHandler {
        agent: Arc::new(agent),
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
        let cmd_body = format!(
            r#"{{"commands":{}}}"#,
            telegram_bot_commands()
        );
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
    // single-message timeline of all tool calls (like the hermes-agent does).
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
                        if let Some(response_text) =
                            handle_command(cmd_def.name, cmd_args, &ctx)
                        {
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
                        // Wait 30s before first notification (quick tasks skip it)
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        let start = std::time::Instant::now();
                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(60));
                        loop {
                            interval.tick().await;
                            let elapsed = start.elapsed().as_secs();
                            let minutes = elapsed / 60;
                            let seconds = elapsed % 60;
                            let body = if minutes > 0 {
                                format!(
                                    "\u{23F3} Still working... ({}m {}s elapsed...)",
                                    minutes, seconds
                                )
                            } else {
                                format!("\u{23F3} Still working... ({}s elapsed...)", seconds)
                            };
                            let msg = OutgoingMessage::new(&ch, &body).no_markdown();
                            if gw.send_to_platform("telegram", msg).await.is_err() {
                                break;
                            }
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
                hermes_core::env_passthrough::reload_dotenv();

                // ── 5.7 Turn state: mark pending ─────────────────────────────
                save_turn_state(&channel_id, "pending");

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
    *guard = Some(gateway);

    // Write PID file for cross-process status checks
    if let Ok(pid) = std::time::SystemTime::now()
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
    let temp_file = temp_dir.join(format!("hermes_tg_{}.{}", short_id, ext));
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
    use hermes_core::config::GatewaySettings;

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
        };

        assert!(gw_config.telegram_enabled);
        assert!(!gw_config.discord_enabled);
        assert!(gw_config.slack_enabled);
        assert!(!gw_config.webhooks_enabled);
        assert_eq!(gw_config.admins, vec!["admin1"]);
    }

    #[tokio::test]
    async fn test_gateway_start_stop_with_disabled_platforms() {
        // Stop any prior gateway first
        stop_gateway().await.ok();

        let config = AppConfig {
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
fn save_turn_state(channel_id: &str, status: &str) {
    let path = hermes_core::platform::hermes_home().join(".turn_state.json");
    let ts = chrono::Utc::now().to_rfc3339();
    let json = serde_json::json!({
        "channel_id": channel_id,
        "status": status,
        "timestamp": ts,
    });
    let _ = std::fs::write(path, json.to_string());
}

/// Check for interrupted turns on startup and log a warning.
pub fn check_interrupted_turns() {
    let path = hermes_core::platform::hermes_home().join(".turn_state.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(state) = serde_json::from_str::<serde_json::Value>(&content) {
            if state.get("status").and_then(|s| s.as_str()) == Some("pending") {
                tracing::warn!(
                    channel_id = %state["channel_id"],
                    timestamp = %state["timestamp"],
                    "Detected interrupted turn from previous session"
                );
            }
        }
    }
}
