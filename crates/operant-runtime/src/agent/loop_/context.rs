//! `context` — extracted verbatim from the former loop_.rs monolith.
//! Re-exported from `loop_` so every import path is unchanged.

/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
use super::*;

pub fn retain_registered_tool_descriptions(
    tool_descs: &mut Vec<(&str, &str)>,
    tools_registry: &[Box<dyn Tool>],
) {
    let registered_tool_names: HashSet<&str> =
        tools_registry.iter().map(|tool| tool.name()).collect();
    tool_descs.retain(|(name, _)| registered_tool_names.contains(name));
}

// Re-export tool call parsing from the standalone parser crate.
pub use operant_tool_call_parser::{
    ParsedToolCall, build_native_assistant_history_from_parsed_calls,
    canonicalize_json_for_tool_signature, detect_tool_call_parse_issue, parse_tool_calls,
    strip_think_tags, strip_tool_result_blocks,
};

/// Default trigger for auto-compaction when non-system message count exceeds this threshold.
/// Prefer passing the config-driven value via `run_tool_call_loop`; this constant is only
/// used when callers omit the parameter.
/// Minimum interval between progress sends to avoid flooding the draft channel.
pub const PROGRESS_MIN_INTERVAL_MS: u64 = 500;

/// Delta sent from the agent loop to the channel's draft updater.
/// Append-only — no clear/reset variant exists by design.
#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// Response text to append to the message buffer.
    Text(String),
    /// Ephemeral tool progress (not part of the response body).
    Status(String),
}

/// Backwards-compatible alias while callers are migrated.
pub type DraftEvent = StreamDelta;

pub use operant_api::TOOL_CHOICE_OVERRIDE;

/// Convert a tool registry to OpenAI function-calling format for native tool support.
#[cfg(test)]
pub fn tools_to_openai_format(tools_registry: &[Box<dyn Tool>]) -> Vec<serde_json::Value> {
    tools_registry
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema()
                }
            })
        })
        .collect()
}

pub fn autosave_memory_key(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}

/// Build context preamble by searching memory for relevant entries.
/// Entries with a hybrid score below `min_relevance_score` are dropped to
/// prevent unrelated memories from bleeding into the conversation.
/// Core memories are exempt from time decay (evergreen).
///
/// `exclude_conversation` skips `MemoryCategory::Conversation` entries
/// regardless of their key shape. Set to `true` for autonomous/scheduled
/// runs (cron, daemon heartbeat) so chat memory cannot leak into prompts
/// the user did not initiate. See #5415 / #5456.
pub async fn build_context(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
    session_id: Option<&str>,
    exclude_conversation: bool,
) -> String {
    let mut context = String::new();

    // Pull relevant memories for this message
    if let Ok(mut entries) = mem.recall(user_msg, 5, session_id, None, None).await {
        // Apply time decay: older non-Core memories score lower
        decay::apply_time_decay(&mut entries, decay::DEFAULT_HALF_LIFE_DAYS);

        let relevant: Vec<_> = entries
            .iter()
            .filter(|e| match e.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .collect();

        if !relevant.is_empty() {
            let mut included = false;
            for entry in &relevant {
                // Scheduled (cron / heartbeat) runs must not see chat-origin
                // memories. The autosave-key checks below catch the agent's
                // own autosaves but miss Conversation entries written by
                // channel handlers (Discord, gateway, WhatsApp, …) under
                // their own keys. See #5415 / #5456.
                if exclude_conversation && matches!(entry.category, MemoryCategory::Conversation) {
                    continue;
                }
                if operant_memory::is_assistant_autosave_key(&entry.key) {
                    continue;
                }
                // Skip raw per-turn user messages: re-injecting them causes each
                // recalled entry to embed all prior generations, growing exponentially.
                // Consolidated knowledge is already promoted to Core/Daily entries.
                if operant_memory::is_user_autosave_key(&entry.key) {
                    continue;
                }
                if operant_memory::should_skip_autosave_content(&entry.content) {
                    continue;
                }
                // Skip entries containing tool_result blocks — they can leak
                // stale tool output from previous heartbeat ticks into new
                // sessions, presenting the LLM with orphan tool_result data.
                if entry.content.contains("<tool_result") {
                    continue;
                }
                if !included {
                    context.push_str(MEMORY_CONTEXT_OPEN);
                    context.push('\n');
                    included = true;
                }
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            if included {
                context.push_str(MEMORY_CONTEXT_CLOSE);
                context.push_str("\n\n");
            }
        }
    }

    context
}

/// Build hardware datasheet context from RAG when peripherals are enabled.
/// Includes pin-alias lookup (e.g. "red_led" → 13) when query matches, plus retrieved chunks.
pub fn build_hardware_context(
    rag: &crate::rag::HardwareRag,
    user_msg: &str,
    boards: &[String],
    chunk_limit: usize,
) -> String {
    if rag.is_empty() || boards.is_empty() {
        return String::new();
    }

    let mut context = String::new();

    // Pin aliases: when user says "red led", inject "red_led: 13" for matching boards
    let pin_ctx = rag.pin_alias_context(user_msg, boards);
    if !pin_ctx.is_empty() {
        context.push_str(&pin_ctx);
    }

    let chunks = rag.retrieve(user_msg, boards, chunk_limit);
    if chunks.is_empty() && pin_ctx.is_empty() {
        return String::new();
    }

    if !chunks.is_empty() {
        context.push_str("[Hardware documentation]\n");
    }
    for chunk in chunks {
        let board_tag = chunk.board.as_deref().unwrap_or("generic");
        let _ = writeln!(
            context,
            "--- {} ({}) ---\n{}\n",
            chunk.source, board_tag, chunk.content
        );
    }
    context.push('\n');
    context
}

// Tool execution moved to `super::tool_execution`.
pub use crate::agent::tool_execution::{
    ToolExecutionOutcome, execute_tools_parallel, execute_tools_sequential,
    should_execute_tools_in_parallel,
};

#[expect(
    clippy::unwrap_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Build assistant history entry in JSON format for native tool-call APIs.
/// `convert_messages` in the OpenRouter provider parses this JSON to reconstruct
/// the proper `NativeMessage` with structured `tool_calls`.
pub fn build_native_assistant_history(
    text: &str,
    tool_calls: &[ToolCall],
    reasoning_content: Option<&str>,
) -> String {
    let calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })
        })
        .collect();

    let content = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text.trim().to_string())
    };

    let mut obj = serde_json::json!({
        "content": content,
        "tool_calls": calls_json,
    });

    if let Some(rc) = reasoning_content {
        obj.as_object_mut().unwrap().insert(
            "reasoning_content".to_string(),
            serde_json::Value::String(rc.to_string()),
        );
    }

    obj.to_string()
}

pub fn resolve_display_text(
    response_text: &str,
    parsed_text: &str,
    has_tool_calls: bool,
    has_native_tool_calls: bool,
) -> String {
    if has_tool_calls {
        if !parsed_text.is_empty() {
            return parsed_text.to_string();
        }
        if has_native_tool_calls {
            return response_text.to_string();
        }
        return String::new();
    }

    if parsed_text.is_empty() {
        response_text.to_string()
    } else {
        parsed_text.to_string()
    }
}

#[derive(Debug)]
pub struct ToolLoopCancelled;

impl std::fmt::Display for ToolLoopCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tool loop cancelled")
    }
}

impl std::error::Error for ToolLoopCancelled {}
