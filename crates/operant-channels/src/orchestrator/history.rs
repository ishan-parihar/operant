//! `history` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use operant_api::session_keys::sanitize_session_key;
use operant_providers::{self, ChatMessage};
use operant_runtime::util::truncate_with_ellipsis;

use super::*;

pub(crate) fn conversation_memory_key(msg: &operant_api::channel::ChannelMessage) -> String {
    // Include thread_ts for per-topic memory isolation in forum groups
    let raw = match &msg.thread_ts {
        Some(tid) => format!("{}_{}_{}_{}", msg.channel, tid, msg.sender, msg.id),
        None => format!("{}_{}_{}", msg.channel, msg.sender, msg.id),
    };
    sanitize_session_key(&raw)
}

pub fn conversation_history_key(msg: &operant_api::channel::ChannelMessage) -> String {
    // Sanitize so the runtime HashMap key matches `SessionStore::list_sessions`
    // after a restart; otherwise hydration loads sessions under the on-disk
    // (sanitized) name while lookup keeps producing the un-sanitized form.
    let raw = match &msg.thread_ts {
        Some(tid) => format!(
            "{}_{}_{}_{}",
            msg.channel, msg.reply_target, tid, msg.sender
        ),
        None => format!("{}_{}_{}", msg.channel, msg.reply_target, msg.sender),
    };
    sanitize_session_key(&raw)
}

pub(crate) fn followup_thread_id(msg: &operant_api::channel::ChannelMessage) -> Option<String> {
    if is_matrix_channel_name(&msg.channel) {
        msg.thread_ts.clone()
    } else {
        msg.thread_ts.clone().or_else(|| Some(msg.id.clone()))
    }
}

pub(crate) fn interruption_scope_key(msg: &operant_api::channel::ChannelMessage) -> String {
    match &msg.interruption_scope_id {
        Some(scope) => format!(
            "{}_{}_{}_{}",
            msg.channel, msg.reply_target, msg.sender, scope
        ),
        None => format!("{}_{}_{}", msg.channel, msg.reply_target, msg.sender),
    }
}

pub(crate) fn normalize_cached_channel_turns(turns: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut normalized = Vec::with_capacity(turns.len());
    let mut expecting_user = true;

    for turn in turns {
        match (expecting_user, turn.role.as_str()) {
            // Pass through tool-role messages preserved by
            // keep_tool_context_turns (#4827).  After a tool result the
            // next expected message is an assistant response, same as
            // after a user message.
            (_, "tool") | (true, "user") => {
                normalized.push(turn);
                expecting_user = false;
            }
            (false, "assistant") => {
                normalized.push(turn);
                expecting_user = true;
            }
            // Interrupted channel turns can produce consecutive user messages
            // (no assistant persisted yet). Merge instead of dropping.
            (false, "user") | (true, "assistant") => {
                if let Some(last_turn) = normalized.last_mut()
                    && !turn.content.is_empty()
                {
                    if !last_turn.content.is_empty() {
                        last_turn.content.push_str("\n\n");
                    }
                    last_turn.content.push_str(&turn.content);
                }
            }
            _ => {}
        }
    }

    normalized
}

#[expect(clippy::unwrap_used, reason = "infallible once-init / static init")]
/// Remove `<tool_result …>…</tool_result>` blocks (and a leading `[Tool results]`
/// header, if present) from a conversation-history entry so that stale tool
/// output is never presented to the LLM without the corresponding `<tool_call>`.
pub(crate) fn strip_tool_result_content(text: &str) -> String {
    static TOOL_RESULT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?s)<tool_result[^>]*>.*?</tool_result>").unwrap()
    });

    let cleaned = TOOL_RESULT_RE.replace_all(text, "");
    let cleaned = cleaned.trim();

    // If the only remaining content is the header, drop it entirely.
    if cleaned == "[Tool results]" || cleaned.is_empty() {
        return String::new();
    }

    cleaned.to_string()
}

/// Remove a leading `[Used tools: ...]` line from a cached assistant turn.
///
/// The tool-context summary is prepended to history entries so the LLM retains
/// awareness of prior tool usage. However, when these entries are loaded back
/// into the LLM context, the bracket-format leaks into generated output and
/// gets forwarded to end users as-is (bug #4400). Stripping the prefix on
/// reload prevents the model from learning and reproducing this internal format.
pub(crate) fn strip_tool_summary_prefix(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("[Used tools:") {
        // Find the closing bracket, then skip it and any leading newline(s).
        if let Some(bracket_end) = rest.find(']') {
            let after_bracket = &rest[bracket_end + 1..];
            let trimmed = after_bracket.trim_start_matches('\n');
            if trimmed.is_empty() {
                return String::new();
            }
            return trimmed.to_string();
        }
    }
    text.to_string()
}

pub(crate) fn clear_sender_history(ctx: &ChannelRuntimeContext, sender_key: &str) {
    ctx.conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop(sender_key);
}

pub(crate) fn mark_sender_for_new_session(ctx: &ChannelRuntimeContext, sender_key: &str) {
    ctx.pending_new_sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(sender_key.to_string());
}

pub(crate) fn take_pending_new_session(ctx: &ChannelRuntimeContext, sender_key: &str) -> bool {
    ctx.pending_new_sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(sender_key)
}

pub(crate) fn compact_sender_history(ctx: &ChannelRuntimeContext, sender_key: &str) -> bool {
    let mut histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let Some(turns) = histories.get_mut(sender_key) else {
        return false;
    };

    if turns.is_empty() {
        return false;
    }

    let keep_from = turns
        .len()
        .saturating_sub(CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
    let mut compacted = normalize_cached_channel_turns(turns[keep_from..].to_vec());

    for turn in &mut compacted {
        if turn.content.chars().count() > CHANNEL_HISTORY_COMPACT_CONTENT_CHARS {
            turn.content =
                truncate_with_ellipsis(&turn.content, CHANNEL_HISTORY_COMPACT_CONTENT_CHARS);
        }
    }

    if compacted.is_empty() {
        turns.clear();
        return false;
    }

    *turns = compacted;
    true
}

/// Proactively trim conversation turns so that the total estimated character
/// count stays within [`PROACTIVE_CONTEXT_BUDGET_CHARS`].  Drops the oldest
/// turns first, but always preserves the most recent turn (the current user
/// message).  Returns the number of turns dropped.
pub(crate) fn proactive_trim_turns(turns: &mut Vec<ChatMessage>, budget: usize) -> usize {
    let total_chars: usize = turns.iter().map(|t| t.content.chars().count()).sum();
    if total_chars <= budget || turns.len() <= 1 {
        return 0;
    }

    let mut excess = total_chars.saturating_sub(budget);
    let mut drop_count = 0;

    // Walk from the oldest turn forward, but never drop the very last turn.
    while excess > 0 && drop_count < turns.len().saturating_sub(1) {
        excess = excess.saturating_sub(turns[drop_count].content.chars().count());
        drop_count += 1;
    }

    if drop_count > 0 {
        turns.drain(..drop_count);
    }
    drop_count
}

pub(crate) fn append_sender_turn(ctx: &ChannelRuntimeContext, sender_key: &str, turn: ChatMessage) {
    // Persist to JSONL before adding to in-memory history.
    if let Some(ref store) = ctx.session_store
        && let Err(e) = store.append(sender_key, &turn)
    {
        tracing::warn!("Failed to persist session turn: {e}");
    }

    // Use the user-configured max_history_messages (fall back to
    // MAX_CHANNEL_HISTORY when the config value is 0 or absent).
    let max_history = {
        let configured = ctx.prompt_config.agent.max_history_messages;
        if configured > 0 {
            configured
        } else {
            MAX_CHANNEL_HISTORY
        }
    };

    let mut histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories.get_or_insert_mut(sender_key.to_string(), Vec::new);
    turns.push(turn);
    while turns.len() > max_history {
        turns.remove(0);
    }
}

/// Extract tool-call (assistant with tool_call content) and tool-result
/// messages from the current turn in the LLM history, excluding the final
/// assistant text response.  "Current turn" = everything after the last
/// user-role message.
pub(crate) fn extract_current_turn_tool_messages(history: &[ChatMessage]) -> Vec<ChatMessage> {
    // Find the index of the last user message — tool messages for the
    // current turn come after it.
    let last_user_idx = history.iter().rposition(|m| m.role == "user").unwrap_or(0);

    let tail = &history[last_user_idx + 1..];
    if tail.is_empty() {
        return Vec::new();
    }

    // Everything except the very last assistant message (which is the
    // final text response that gets stored separately).
    let end = if tail.last().is_some_and(|m| m.role == "assistant") {
        tail.len() - 1
    } else {
        tail.len()
    };

    tail[..end]
        .iter()
        .filter(|m| m.role == "assistant" || m.role == "tool")
        .cloned()
        .collect()
}

/// Remove tool-role and intermediate assistant tool-call messages from
/// conversation turns older than the most recent `keep_turns` user→assistant
/// exchanges.  This prevents unbounded history growth while preserving
/// tool context for the N most recent turns.
pub(crate) fn strip_old_tool_context(
    ctx: &ChannelRuntimeContext,
    sender_key: &str,
    keep_turns: usize,
) {
    let mut histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let Some(turns) = histories.get_mut(sender_key) else {
        return;
    };

    // Walk backwards to find the boundary: count user messages to
    // identify which turns are "recent" (protected from stripping).
    let mut user_count = 0;
    let mut protect_from = turns.len();
    for (i, turn) in turns.iter().enumerate().rev() {
        if turn.role == "user" {
            user_count += 1;
            if user_count > keep_turns {
                // Everything before this index is old enough to strip.
                protect_from = i + 1; // protect from next message onward
                break;
            }
        }
    }

    // Remove tool and intermediate assistant messages before the boundary.
    // An "intermediate assistant" is one whose content looks like a tool
    // call (contains `<tool_call>` or starts with `{\"tool_call`).
    let mut i = 0;
    while i < protect_from && i < turns.len() {
        let dominated = turns[i].role == "tool"
            || (turns[i].role == "assistant" && is_tool_call_content(&turns[i].content));
        if dominated {
            turns.remove(i);
            // Adjust boundary since we removed an element.
            protect_from = protect_from.saturating_sub(1);
        } else {
            i += 1;
        }
    }
}

/// Heuristic: does this assistant message content represent a tool call
/// rather than a final text response?
pub(crate) fn is_tool_call_content(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.contains("<tool_call>")
        || trimmed.starts_with("{\"tool_call\"")
        || trimmed.starts_with("{\"name\"")
}

pub(crate) fn rollback_orphan_user_turn(
    ctx: &ChannelRuntimeContext,
    sender_key: &str,
    expected_content: &str,
) -> bool {
    let mut histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(turns) = histories.get_mut(sender_key) else {
        return false;
    };

    let should_pop = turns
        .last()
        .is_some_and(|turn| turn.role == "user" && turn.content == expected_content);
    if !should_pop {
        return false;
    }

    turns.pop();
    if turns.is_empty() {
        histories.pop(sender_key);
    }

    // Also remove the orphan turn from the persisted JSONL session store so
    // it doesn't resurface after a daemon restart (fixes #3674).
    if let Some(ref store) = ctx.session_store
        && let Err(e) = store.remove_last(sender_key)
    {
        tracing::warn!("Failed to rollback session store entry: {e}");
    }

    true
}

pub(crate) fn should_rollback_failed_user_turn(error: &anyhow::Error) -> bool {
    if error
        .downcast_ref::<operant_providers::ProviderCapabilityError>()
        .is_some_and(|capability| capability.capability.eq_ignore_ascii_case("vision"))
    {
        return true;
    }

    operant_providers::reliable::is_non_retryable(error)
}

pub(crate) fn is_context_window_overflow_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    [
        "exceeds the context window",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "too many tokens",
        "token limit exceeded",
        "prompt is too long",
        "input is too long",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}
