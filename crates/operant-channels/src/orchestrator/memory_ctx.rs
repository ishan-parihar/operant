//! `memory_ctx` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use operant_api::session_keys::sanitize_session_key;
use operant_memory::{self, MEMORY_CONTEXT_CLOSE, MEMORY_CONTEXT_OPEN, Memory, MemoryResult};
use operant_runtime::util::truncate_with_ellipsis;
use std::collections::HashSet;

use super::*;

pub(crate) fn should_skip_memory_context_entry(key: &str, content: &str) -> bool {
    if operant_memory::is_assistant_autosave_key(key) {
        return true;
    }

    // Skip raw per-turn user messages: re-injecting them causes each
    // recalled entry to embed all prior generations, growing exponentially.
    // Consolidated knowledge is already promoted to Core/Daily entries.
    if operant_memory::is_user_autosave_key(key) {
        return true;
    }

    if operant_memory::should_skip_autosave_content(content) {
        return true;
    }

    if key.trim().to_ascii_lowercase().ends_with("_history") {
        return true;
    }

    // Skip entries containing image markers to prevent duplication.
    // When auto_save stores a photo message to memory, a subsequent
    // memory recall on the same turn would surface the marker again,
    // causing two identical image blocks in the provider request.
    if content.contains("[IMAGE:") {
        return true;
    }

    // Skip entries containing tool_result blocks. After a daemon restart
    // these can be recalled from SQLite and injected as memory context,
    // presenting the LLM with a `<tool_result>` without a preceding
    // `<tool_call>` and triggering hallucinated output.
    if content.contains("<tool_result") {
        return true;
    }

    content.chars().count() > MEMORY_CONTEXT_MAX_CHARS
}

pub(crate) async fn build_memory_context(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
    session_id: Option<&str>,
) -> String {
    build_memory_context_for_sessions(mem, user_msg, min_relevance_score, &[session_id]).await
}

pub(crate) async fn build_memory_context_for_sessions(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
    session_ids: &[Option<&str>],
) -> String {
    let mut entries = Vec::new();
    let mut seen_keys = HashSet::new();

    match session_ids {
        [] => {}
        [session_id] => {
            let recalled = mem.recall(user_msg, 5, *session_id, None, None).await;
            append_recalled_memory_entries(&mut entries, &mut seen_keys, recalled);
        }
        [first_session_id, second_session_id] => {
            let (first_entries, second_entries) = tokio::join!(
                mem.recall(user_msg, 5, *first_session_id, None, None),
                mem.recall(user_msg, 5, *second_session_id, None, None)
            );
            append_recalled_memory_entries(&mut entries, &mut seen_keys, first_entries);
            append_recalled_memory_entries(&mut entries, &mut seen_keys, second_entries);
        }
        _ => {
            for session_id in session_ids {
                let recalled = mem.recall(user_msg, 5, *session_id, None, None).await;
                append_recalled_memory_entries(&mut entries, &mut seen_keys, recalled);
            }
        }
    }

    format_memory_context(&entries, min_relevance_score)
}

pub(crate) fn append_recalled_memory_entries(
    entries: &mut Vec<operant_memory::MemoryEntry>,
    seen_keys: &mut HashSet<String>,
    recalled: MemoryResult<Vec<operant_memory::MemoryEntry>>,
) {
    if let Ok(recalled) = recalled {
        for entry in recalled {
            if seen_keys.insert(entry.key.clone()) {
                entries.push(entry);
            }
        }
    }
}

pub(crate) fn format_memory_context(
    entries: &[operant_memory::MemoryEntry],
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();

    let mut included = 0usize;
    let mut used_chars = 0usize;

    for entry in entries.iter().filter(|e| match e.score {
        Some(score) => score >= min_relevance_score,
        None => true, // keep entries without a score (e.g. non-vector backends)
    }) {
        if included >= MEMORY_CONTEXT_MAX_ENTRIES {
            break;
        }

        if should_skip_memory_context_entry(&entry.key, &entry.content) {
            continue;
        }

        let content = if entry.content.chars().count() > MEMORY_CONTEXT_ENTRY_MAX_CHARS {
            truncate_with_ellipsis(&entry.content, MEMORY_CONTEXT_ENTRY_MAX_CHARS)
        } else {
            entry.content.clone()
        };

        let line = format!("- {}: {}\n", entry.key, content);
        let line_chars = line.chars().count();
        if used_chars + line_chars > MEMORY_CONTEXT_MAX_CHARS {
            break;
        }

        if included == 0 {
            context.push_str(MEMORY_CONTEXT_OPEN);
            context.push('\n');
        }

        context.push_str(&line);
        used_chars += line_chars;
        included += 1;
    }

    if included > 0 {
        context.push_str(MEMORY_CONTEXT_CLOSE);
        context.push_str("\n\n");
    }

    context
}

pub(crate) fn is_group_reply_target(reply_target: &str) -> bool {
    reply_target.contains("@g.us") || reply_target.starts_with("group:")
}

pub(crate) fn sender_memory_session_ids(
    msg: &operant_api::channel::ChannelMessage,
    history_key: &str,
) -> Vec<String> {
    // Match the sanitized form persisted by memory backend migrations.
    let sanitized_sender = sanitize_session_key(&msg.sender);
    if is_group_reply_target(&msg.reply_target) {
        vec![sanitized_sender]
    } else {
        vec![history_key.to_string(), sanitized_sender]
    }
}

/// Extract a compact summary of tool interactions from history messages added
/// during `run_tool_call_loop`. Scans assistant messages for `<tool_call>` tags
/// or native tool-call JSON to collect tool names used.
/// Returns an empty string when no tools were invoked.
#[cfg(test)]
pub(crate) fn extract_tool_context_summary(history: &[operant_providers::ChatMessage], start_index: usize) -> String {
    fn push_unique_tool_name(tool_names: &mut Vec<String>, name: &str) {
        let candidate = name.trim();
        if candidate.is_empty() {
            return;
        }
        if !tool_names.iter().any(|existing| existing == candidate) {
            tool_names.push(candidate.to_string());
        }
    }

    fn collect_tool_names_from_tool_call_tags(content: &str, tool_names: &mut Vec<String>) {
        const TAG_PAIRS: [(&str, &str); 4] = [
            ("<tool_call>", "</tool_call>"),
            ("<toolcall>", "</toolcall>"),
            ("<tool-call>", "</tool-call>"),
            ("<invoke>", "</invoke>"),
        ];

        for (open_tag, close_tag) in TAG_PAIRS {
            for segment in content.split(open_tag) {
                if let Some(json_end) = segment.find(close_tag) {
                    let json_str = segment[..json_end].trim();
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str)
                        && let Some(name) = val.get("name").and_then(|n| n.as_str())
                    {
                        push_unique_tool_name(tool_names, name);
                    }
                }
            }
        }
    }

    fn collect_tool_names_from_native_json(content: &str, tool_names: &mut Vec<String>) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content)
            && let Some(calls) = val.get("tool_calls").and_then(|c| c.as_array())
        {
            for call in calls {
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .or_else(|| call.get("name").and_then(|n| n.as_str()));
                if let Some(name) = name {
                    push_unique_tool_name(tool_names, name);
                }
            }
        }
    }

    fn collect_tool_names_from_tool_results(content: &str, tool_names: &mut Vec<String>) {
        let marker = "<tool_result name=\"";
        let mut remaining = content;
        while let Some(start) = remaining.find(marker) {
            let name_start = start + marker.len();
            let after_name_start = &remaining[name_start..];
            if let Some(name_end) = after_name_start.find('"') {
                let name = &after_name_start[..name_end];
                push_unique_tool_name(tool_names, name);
                remaining = &after_name_start[name_end + 1..];
            } else {
                break;
            }
        }
    }

    let mut tool_names: Vec<String> = Vec::new();

    for msg in history.iter().skip(start_index) {
        match msg.role.as_str() {
            "assistant" => {
                collect_tool_names_from_tool_call_tags(&msg.content, &mut tool_names);
                collect_tool_names_from_native_json(&msg.content, &mut tool_names);
            }
            "user" => {
                // Prompt-mode tool calls are always followed by [Tool results] entries
                // containing `<tool_result name="...">` tags with canonical tool names.
                collect_tool_names_from_tool_results(&msg.content, &mut tool_names);
            }
            _ => {}
        }
    }

    if tool_names.is_empty() {
        return String::new();
    }

    format!("[Used tools: {}]", tool_names.join(", "))
}
