//! Discord server introspection and management tool.
//!
//! Provides the agent with the ability to interact with Discord servers
//! via the Discord REST API. Uses the bot token from `DISCORD_BOT_TOKEN`.
//!
//! Two tools are exposed:
//! - `DiscordTool` ("discord") — core read/participate actions
//! - `DiscordAdminTool` ("discord_admin") — server management actions

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const TIMEOUT_SECS: u64 = 15;

// ---------------------------------------------------------------------------
// Channel type mapping
// ---------------------------------------------------------------------------

fn channel_type_name(type_id: i64) -> &'static str {
    match type_id {
        0 => "text",
        2 => "voice",
        4 => "category",
        5 => "announcement",
        10 => "announcement_thread",
        11 => "public_thread",
        12 => "private_thread",
        13 => "stage",
        15 => "forum",
        16 => "media",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// 403 error enrichment hints
// ---------------------------------------------------------------------------

fn enrich_403(action: &str, body: &str) -> String {
    let hint = match action {
        "pin_message" => {
            "Bot lacks MANAGE_MESSAGES permission in this channel. \
            Ask the server admin to grant the bot a role that has MANAGE_MESSAGES, \
            or a per-channel overwrite."
        }
        "unpin_message" => "Bot lacks MANAGE_MESSAGES permission in this channel.",
        "delete_message" => {
            "Bot lacks MANAGE_MESSAGES permission in this channel, \
            or cannot view the channel/message."
        }
        "create_thread" => "Bot lacks CREATE_PUBLIC_THREADS in this channel, or cannot view it.",
        "add_role" => {
            "Either the bot lacks MANAGE_ROLES, or the target role sits higher \
            than the bot's highest role. Roles can only be assigned below the \
            bot's own position in the role hierarchy."
        }
        "remove_role" => {
            "Either the bot lacks MANAGE_ROLES, or the target role sits higher \
            than the bot's highest role."
        }
        "fetch_messages" => {
            "Bot cannot view this channel (missing VIEW_CHANNEL \
            or READ_MESSAGE_HISTORY)."
        }
        "list_pins" => {
            "Bot cannot view this channel (missing VIEW_CHANNEL \
            or READ_MESSAGE_HISTORY)."
        }
        "channel_info" => "Bot cannot view this channel (missing VIEW_CHANNEL).",
        "search_members" => {
            "Likely missing the Server Members privileged intent — enable it \
            in the Discord Developer Portal under your bot's settings."
        }
        "member_info" => {
            "Bot cannot see this guild member (missing Server Members intent or \
            insufficient permissions)."
        }
        _ => "",
    };
    if hint.is_empty() {
        format!("Discord API 403 (forbidden) on '{action}'. (Raw: {body})")
    } else {
        format!("Discord API 403 (forbidden) on '{action}'. {hint} (Raw: {body})")
    }
}

// ---------------------------------------------------------------------------
// Discord API error
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DiscordApiError {
    status: u16,
    body: String,
}

// ---------------------------------------------------------------------------
// HTTP request helper
// ---------------------------------------------------------------------------

async fn discord_request(
    method: &str,
    path: &str,
    token: &str,
    params: Option<&HashMap<String, String>>,
    body: Option<Value>,
) -> Result<Value, DiscordApiError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| DiscordApiError {
            status: 0,
            body: format!("Failed to create HTTP client: {e}"),
        })?;

    let url = format!("{DISCORD_API_BASE}{path}");

    let mut req = client
        .request(
            match method {
                "GET" => reqwest::Method::GET,
                "POST" => reqwest::Method::POST,
                "PUT" => reqwest::Method::PUT,
                "DELETE" => reqwest::Method::DELETE,
                _ => reqwest::Method::GET,
            },
            &url,
        )
        .header("Authorization", format!("Bot {token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", "Operant-RS");

    if let Some(p) = params {
        req = req.query(p);
    }

    if let Some(b) = body {
        req = req.json(&b);
    }

    let resp = req.send().await.map_err(|e| DiscordApiError {
        status: 0,
        body: format!("Request failed: {e}"),
    })?;

    let status = resp.status().as_u16();

    if status == 204 {
        return Ok(json!(null));
    }

    let response_body: Value = resp.json().await.map_err(|e| DiscordApiError {
        status,
        body: format!("Failed to parse response: {e}"),
    })?;

    if status >= 400 {
        let body_str = response_body.to_string();
        return Err(DiscordApiError {
            status,
            body: body_str,
        });
    }

    Ok(response_body)
}

// ---------------------------------------------------------------------------
// Required params validation
// ---------------------------------------------------------------------------

fn check_required_params(action: &str, args: &DiscordArgs) -> Vec<String> {
    let params: &[&str] = match action {
        "server_info" => &["guild_id"],
        "list_channels" => &["guild_id"],
        "list_roles" => &["guild_id"],
        "member_info" => &["guild_id", "user_id"],
        "search_members" => &["guild_id", "query"],
        "channel_info" => &["channel_id"],
        "fetch_messages" => &["channel_id"],
        "list_pins" => &["channel_id"],
        "pin_message" => &["channel_id", "message_id"],
        "unpin_message" => &["channel_id", "message_id"],
        "delete_message" => &["channel_id", "message_id"],
        "create_thread" => &["channel_id", "name"],
        "add_role" => &["guild_id", "user_id", "role_id"],
        "remove_role" => &["guild_id", "user_id", "role_id"],
        _ => &[],
    };

    params
        .iter()
        .filter(|p| {
            let val = match **p {
                "guild_id" => args.guild_id.as_deref(),
                "channel_id" => args.channel_id.as_deref(),
                "user_id" => args.user_id.as_deref(),
                "role_id" => args.role_id.as_deref(),
                "message_id" => args.message_id.as_deref(),
                "query" => args.query.as_deref(),
                "name" => args.name.as_deref(),
                _ => None,
            };
            val.map_or(true, |s| s.is_empty())
        })
        .map(|p| p.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Shared arguments for both Discord tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscordArgs {
    /// The action to perform
    action: String,
    /// Discord server (guild) ID
    guild_id: Option<String>,
    /// Discord channel ID
    channel_id: Option<String>,
    /// Discord user ID
    user_id: Option<String>,
    /// Discord role ID
    role_id: Option<String>,
    /// Discord message ID
    message_id: Option<String>,
    /// Member name prefix to search for (search_members)
    query: Option<String>,
    /// New thread name (create_thread)
    name: Option<String>,
    /// Max results (default 50, max 100). Applies to fetch_messages, search_members.
    limit: Option<i64>,
    /// Snowflake ID for reverse pagination (fetch_messages)
    before: Option<String>,
    /// Snowflake ID for forward pagination (fetch_messages)
    after: Option<String>,
    /// Thread archive duration in minutes (create_thread, default 1440). Valid: 60, 1440, 4320, 10080.
    auto_archive_duration: Option<i64>,
}

// ---------------------------------------------------------------------------
// Action handler type and registry
// ---------------------------------------------------------------------------

type ActionHandler =
    for<'a> fn(
        &'a str,
        &'a DiscordArgs,
    ) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>>;

fn action_handler_registry() -> &'static HashMap<&'static str, ActionHandler> {
    static REGISTRY: OnceLock<HashMap<&'static str, ActionHandler>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut m: HashMap<&'static str, ActionHandler> = HashMap::new();
        m.insert("list_guilds", handle_list_guilds as ActionHandler);
        m.insert("server_info", handle_server_info as ActionHandler);
        m.insert("list_channels", handle_list_channels as ActionHandler);
        m.insert("channel_info", handle_channel_info as ActionHandler);
        m.insert("list_roles", handle_list_roles as ActionHandler);
        m.insert("member_info", handle_member_info as ActionHandler);
        m.insert("search_members", handle_search_members as ActionHandler);
        m.insert("fetch_messages", handle_fetch_messages as ActionHandler);
        m.insert("list_pins", handle_list_pins as ActionHandler);
        m.insert("pin_message", handle_pin_message as ActionHandler);
        m.insert("unpin_message", handle_unpin_message as ActionHandler);
        m.insert("delete_message", handle_delete_message as ActionHandler);
        m.insert("create_thread", handle_create_thread as ActionHandler);
        m.insert("add_role", handle_add_role as ActionHandler);
        m.insert("remove_role", handle_remove_role as ActionHandler);
        m
    })
}

// ---------------------------------------------------------------------------
// Dispatch logic
// ---------------------------------------------------------------------------

async fn dispatch_action(
    args: &DiscordArgs,
    token: &str,
    handler_name: &str,
    allowed_actions: &[&str],
) -> ToolResult {
    let action = args.action.trim();

    // Validate action is in allowed set
    if !allowed_actions.contains(&action) {
        let available = allowed_actions.join(", ");
        return ToolResult::error(
            handler_name,
            format!("Unknown action: {action}. Available actions: [{available}]"),
        );
    }

    // Validate required params
    let missing = check_required_params(action, args);
    if !missing.is_empty() {
        return ToolResult::error(
            handler_name,
            format!(
                "Missing required parameters for '{action}': {}",
                missing.join(", ")
            ),
        );
    }

    // Look up handler and execute
    let registry = action_handler_registry();
    match registry.get(action) {
        Some(handler) => match handler(token, args).await {
            Ok(value) => ToolResult::success(handler_name, value),
            Err(err) => {
                if err.status == 403 {
                    ToolResult::error(handler_name, enrich_403(action, &err.body))
                } else if err.status == 0 {
                    ToolResult::error(handler_name, err.body)
                } else {
                    ToolResult::error(
                        handler_name,
                        format!("Discord API error {}: {}", err.status, err.body),
                    )
                }
            }
        },
        None => ToolResult::error(handler_name, format!("Unknown action: {action}")),
    }
}

// ---------------------------------------------------------------------------
// Helper: get bot token from environment
// ---------------------------------------------------------------------------

fn get_bot_token() -> Option<String> {
    let token = std::env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
    let trimmed = token.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

// ---------------------------------------------------------------------------
// Manifest for schema descriptions
// ---------------------------------------------------------------------------

const ACTION_MANIFEST: &[(&str, &str, &str)] = &[
    ("list_guilds", "()", "list servers the bot is in"),
    (
        "server_info",
        "(guild_id)",
        "server details + member counts",
    ),
    (
        "list_channels",
        "(guild_id)",
        "all channels grouped by category",
    ),
    ("channel_info", "(channel_id)", "single channel details"),
    ("list_roles", "(guild_id)", "roles sorted by position"),
    (
        "member_info",
        "(guild_id, user_id)",
        "lookup a specific member",
    ),
    (
        "search_members",
        "(guild_id, query)",
        "find members by name prefix",
    ),
    (
        "fetch_messages",
        "(channel_id)",
        "recent messages; optional before/after snowflakes",
    ),
    ("list_pins", "(channel_id)", "pinned messages in a channel"),
    ("pin_message", "(channel_id, message_id)", "pin a message"),
    (
        "unpin_message",
        "(channel_id, message_id)",
        "unpin a message",
    ),
    (
        "delete_message",
        "(channel_id, message_id)",
        "delete a message",
    ),
    (
        "create_thread",
        "(channel_id, name)",
        "create a public thread; optional message_id anchor",
    ),
    ("add_role", "(guild_id, user_id, role_id)", "assign a role"),
    (
        "remove_role",
        "(guild_id, user_id, role_id)",
        "remove a role",
    ),
];

fn build_schema_description(tool_name: &str, actions: &[&str]) -> String {
    let manifest_lines: Vec<String> = ACTION_MANIFEST
        .iter()
        .filter(|(name, _, _)| actions.contains(name))
        .map(|(name, sig, desc)| format!("  {name}{sig}  — {desc}"))
        .collect();
    let manifest_block = manifest_lines.join("\n");

    if tool_name == "discord_admin" {
        format!(
            "Manage a Discord server via the REST API.\n\n\
             Available actions:\n\
             {manifest_block}\n\n\
             Call list_guilds first to discover guild_ids, then list_channels for \
             channel_ids. Runtime errors will tell you if the bot lacks a specific \
             per-guild permission (e.g. MANAGE_ROLES for add_role)."
        )
    } else {
        format!(
            "Read and participate in a Discord server.\n\n\
             Available actions:\n\
             {manifest_block}\n\n\
             Use the channel_id from the current conversation context. \
             Use search_members to look up user IDs by name prefix."
        )
    }
}

fn build_schema_params(actions: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": actions,
                "description": "The action to perform"
            },
            "guild_id": {
                "type": "string",
                "description": "Discord server (guild) ID."
            },
            "channel_id": {
                "type": "string",
                "description": "Discord channel ID."
            },
            "user_id": {
                "type": "string",
                "description": "Discord user ID."
            },
            "role_id": {
                "type": "string",
                "description": "Discord role ID."
            },
            "message_id": {
                "type": "string",
                "description": "Discord message ID."
            },
            "query": {
                "type": "string",
                "description": "Member name prefix to search for (search_members)."
            },
            "name": {
                "type": "string",
                "description": "New thread name (create_thread)."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "description": "Max results (default 50). Applies to fetch_messages, search_members."
            },
            "before": {
                "type": "string",
                "description": "Snowflake ID for reverse pagination (fetch_messages)."
            },
            "after": {
                "type": "string",
                "description": "Snowflake ID for forward pagination (fetch_messages)."
            },
            "auto_archive_duration": {
                "type": "integer",
                "enum": [60, 1440, 4320, 10080],
                "description": "Thread archive duration in minutes (create_thread, default 1440)."
            }
        },
        "required": ["action"]
    })
}

// ============================================================================
// ACTION HANDLERS — all 15 fully implemented
// ============================================================================

fn handle_list_guilds<'a>(
    token: &'a str,
    _args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    Box::pin(async move {
        let resp = discord_request("GET", "/users/@me/guilds", token, None, None).await?;
        let guilds = resp
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|g| {
                        json!({
                            "id": g["id"],
                            "name": g["name"],
                            "icon": g.get("icon"),
                            "owner": g.get("owner").unwrap_or(&json!(false)),
                            "permissions": g.get("permissions"),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(json!({
            "guilds": guilds,
            "count": guilds.len(),
        }))
    })
}

fn handle_server_info<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    Box::pin(async move {
        let mut params = HashMap::new();
        params.insert("with_counts".to_string(), "true".to_string());
        let g = discord_request(
            "GET",
            &format!("/guilds/{guild_id}"),
            token,
            Some(&params),
            None,
        )
        .await?;

        Ok(json!({
            "id": g["id"],
            "name": g["name"],
            "description": g.get("description"),
            "icon": g.get("icon"),
            "owner_id": g.get("owner_id"),
            "member_count": g.get("approximate_member_count"),
            "online_count": g.get("approximate_presence_count"),
            "features": g.get("features").unwrap_or(&json!([])),
            "premium_tier": g.get("premium_tier"),
            "premium_subscription_count": g.get("premium_subscription_count"),
            "verification_level": g.get("verification_level"),
        }))
    })
}

fn handle_list_channels<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    Box::pin(async move {
        let channels = discord_request(
            "GET",
            &format!("/guilds/{guild_id}/channels"),
            token,
            None,
            None,
        )
        .await?;
        let channels_arr = channels
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default();

        // First pass: collect categories
        let mut categories: HashMap<String, Value> = HashMap::new();
        for ch in channels_arr {
            if ch["type"].as_i64() == Some(4) {
                categories.insert(
                    ch["id"].as_str().unwrap_or("").to_string(),
                    json!({
                        "id": ch["id"],
                        "name": ch["name"],
                        "position": ch.get("position").unwrap_or(&json!(0)),
                        "channels": Vec::<Value>::new(),
                    }),
                );
            }
        }

        // Second pass: assign channels to categories
        let mut uncategorized: Vec<Value> = Vec::new();
        for ch in channels_arr {
            if ch["type"].as_i64() == Some(4) {
                continue;
            }
            let entry = json!({
                "id": ch["id"],
                "name": ch.get("name").unwrap_or(&json!("")),
                "type": channel_type_name(ch["type"].as_i64().unwrap_or(0)),
                "position": ch.get("position").unwrap_or(&json!(0)),
                "topic": ch.get("topic"),
                "nsfw": ch.get("nsfw").unwrap_or(&json!(false)),
            });
            let parent_id = ch
                .get("parent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let Some(ref pid) = parent_id {
                if categories.contains_key(pid) {
                    if let Some(cat) = categories.get_mut(pid) {
                        if let Some(arr) = cat.get_mut("channels").and_then(|a| a.as_array_mut()) {
                            arr.push(entry);
                            continue;
                        }
                    }
                }
            }
            uncategorized.push(entry);
        }

        // Sort categories by position and their channels by position
        let mut sorted_cats: Vec<Value> = categories.into_values().collect();
        sorted_cats.sort_by_key(|c| c["position"].as_i64().unwrap_or(0));
        for cat in &mut sorted_cats {
            if let Some(arr) = cat.get_mut("channels").and_then(|a| a.as_array_mut()) {
                arr.sort_by_key(|c| c["position"].as_i64().unwrap_or(0));
            }
        }
        uncategorized.sort_by_key(|c| c["position"].as_i64().unwrap_or(0));

        let mut result: Vec<Value> = Vec::new();
        if !uncategorized.is_empty() {
            result.push(json!({
                "category": null,
                "channels": uncategorized,
            }));
        }
        for cat in sorted_cats {
            result.push(json!({
                "category": {
                    "id": cat["id"],
                    "name": cat["name"],
                },
                "channels": cat["channels"],
            }));
        }

        let total: usize = result
            .iter()
            .map(|g| g["channels"].as_array().map(|a| a.len()).unwrap_or(0))
            .sum();

        Ok(json!({
            "channel_groups": result,
            "total_channels": total,
        }))
    })
}

fn handle_channel_info<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    Box::pin(async move {
        let ch =
            discord_request("GET", &format!("/channels/{channel_id}"), token, None, None).await?;

        Ok(json!({
            "id": ch["id"],
            "name": ch.get("name"),
            "type": channel_type_name(ch["type"].as_i64().unwrap_or(0)),
            "guild_id": ch.get("guild_id"),
            "topic": ch.get("topic"),
            "nsfw": ch.get("nsfw").unwrap_or(&json!(false)),
            "position": ch.get("position"),
            "parent_id": ch.get("parent_id"),
            "rate_limit_per_user": ch.get("rate_limit_per_user").unwrap_or(&json!(0)),
            "last_message_id": ch.get("last_message_id"),
        }))
    })
}

fn handle_list_roles<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    Box::pin(async move {
        let roles = discord_request(
            "GET",
            &format!("/guilds/{guild_id}/roles"),
            token,
            None,
            None,
        )
        .await?;
        let roles_arr = roles.as_array().map(|a| a.as_slice()).unwrap_or_default();

        let mut sorted: Vec<&Value> = roles_arr.iter().collect();
        sorted.sort_by_key(|r| -(r.get("position").and_then(|v| v.as_i64()).unwrap_or(0)));

        let result: Vec<Value> = sorted
            .iter()
            .map(|r| {
                let color = r.get("color").and_then(|v| v.as_i64()).unwrap_or(0);
                json!({
                    "id": r["id"],
                    "name": r["name"],
                    "color": if color != 0 { Some(format!("#{color:06x}")) } else { None },
                    "position": r.get("position").unwrap_or(&json!(0)),
                    "mentionable": r.get("mentionable").unwrap_or(&json!(false)),
                    "managed": r.get("managed").unwrap_or(&json!(false)),
                    "member_count": r.get("member_count"),
                    "hoist": r.get("hoist").unwrap_or(&json!(false)),
                })
            })
            .collect();

        Ok(json!({
            "roles": result,
            "count": result.len(),
        }))
    })
}

fn handle_member_info<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    let user_id = args.user_id.clone().unwrap_or_default();
    Box::pin(async move {
        let m = discord_request(
            "GET",
            &format!("/guilds/{guild_id}/members/{user_id}"),
            token,
            None,
            None,
        )
        .await?;
        let user = m.get("user");

        Ok(json!({
            "user_id": user.and_then(|u| u.get("id")),
            "username": user.and_then(|u| u.get("username")),
            "display_name": user.and_then(|u| u.get("global_name")),
            "nickname": m.get("nick"),
            "avatar": user.and_then(|u| u.get("avatar")),
            "bot": user.and_then(|u| u.get("bot")).unwrap_or(&json!(false)),
            "roles": m.get("roles").unwrap_or(&json!([])),
            "joined_at": m.get("joined_at"),
            "premium_since": m.get("premium_since"),
        }))
    })
}

fn handle_search_members<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    let query = args.query.clone().unwrap_or_default();
    let limit = args.limit.unwrap_or(20).min(100).max(1);
    Box::pin(async move {
        let mut params = HashMap::new();
        params.insert("query".to_string(), query);
        params.insert("limit".to_string(), limit.to_string());

        let members = discord_request(
            "GET",
            &format!("/guilds/{guild_id}/members/search"),
            token,
            Some(&params),
            None,
        )
        .await?;
        let result: Vec<Value> = members
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|m| {
                        let user = m.get("user");
                        json!({
                            "user_id": user.and_then(|u| u.get("id")),
                            "username": user.and_then(|u| u.get("username")),
                            "display_name": user.and_then(|u| u.get("global_name")),
                            "nickname": m.get("nick"),
                            "bot": user.and_then(|u| u.get("bot")).unwrap_or(&json!(false)),
                            "roles": m.get("roles").unwrap_or(&json!([])),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(json!({
            "members": result,
            "count": result.len(),
        }))
    })
}

fn handle_fetch_messages<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    let limit = args.limit.unwrap_or(50).min(100).max(1);
    let before = args.before.clone().unwrap_or_default();
    let after = args.after.clone().unwrap_or_default();
    Box::pin(async move {
        let mut params = HashMap::new();
        params.insert("limit".to_string(), limit.to_string());
        if !before.is_empty() {
            params.insert("before".to_string(), before);
        }
        if !after.is_empty() {
            params.insert("after".to_string(), after);
        }

        let messages = discord_request(
            "GET",
            &format!("/channels/{channel_id}/messages"),
            token,
            Some(&params),
            None,
        )
        .await?;
        let result: Vec<Value> =
            messages
                .as_array()
                .map(|arr| {
                    arr.iter().map(|msg| {
                let author = msg.get("author");
                json!({
                    "id": msg["id"],
                    "content": msg.get("content").unwrap_or(&json!("")),
                    "author": {
                        "id": author.and_then(|a| a.get("id")),
                        "username": author.and_then(|a| a.get("username")),
                        "display_name": author.and_then(|a| a.get("global_name")),
                        "bot": author.and_then(|a| a.get("bot")).unwrap_or(&json!(false)),
                    },
                    "timestamp": msg.get("timestamp"),
                    "edited_timestamp": msg.get("edited_timestamp"),
                    "attachments": msg.get("attachments").map(|atts| {
                        atts.as_array().map(|arr| {
                            arr.iter().map(|a| {
                                json!({
                                    "filename": a.get("filename"),
                                    "url": a.get("url"),
                                    "size": a.get("size"),
                                })
                            }).collect::<Vec<_>>()
                        }).unwrap_or_default()
                    }).unwrap_or_default(),
                    "reactions": msg.get("reactions").map(|r| {
                        r.as_array().map(|arr| {
                            arr.iter().map(|rxn| {
                                json!({
                                    "emoji": rxn.get("emoji").and_then(|e| e.get("name")),
                                    "count": rxn.get("count").unwrap_or(&json!(0)),
                                })
                            }).collect::<Vec<_>>()
                        }).unwrap_or_default()
                    }).unwrap_or_default(),
                    "pinned": msg.get("pinned").unwrap_or(&json!(false)),
                })
            }).collect()
                })
                .unwrap_or_default();

        Ok(json!({
            "messages": result,
            "count": result.len(),
        }))
    })
}

fn handle_list_pins<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    Box::pin(async move {
        let messages = discord_request(
            "GET",
            &format!("/channels/{channel_id}/pins"),
            token,
            None,
            None,
        )
        .await?;
        let result: Vec<Value> = messages
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|msg| {
                        let author = msg.get("author");
                        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                        let truncated: String = content.chars().take(200).collect();
                        json!({
                            "id": msg["id"],
                            "content": truncated,
                            "author": author.and_then(|a| a.get("username")),
                            "timestamp": msg.get("timestamp"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(json!({
            "pinned_messages": result,
            "count": result.len(),
        }))
    })
}

fn handle_pin_message<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    let message_id = args.message_id.clone().unwrap_or_default();
    Box::pin(async move {
        discord_request(
            "PUT",
            &format!("/channels/{channel_id}/pins/{message_id}"),
            token,
            None,
            None,
        )
        .await?;
        Ok(json!({
            "success": true,
            "message": format!("Message {message_id} pinned."),
        }))
    })
}

fn handle_unpin_message<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    let message_id = args.message_id.clone().unwrap_or_default();
    Box::pin(async move {
        discord_request(
            "DELETE",
            &format!("/channels/{channel_id}/pins/{message_id}"),
            token,
            None,
            None,
        )
        .await?;
        Ok(json!({
            "success": true,
            "message": format!("Message {message_id} unpinned."),
        }))
    })
}

fn handle_delete_message<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    let message_id = args.message_id.clone().unwrap_or_default();
    Box::pin(async move {
        discord_request(
            "DELETE",
            &format!("/channels/{channel_id}/messages/{message_id}"),
            token,
            None,
            None,
        )
        .await?;
        Ok(json!({
            "success": true,
            "message": format!("Message {message_id} deleted."),
        }))
    })
}

fn handle_create_thread<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let channel_id = args.channel_id.clone().unwrap_or_default();
    let name = args.name.clone().unwrap_or_default();
    let message_id = args.message_id.clone().unwrap_or_default();
    let auto_archive = args.auto_archive_duration.unwrap_or(1440);
    Box::pin(async move {
        let (path, body) = if message_id.is_empty() {
            (
                format!("/channels/{channel_id}/threads"),
                json!({
                    "name": name,
                    "auto_archive_duration": auto_archive,
                    "type": 11,
                }),
            )
        } else {
            (
                format!("/channels/{channel_id}/messages/{message_id}/threads"),
                json!({
                    "name": name,
                    "auto_archive_duration": auto_archive,
                }),
            )
        };

        let thread = discord_request("POST", &path, token, None, Some(body)).await?;
        Ok(json!({
            "success": true,
            "thread_id": thread["id"],
            "name": thread.get("name"),
        }))
    })
}

fn handle_add_role<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    let user_id = args.user_id.clone().unwrap_or_default();
    let role_id = args.role_id.clone().unwrap_or_default();
    Box::pin(async move {
        discord_request(
            "PUT",
            &format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
            token,
            None,
            None,
        )
        .await?;
        Ok(json!({
            "success": true,
            "message": format!("Role {role_id} added to user {user_id}."),
        }))
    })
}

fn handle_remove_role<'a>(
    token: &'a str,
    args: &'a DiscordArgs,
) -> Pin<Box<dyn Future<Output = Result<Value, DiscordApiError>> + Send + 'a>> {
    let guild_id = args.guild_id.clone().unwrap_or_default();
    let user_id = args.user_id.clone().unwrap_or_default();
    let role_id = args.role_id.clone().unwrap_or_default();
    Box::pin(async move {
        discord_request(
            "DELETE",
            &format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
            token,
            None,
            None,
        )
        .await?;
        Ok(json!({
            "success": true,
            "message": format!("Role {role_id} removed from user {user_id}."),
        }))
    })
}

// ============================================================================
// TOOL STRUCTS
// ============================================================================

const CORE_ACTIONS: &[&str] = &["fetch_messages", "search_members", "create_thread"];
const ADMIN_ACTIONS: &[&str] = &[
    "list_guilds",
    "server_info",
    "list_channels",
    "channel_info",
    "list_roles",
    "member_info",
    "list_pins",
    "pin_message",
    "unpin_message",
    "delete_message",
    "add_role",
    "remove_role",
];

/// Core Discord tool (read/participate actions only)
pub struct DiscordTool;

#[async_trait]
impl OperantTool for DiscordTool {
    fn name(&self) -> &str {
        "discord"
    }

    fn description(&self) -> &str {
        "Read and participate in a Discord server"
    }

    fn toolset(&self) -> &str {
        "messaging"
    }

    fn is_available(&self) -> bool {
        std::env::var("DISCORD_BOT_TOKEN").is_ok() || std::env::var("DISCORD_TOKEN").is_ok()
    }

    fn schema(&self) -> ToolSchema {
        let desc = build_schema_description("discord", CORE_ACTIONS);
        let params = build_schema_params(CORE_ACTIONS);
        ToolSchema::new("discord", desc, params)
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let token = match get_bot_token() {
            Some(t) => t,
            None => return ToolResult::error("discord", "DISCORD_BOT_TOKEN not configured"),
        };

        let args: DiscordArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("discord", format!("Invalid arguments: {e}")),
        };

        dispatch_action(&args, &token, "discord", CORE_ACTIONS).await
    }
}

/// Discord admin tool (server management actions)
pub struct DiscordAdminTool;

#[async_trait]
impl OperantTool for DiscordAdminTool {
    fn name(&self) -> &str {
        "discord_admin"
    }

    fn description(&self) -> &str {
        "Manage a Discord server via the REST API"
    }

    fn toolset(&self) -> &str {
        "messaging"
    }

    fn is_available(&self) -> bool {
        std::env::var("DISCORD_BOT_TOKEN").is_ok() || std::env::var("DISCORD_TOKEN").is_ok()
    }

    fn schema(&self) -> ToolSchema {
        let desc = build_schema_description("discord_admin", ADMIN_ACTIONS);
        let params = build_schema_params(ADMIN_ACTIONS);
        ToolSchema::new("discord_admin", desc, params)
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let token = match get_bot_token() {
            Some(t) => t,
            None => return ToolResult::error("discord_admin", "DISCORD_BOT_TOKEN not configured"),
        };

        let args: DiscordArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("discord_admin", format!("Invalid arguments: {e}")),
        };

        dispatch_action(&args, &token, "discord_admin", ADMIN_ACTIONS).await
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discord_tool_metadata() {
        let tool = DiscordTool;
        assert_eq!(tool.name(), "discord");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "discord");
        let params = &schema.parameters;
        assert!(params["properties"]["action"].is_object());
        assert!(params["required"][0] == "action");
    }

    #[tokio::test]
    async fn test_discord_admin_metadata() {
        let tool = DiscordAdminTool;
        assert_eq!(tool.name(), "discord_admin");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "discord_admin");
        assert!(schema.parameters["properties"]["action"].is_object());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_discord_no_token_returns_error() {
        let saved = std::env::var("DISCORD_BOT_TOKEN").ok();
        std::env::remove_var("DISCORD_BOT_TOKEN");
        let tool = DiscordTool;
        let result = tool
            .execute(
                json!({ "action": "fetch_messages", "channel_id": "123" }),
                ToolContext::default(),
            )
            .await;
        if let Some(token) = saved {
            std::env::set_var("DISCORD_BOT_TOKEN", token);
        }
        assert!(!result.success);
        assert!(result.error.unwrap().contains("DISCORD_BOT_TOKEN"));
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_discord_unknown_action() {
        // Set a fake token so we get past the token check
        std::env::set_var("DISCORD_BOT_TOKEN", "test_token");
        let tool = DiscordTool;
        let result = tool
            .execute(json!({ "action": "nonexistent" }), ToolContext::default())
            .await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Unknown action"));
        assert!(err.contains("fetch_messages"));
        std::env::remove_var("DISCORD_BOT_TOKEN");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_discord_missing_params() {
        std::env::set_var("DISCORD_BOT_TOKEN", "test_token");
        let tool = DiscordTool;
        let result = tool
            .execute(
                json!({ "action": "fetch_messages" }), // missing channel_id
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Missing required parameters"));
        assert!(err.contains("channel_id"));
        std::env::remove_var("DISCORD_BOT_TOKEN");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_discord_admin_unknown_action_from_core() {
        std::env::set_var("DISCORD_BOT_TOKEN", "test_token");
        // DiscordTool should reject an admin action
        let tool = DiscordTool;
        let result = tool
            .execute(json!({ "action": "list_guilds" }), ToolContext::default())
            .await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Unknown action"));
        std::env::remove_var("DISCORD_BOT_TOKEN");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_discord_admin_rejects_core_action() {
        std::env::set_var("DISCORD_BOT_TOKEN", "test_token");
        // DiscordAdminTool should reject a core action
        let tool = DiscordAdminTool;
        let result = tool
            .execute(
                json!({ "action": "fetch_messages", "channel_id": "123" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("Unknown action"));
        std::env::remove_var("DISCORD_BOT_TOKEN");
    }

    #[tokio::test]
    async fn test_channel_type_name() {
        assert_eq!(channel_type_name(0), "text");
        assert_eq!(channel_type_name(2), "voice");
        assert_eq!(channel_type_name(4), "category");
        assert_eq!(channel_type_name(11), "public_thread");
        assert_eq!(channel_type_name(99), "unknown");
    }

    #[test]
    fn test_enrich_403() {
        let msg = enrich_403("pin_message", "test body");
        assert!(msg.contains("MANAGE_MESSAGES"));
        assert!(msg.contains("test body"));

        let generic = enrich_403("unknown_action", "body");
        assert!(generic.contains("unknown_action"));
        assert!(generic.contains("body"));
    }

    #[test]
    fn test_required_params_valid() {
        let args = DiscordArgs {
            action: "server_info".to_string(),
            guild_id: Some("123".to_string()),
            channel_id: None,
            user_id: None,
            role_id: None,
            message_id: None,
            query: None,
            name: None,
            limit: None,
            before: None,
            after: None,
            auto_archive_duration: None,
        };
        let missing = check_required_params("server_info", &args);
        assert!(missing.is_empty());

        // Test missing guild_id
        let args2 = DiscordArgs {
            guild_id: None,
            ..args
        };
        let missing2 = check_required_params("server_info", &args2);
        assert_eq!(missing2.len(), 1);
        assert_eq!(missing2[0], "guild_id");
    }

    #[test]
    fn test_schema_actions_enum() {
        let core_schema = DiscordTool.schema();
        let core_actions = core_schema.parameters["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(core_actions.contains(&"fetch_messages"));
        assert!(core_actions.contains(&"search_members"));
        assert!(core_actions.contains(&"create_thread"));
        assert!(!core_actions.contains(&"list_guilds"));

        let admin_schema = DiscordAdminTool.schema();
        let admin_actions = admin_schema.parameters["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(admin_actions.contains(&"list_guilds"));
        assert!(admin_actions.contains(&"server_info"));
        assert!(!admin_actions.contains(&"fetch_messages"));
    }
}

// ---------------------------------------------------------------------------
// (the tests above need serial_test for env var isolation)
// injected below the existing test module
