//! `commands` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use operant_providers::{self};
use std::sync::Arc;

use super::*;

/// Returns `true` when `content` is a `/stop` command (with optional `@botname` suffix).
/// Not gated on channel type — all non-CLI channels support `/stop`.
pub(crate) fn is_stop_command(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return false;
    }
    let cmd = trimmed.split_whitespace().next().unwrap_or("");
    let base = cmd.split('@').next().unwrap_or(cmd);
    base.eq_ignore_ascii_case("/stop")
}

pub(crate) fn supports_runtime_model_switch(channel_name: &str) -> bool {
    matches!(channel_name, "telegram" | "discord" | "matrix" | "slack")
}

pub(crate) fn is_matrix_channel_name(channel_name: &str) -> bool {
    channel_name == "matrix" || channel_name.starts_with("matrix:")
}

pub(crate) fn parse_runtime_command(
    channel_name: &str,
    content: &str,
) -> Option<ChannelRuntimeCommand> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let command_token = parts.next()?;
    let base_command = command_token
        .split('@')
        .next()
        .unwrap_or(command_token)
        .to_ascii_lowercase();

    match base_command.as_str() {
        // `/new` is available on every channel — no model-switch gate.
        "/new" => Some(ChannelRuntimeCommand::NewSession),
        // Model/provider switching is channel-gated.
        "/models" if supports_runtime_model_switch(channel_name) => {
            if let Some(provider) = parts.next() {
                Some(ChannelRuntimeCommand::SetProvider(
                    provider.trim().to_string(),
                ))
            } else {
                Some(ChannelRuntimeCommand::ShowProviders)
            }
        }
        "/model" if supports_runtime_model_switch(channel_name) => {
            let model = parts.collect::<Vec<_>>().join(" ").trim().to_string();
            if model.is_empty() {
                Some(ChannelRuntimeCommand::ShowModel)
            } else {
                Some(ChannelRuntimeCommand::SetModel(model))
            }
        }
        "/config" if supports_runtime_model_switch(channel_name) => {
            Some(ChannelRuntimeCommand::ShowConfig)
        }
        _ => None,
    }
}

pub(crate) async fn handle_runtime_command_if_needed(
    ctx: &ChannelRuntimeContext,
    msg: &operant_api::channel::ChannelMessage,
    target_channel: Option<&Arc<dyn Channel>>,
) -> bool {
    let Some(command) = parse_runtime_command(&msg.channel, &msg.content) else {
        return false;
    };

    let Some(channel) = target_channel else {
        return true;
    };

    let sender_key = conversation_history_key(msg);
    let mut current = get_route_selection(ctx, &sender_key);

    let response = match command {
        ChannelRuntimeCommand::ShowProviders => build_providers_help_response(&current),
        ChannelRuntimeCommand::SetProvider(raw_provider) => {
            match resolve_provider_alias(&raw_provider) {
                Some(provider_name) => {
                    match get_or_create_provider(ctx, &provider_name, None).await {
                        Ok(_) => {
                            if provider_name != current.provider {
                                current.provider = provider_name.clone();
                                set_route_selection(ctx, &sender_key, current.clone());
                            }

                            channel_runtime_string_with_args(
                                "channel-runtime-provider-switched",
                                &[
                                    ("provider", provider_name.as_str()),
                                    ("model", current.model.as_str()),
                                ],
                            )
                        }
                        Err(err) => {
                            let safe_err = operant_providers::sanitize_api_error(&err.to_string());
                            channel_runtime_string_with_args(
                                "channel-runtime-provider-init-failed",
                                &[
                                    ("provider", provider_name.as_str()),
                                    ("details", safe_err.as_str()),
                                ],
                            )
                        }
                    }
                }
                None => channel_runtime_string_with_args(
                    "channel-runtime-unknown-provider",
                    &[("provider", raw_provider.as_str())],
                ),
            }
        }
        ChannelRuntimeCommand::ShowModel => {
            build_models_help_response(&current, ctx.workspace_dir.as_path(), &ctx.model_routes)
        }
        ChannelRuntimeCommand::SetModel(raw_model) => {
            let model = raw_model.trim().trim_matches('`').to_string();
            if model.is_empty() {
                channel_runtime_string("channel-runtime-model-id-empty")
            } else {
                // Resolve provider+model from model_routes (match by model name or hint)
                if let Some(route) = ctx.model_routes.iter().find(|r| {
                    r.model.eq_ignore_ascii_case(&model) || r.hint.eq_ignore_ascii_case(&model)
                }) {
                    current.provider = route.provider.clone();
                    current.model = route.model.clone();
                    current.api_key = route.api_key.clone();
                } else {
                    current.model = model.clone();
                }
                set_route_selection(ctx, &sender_key, current.clone());

                channel_runtime_string_with_args(
                    "channel-runtime-model-switched",
                    &[
                        ("model", current.model.as_str()),
                        ("provider", current.provider.as_str()),
                    ],
                )
            }
        }
        ChannelRuntimeCommand::ShowConfig => {
            if msg.channel == "slack" {
                let blocks_json = build_config_block_kit(
                    &current,
                    ctx.workspace_dir.as_path(),
                    &ctx.model_routes,
                );
                // Use a magic prefix so SlackChannel::send() can detect Block Kit JSON.
                format!("__OPERANT_BLOCK_KIT__{blocks_json}")
            } else {
                build_config_text_response(&current, ctx.workspace_dir.as_path(), &ctx.model_routes)
            }
        }
        ChannelRuntimeCommand::NewSession => {
            clear_sender_history(ctx, &sender_key);
            if let Some(ref store) = ctx.session_store
                && let Err(e) = store.delete_session(&sender_key)
            {
                tracing::warn!("Failed to delete persisted session for {sender_key}: {e}");
            }
            mark_sender_for_new_session(ctx, &sender_key);
            channel_runtime_string("channel-runtime-new-session")
        }
    };

    if let Err(err) = channel
        .send(&SendMessage::new(response, &msg.reply_target).in_thread(msg.thread_ts.clone()))
        .await
    {
        tracing::warn!(
            "Failed to send runtime command response on {}: {err}",
            channel.name()
        );
    }

    true
}
