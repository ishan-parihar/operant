//! `prompts` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use operant_providers::{self};
use operant_runtime::i18n;
use std::fmt::Write;
use std::path::Path;

use super::*;

pub(crate) fn channel_delivery_instructions(channel_name: &str) -> Option<&'static str> {
    match channel_name {
        "matrix" => Some(
            "When responding on Matrix:\n\
             - Use Markdown formatting (bold, italic, code blocks)\n\
             - Be concise and direct\n\
             - For media attachments use markers: [IMAGE:<absolute-path>], [DOCUMENT:<absolute-path>], [VIDEO:<absolute-path>], [AUDIO:<absolute-path>], or [VOICE:<absolute-path>]\n\
             - Paths inside markers MUST be absolute (starting with /). Never use relative paths.\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n\
             - When you receive a [Voice message], the user spoke to you. Respond naturally as in conversation.\n\
             - Your text reply will automatically be converted to audio and sent back as a voice message.\n",
        ),
        "discord" => Some(
            "When responding on Discord:\n\
             - Use Markdown formatting (bold, italic, code blocks)\n\
             - Be concise and direct\n\
             - For media attachments use markers: [IMAGE:<absolute-path>], [DOCUMENT:<absolute-path>], [VIDEO:<absolute-path>], [AUDIO:<absolute-path>], or [VOICE:<absolute-path>]\n\
             - Paths inside markers MUST be absolute (starting with /) and live inside the configured workspace directory. Never use relative paths.\n\
             - Remote media is also accepted via http:// or https:// URLs in the same marker form.\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n",
        ),
        "telegram" => Some(
            "When responding on Telegram:\n\
             - Include media markers for files or URLs that should be sent as attachments\n\
             - Use **bold** for key terms, section titles, and important info (renders as <b>)\n\
             - Use *italic* for emphasis (renders as <i>)\n\
             - Use `backticks` for inline code, commands, or technical terms\n\
             - Use triple backticks for code blocks\n\
             - Use emoji naturally to add personality — but don't overdo it\n\
             - Be concise and direct. Skip filler phrases like 'Great question!' or 'Certainly!'\n\
             - Structure longer answers with bold headers, not raw markdown ## headers\n\
             - For media attachments use markers: [IMAGE:<path-or-url>], [DOCUMENT:<path-or-url>], [VIDEO:<path-or-url>], [AUDIO:<path-or-url>], or [VOICE:<path-or-url>]\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n\
             - Use tool results silently: answer the latest user message directly, and do not narrate delayed/internal tool execution bookkeeping.",
        ),
        "qq" => Some(
            "When responding on QQ:\n\
             - Use Markdown formatting\n\
             - Be concise and direct\n\
             - For media attachments use markers: [IMAGE:<path-or-url>], [DOCUMENT:<path-or-url>], \
               [VIDEO:<path-or-url>], [VOICE:<path-or-url>]\n\
             - Voice supports .wav, .mp3, .silk formats only. Other audio formats use [DOCUMENT:]\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n",
        ),
        "wechat" => Some(
            "When responding on WeChat:\n\
             - Be concise and direct\n\
             - For media attachments use markers: [IMAGE:<path-or-url>], [DOCUMENT:<path-or-url>], \
               [VIDEO:<path-or-url>], [AUDIO:<path-or-url>], or [VOICE:<path-or-url>]\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n\
             - Use absolute local paths when sending generated files whenever possible.\n",
        ),
        _ => None,
    }
}

pub(crate) fn build_channel_system_prompt(
    base_prompt: &str,
    channel_name: &str,
    reply_target: &str,
    sender: &str,
) -> String {
    let mut prompt = base_prompt.to_string();

    // Refresh the stale datetime in the cached system prompt
    {
        let now = chrono::Local::now();
        let fresh = format!(
            "## Current Date & Time\n\n{} ({})\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%Z"),
        );
        if let Some(start) = prompt.find("## Current Date & Time\n\n") {
            // Find the end of this section (next "## " heading or end of string)
            let rest = &prompt[start + 24..]; // skip past "## Current Date & Time\n\n"
            let section_end = rest
                .find("\n## ")
                .map(|i| start + 24 + i)
                .unwrap_or(prompt.len());
            prompt.replace_range(start..section_end, fresh.trim_end());
        }
    }

    if let Some(instructions) = channel_delivery_instructions(channel_name) {
        if prompt.is_empty() {
            prompt = instructions.to_string();
        } else {
            prompt = format!("{prompt}\n\n{instructions}");
        }
    }

    if !reply_target.is_empty() {
        // For most channels, `reply_target` is the address to send to (channel/room
        // ID for Slack/Discord/Matrix, peer ID for Telegram/Signal). The webhook
        // channel is the exception: its outbound JSON has both `recipient` and
        // `thread_id`, and downstream services routing through it expect the
        // *sender* as the recipient and the *thread/conversation* identifier in
        // `thread_id`. Reusing `reply_target` as `to` for webhook would strip the
        // thread context and the receiver would discard the callback.
        let delivery_hint = if channel_name.eq_ignore_ascii_case("webhook") {
            format!(
                "delivery={{\"mode\":\"announce\",\"channel\":\"{channel_name}\",\
                 \"to\":\"{sender}\",\"thread_id\":\"{reply_target}\"}}"
            )
        } else {
            format!(
                "delivery={{\"mode\":\"announce\",\"channel\":\"{channel_name}\",\
                 \"to\":\"{reply_target}\"}}"
            )
        };
        let context = format!(
            "\n\nChannel context: You are currently responding on channel={channel_name}, \
             reply_target={reply_target}, sender={sender}. \
             The sender field is the platform-specific user ID of the person who sent \
             this message. Use it to distinguish between different users. \
             When scheduling delayed messages or reminders \
             via cron_add for this conversation, use {delivery_hint} so the message \
             reaches the user."
        );
        prompt.push_str(&context);
    }

    prompt
}

pub(crate) fn replace_available_skills_section(
    base_prompt: &str,
    refreshed_skills: &str,
) -> String {
    const SKILLS_HEADER: &str = "## Available Skills\n\n";
    const SKILLS_END: &str = "</available_skills>";
    const WORKSPACE_HEADER: &str = "## Workspace\n\n";

    if let Some(start) = base_prompt.find(SKILLS_HEADER)
        && let Some(rel_end) = base_prompt[start..].find(SKILLS_END)
    {
        let end = start + rel_end + SKILLS_END.len();
        let tail = base_prompt[end..]
            .strip_prefix("\n\n")
            .unwrap_or(&base_prompt[end..]);

        let mut refreshed = String::with_capacity(
            base_prompt.len().saturating_sub(end.saturating_sub(start))
                + refreshed_skills.len()
                + 2,
        );
        refreshed.push_str(&base_prompt[..start]);
        if !refreshed_skills.is_empty() {
            refreshed.push_str(refreshed_skills);
            refreshed.push_str("\n\n");
        }
        refreshed.push_str(tail);
        return refreshed;
    }

    if refreshed_skills.is_empty() {
        return base_prompt.to_string();
    }

    if let Some(workspace_start) = base_prompt.find(WORKSPACE_HEADER) {
        let mut refreshed = String::with_capacity(base_prompt.len() + refreshed_skills.len() + 2);
        refreshed.push_str(&base_prompt[..workspace_start]);
        refreshed.push_str(refreshed_skills);
        refreshed.push_str("\n\n");
        refreshed.push_str(&base_prompt[workspace_start..]);
        return refreshed;
    }

    format!("{base_prompt}\n\n{refreshed_skills}")
}

pub(crate) fn refreshed_new_session_system_prompt(ctx: &ChannelRuntimeContext) -> String {
    let refreshed_skills = operant_runtime::skills::skills_to_prompt_with_mode(
        &operant_runtime::skills::load_skills_with_config(
            ctx.workspace_dir.as_ref(),
            ctx.prompt_config.as_ref(),
        ),
        ctx.workspace_dir.as_ref(),
        ctx.prompt_config.skills.prompt_injection_mode,
    );
    replace_available_skills_section(ctx.system_prompt.as_str(), &refreshed_skills)
}

pub(crate) fn channel_runtime_string(key: &str) -> String {
    i18n::get_required_cli_string(key)
}

pub(crate) fn channel_runtime_string_with_args(key: &str, args: &[(&str, &str)]) -> String {
    i18n::get_required_cli_string_with_args(key, args)
}

pub(crate) fn build_current_route_summary(current: &ChannelRouteSelection) -> String {
    channel_runtime_string_with_args(
        "channel-runtime-current-route",
        &[
            ("provider", current.provider.as_str()),
            ("model", current.model.as_str()),
        ],
    )
}

pub(crate) fn build_models_help_response(
    current: &ChannelRouteSelection,
    workspace_dir: &Path,
    model_routes: &[operant_config::schema::ModelRouteConfig],
) -> String {
    let mut response = String::new();
    response.push_str(&build_current_route_summary(current));
    response.push_str("\n\n");
    response.push_str(&channel_runtime_string("channel-runtime-switch-model-help"));
    response.push('\n');

    if !model_routes.is_empty() {
        response.push('\n');
        response.push_str(&channel_runtime_string(
            "channel-runtime-configured-model-routes",
        ));
        response.push('\n');
        for route in model_routes {
            let _ = writeln!(
                response,
                "  `{}` → {} ({})",
                route.hint, route.model, route.provider
            );
        }
    }

    let cached_models = load_cached_model_preview(workspace_dir, &current.provider);
    if cached_models.is_empty() {
        response.push('\n');
        response.push_str(&channel_runtime_string_with_args(
            "channel-runtime-no-cached-models",
            &[("provider", current.provider.as_str())],
        ));
        response.push('\n');
    } else {
        response.push('\n');
        response.push_str(&channel_runtime_string_with_args(
            "channel-runtime-cached-models",
            &[("count", &cached_models.len().to_string())],
        ));
        response.push('\n');
        for model in cached_models {
            let _ = writeln!(response, "- `{model}`");
        }
    }

    response
}

pub(crate) fn build_providers_help_response(current: &ChannelRouteSelection) -> String {
    let mut response = String::new();
    response.push_str(&build_current_route_summary(current));
    response.push_str("\n\n");
    response.push_str(&channel_runtime_string(
        "channel-runtime-switch-provider-help",
    ));
    response.push('\n');
    response.push_str(&channel_runtime_string(
        "channel-runtime-switch-model-command-help",
    ));
    response.push_str("\n\n");
    response.push_str(&channel_runtime_string(
        "channel-runtime-available-providers",
    ));
    response.push('\n');
    for provider in operant_providers::list_providers() {
        if provider.aliases.is_empty() {
            let _ = writeln!(response, "- {}", provider.name);
        } else {
            let aliases = provider.aliases.join(", ");
            let _ = writeln!(
                response,
                "- {} ({})",
                provider.name,
                channel_runtime_string_with_args(
                    "channel-runtime-provider-aliases",
                    &[("aliases", aliases.as_str())]
                )
            );
        }
    }
    response
}

/// Build a plain-text `/config` response for non-Slack channels.
pub(crate) fn build_config_text_response(
    current: &ChannelRouteSelection,
    _workspace_dir: &Path,
    model_routes: &[operant_config::schema::ModelRouteConfig],
) -> String {
    let mut resp = String::new();
    resp.push_str(&build_current_route_summary(current));
    resp.push_str("\n\n");
    resp.push_str(&channel_runtime_string(
        "channel-runtime-available-providers",
    ));
    resp.push('\n');
    for p in operant_providers::list_providers() {
        let _ = writeln!(resp, "- `{}`", p.name);
    }
    if !model_routes.is_empty() {
        resp.push('\n');
        resp.push_str(&channel_runtime_string(
            "channel-runtime-configured-model-routes",
        ));
        resp.push('\n');
        for route in model_routes {
            let _ = writeln!(
                resp,
                "  `{}` -> {} ({})",
                route.hint, route.model, route.provider
            );
        }
    }
    resp.push('\n');
    resp.push_str(&channel_runtime_string(
        "channel-runtime-use-models-and-model",
    ));
    resp
}

/// Build a Slack Block Kit JSON payload for the `/config` interactive UI.
pub(crate) fn build_config_block_kit(
    current: &ChannelRouteSelection,
    workspace_dir: &Path,
    model_routes: &[operant_config::schema::ModelRouteConfig],
) -> String {
    let provider_options: Vec<serde_json::Value> = operant_providers::list_providers()
        .iter()
        .map(|p| {
            serde_json::json!({
                "text": { "type": "plain_text", "text": p.display_name },
                "value": p.name
            })
        })
        .collect();

    // Build model options from model_routes + cached models.
    let mut model_options: Vec<serde_json::Value> = model_routes
        .iter()
        .map(|r| {
            let label = if r.hint.is_empty() {
                r.model.clone()
            } else {
                format!("{} ({})", r.model, r.hint)
            };
            serde_json::json!({
                "text": { "type": "plain_text", "text": label },
                "value": r.model
            })
        })
        .collect();

    let cached = load_cached_model_preview(workspace_dir, &current.provider);
    for model_id in cached {
        if !model_options.iter().any(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == model_id)
        }) {
            model_options.push(serde_json::json!({
                "text": { "type": "plain_text", "text": model_id },
                "value": model_id
            }));
        }
    }

    // If the current model is not in the list, prepend it.
    if !model_options.iter().any(|o| {
        o.get("value")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == current.model)
    }) {
        model_options.insert(
            0,
            serde_json::json!({
                "text": { "type": "plain_text", "text": &current.model },
                "value": &current.model
            }),
        );
    }

    // Find initial options matching current selection.
    let initial_provider = provider_options
        .iter()
        .find(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == current.provider)
        })
        .cloned();

    let initial_model = model_options
        .iter()
        .find(|o| {
            o.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == current.model)
        })
        .cloned();

    let mut provider_select = serde_json::json!({
        "type": "static_select",
        "action_id": "operant_config_provider",
        "placeholder": {
            "type": "plain_text",
            "text": channel_runtime_string("channel-runtime-select-provider")
        },
        "options": provider_options
    });
    if let Some(init) = initial_provider {
        provider_select["initial_option"] = init;
    }

    let mut model_select = serde_json::json!({
        "type": "static_select",
        "action_id": "operant_config_model",
        "placeholder": {
            "type": "plain_text",
            "text": channel_runtime_string("channel-runtime-select-model")
        },
        "options": model_options
    });
    if let Some(init) = initial_model {
        model_select["initial_option"] = init;
    }

    let blocks = serde_json::json!([
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": channel_runtime_string_with_args(
                    "channel-runtime-config-block-title",
                    &[
                        ("provider", current.provider.as_str()),
                        ("model", current.model.as_str()),
                    ],
                )
            }
        },
        {
            "type": "section",
            "block_id": "config_provider_block",
            "text": {
                "type": "mrkdwn",
                "text": channel_runtime_string("channel-runtime-provider-label")
            },
            "accessory": provider_select
        },
        {
            "type": "section",
            "block_id": "config_model_block",
            "text": {
                "type": "mrkdwn",
                "text": channel_runtime_string("channel-runtime-model-label")
            },
            "accessory": model_select
        }
    ]);

    blocks.to_string()
}
