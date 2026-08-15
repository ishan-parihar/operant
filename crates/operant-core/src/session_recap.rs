//! Session recap — hermes `hermes_cli/session_recap.py` parity.
//!
//! Summarizes what happened recently in a session so users juggling multiple
//! sessions can re-orient quickly (Claude Code `/recap` v2.1.114 parity).
//!
//! Design constraints inherited from hermes:
//! - **Pure local computation** over the persisted message history. No LLM
//!   call, no auxiliary model, no prompt-cache invalidation. A recap is
//!   instant and free.
//! - **Works everywhere** — CLI and every gateway platform can call
//!   [`build_recap`] because it only needs the message rows.
//! - **Tailored to operant's tool vocabulary** — the recap surfaces which
//!   classes of work were most active.

use std::collections::BTreeMap;

use crate::database::MessageData;

/// How many recent messages we consider "recent activity".
const RECENT_WINDOW: usize = 20;
/// Characters of the latest user prompt to show.
const PROMPT_PREVIEW_CHARS: usize = 140;
/// Characters of the latest assistant text to show.
const ASSISTANT_PREVIEW_CHARS: usize = 200;
/// How many recently-touched files to list.
const MAX_FILES_LISTED: usize = 5;
/// Tool names that identify a file action and the argument key holding the
/// path (hermes `_FILE_EDIT_TOOLS` parity, adapted to operant tool names).
const FILE_EDIT_TOOLS: &[(&str, &str)] = &[
    ("write_file", "path"),
    ("patch", "path"),
    ("read_file", "path"),
    ("file_write", "path"),
    ("file_patch", "path"),
    ("file_read", "path"),
    ("skill_manage", "file_path"),
    ("skill_view", "file_path"),
];

/// Build a multi-line recap of recent session activity.
///
/// `messages` is the full persisted history (newest last). Output is plain
/// text designed to render well in a terminal and a gateway message bubble.
pub fn build_recap(
    messages: &[MessageData],
    session_title: Option<&str>,
    session_id: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    let mut header_bits = vec!["Session recap".to_string()];
    if let Some(title) = session_title.filter(|t| !t.is_empty()) {
        header_bits.push(format!("— {title}"));
    } else if let Some(id) = session_id.filter(|i| !i.is_empty()) {
        header_bits.push(format!("— {}", short_id(id)));
    }
    lines.push(header_bits.join(" "));

    if messages.is_empty() {
        lines.push("  (nothing to recap — no messages yet)".to_string());
        return lines.join("\n");
    }

    let (total_users, total_assistants, total_tools) = count_roles(messages);
    let window = messages
        .iter()
        .rev()
        .take(RECENT_WINDOW)
        .cloned()
        .collect::<Vec<_>>();
    let (win_users, win_assistants, win_tools) = count_roles(&window);

    let mut scope = format!(
        "  Recent: {} user turn{} / {} assistant repl{}",
        win_users,
        plural(win_users, "s"),
        win_assistants,
        if win_assistants == 1 { "y" } else { "ies" },
    );
    if (total_users, total_assistants) != (win_users, win_assistants) {
        scope.push_str(&format!(" (of {total_users}/{total_assistants} total)"));
    }
    scope.push_str(&format!(
        ", {} tool result{} in window / {} total",
        win_tools,
        plural(win_tools, "s"),
        total_tools
    ));
    lines.push(scope);

    let (tool_counts, files) = summarise_tool_activity(&window);
    if !tool_counts.is_empty() {
        let mut top: Vec<String> = tool_counts
            .iter()
            .take(5)
            .map(|(name, count)| format!("{name}×{count}"))
            .collect();
        let extra = tool_counts.len().saturating_sub(5);
        if extra > 0 {
            top.push(format!("(+{extra} more)"));
        }
        lines.push(format!("  Tools used: {}", top.join(", ")));
    }
    if !files.is_empty() {
        let shown: Vec<String> = files
            .iter()
            .take(MAX_FILES_LISTED)
            .map(|p| display_path(p))
            .collect();
        let mut line = format!("  Files touched: {}", shown.join(", "));
        let extra = files.len().saturating_sub(MAX_FILES_LISTED);
        if extra > 0 {
            line.push_str(&format!(" (+{extra} more)"));
        }
        lines.push(line);
    }

    if let Some(prompt) = latest_user_prompt(messages) {
        lines.push(format!(
            "  Latest prompt: {}",
            preview(&prompt, PROMPT_PREVIEW_CHARS)
        ));
    }
    if let Some(reply) = latest_assistant_text(messages) {
        lines.push(format!(
            "  Latest reply: {}",
            preview(&reply, ASSISTANT_PREVIEW_CHARS)
        ));
    }

    lines.join("\n")
}

/// Count visible user/assistant turns and tool results.
fn count_roles(messages: &[MessageData]) -> (usize, usize, usize) {
    let mut users = 0;
    let mut assistants = 0;
    let mut tools = 0;
    for m in messages {
        match m.role.as_str() {
            "user" => {
                if m.content.as_deref().is_some_and(|c| !c.trim().is_empty()) {
                    users += 1;
                }
            }
            "assistant" => {
                if m.content.as_deref().is_some_and(|c| !c.trim().is_empty()) {
                    assistants += 1;
                }
            }
            "tool" => tools += 1,
            _ => {}
        }
    }
    (users, assistants, tools)
}

/// Count tool calls by name (from `tool_calls` JSON) and collect
/// recently-touched file paths. Returns (sorted name→count, unique paths).
fn summarise_tool_activity(messages: &[MessageData]) -> (Vec<(String, usize)>, Vec<String>) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut files: Vec<String> = Vec::new();

    for m in messages {
        // Assistant messages carry `tool_calls` JSON
        // `[{"function":{"name":"...","arguments":"{...}"}}]`.
        if let Some(raw) = m.tool_calls.as_deref()
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(raw)
            && let Some(arr) = value.as_array()
        {
            for call in arr {
                if let Some(name) = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                {
                    *counts.entry(name.to_string()).or_insert(0) += 1;
                    if let Some(path) = tool_path(name, call)
                        && !files.contains(&path)
                    {
                        files.push(path);
                    }
                }
            }
        }
        // Tool result messages carry `tool_name` directly.
        if let Some(name) = m.tool_name.as_deref() {
            *counts.entry(name.to_string()).or_insert(0) += 1;
        }
    }

    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (sorted, files)
}

/// Extract the file path from a tool call's arguments for file-edit tools.
fn tool_path(name: &str, call: &serde_json::Value) -> Option<String> {
    let arg_key = FILE_EDIT_TOOLS
        .iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, key)| *key)?;
    let args = call.get("function")?.get("arguments")?;
    let path = if let Some(s) = args.as_str() {
        // Arguments are an escaped JSON string — parse then re-extract.
        serde_json::from_str::<serde_json::Value>(s)
            .ok()?
            .get(arg_key)?
            .as_str()?
            .to_string()
    } else {
        args.as_object()?.get(arg_key)?.as_str()?.to_string()
    };
    Some(path)
}

/// Latest non-empty user message content.
fn latest_user_prompt(messages: &[MessageData]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .and_then(|m| m.content.clone())
}

/// Latest non-empty assistant text.
fn latest_assistant_text(messages: &[MessageData]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == "assistant" && m.content.as_deref().is_some_and(|c| !c.trim().is_empty())
        })
        .and_then(|m| m.content.clone())
}

/// One-line preview with a hard character cap.
fn preview(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max_chars).collect();
    if flat.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// Short human-friendly path — strip the home prefix if present.
fn display_path(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Some(home) = home.to_str()
        && let Some(rest) = path.strip_prefix(home)
    {
        return format!("~{rest}");
    }
    path.to_string()
}

/// First 8 characters of a session id.
fn short_id(id: &str) -> String {
    let mut out: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        out.push('…');
    }
    out
}

fn plural(n: usize, suffix: &'static str) -> &'static str {
    if n == 1 { "" } else { suffix }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_name: Option<&str>,
    ) -> MessageData {
        MessageData {
            id: 0,
            session_id: "s1".to_string(),
            role: role.to_string(),
            content: content.map(|s| s.to_string()),
            tool_call_id: None,
            tool_calls: tool_calls.map(|s| s.to_string()),
            tool_name: tool_name.map(|s| s.to_string()),
            timestamp: String::new(),
            token_count: None,
            finish_reason: None,
            reasoning: None,
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        }
    }

    #[test]
    fn empty_session_recap_says_nothing_to_recap() {
        let out = build_recap(&[], Some("T"), Some("12345678"));
        assert!(out.contains("Session recap — T"));
        assert!(out.contains("nothing to recap"));
    }

    #[test]
    fn recap_counts_roles_and_window() {
        let mut msgs = vec![
            msg("user", Some("first prompt"), None, None),
            msg("assistant", Some("first reply"), None, None),
            msg("tool", Some("terminal output"), None, Some("terminal")),
        ];
        for i in 0..25 {
            msgs.push(msg("user", Some(&format!("prompt {i}")), None, None));
            msgs.push(msg("assistant", Some(&format!("reply {i}")), None, None));
        }
        let out = build_recap(&msgs, None, Some("abcdefghijkl"));
        assert!(out.contains("Session recap — abcdefgh"));
        assert!(out.contains("tool result"));
    }

    #[test]
    fn recap_counts_tool_calls_and_files() {
        let tool_calls = r#"[{"function":{"name":"write_file","arguments":"{\"path\":\"/home/u/proj/src/main.rs\",\"content\":\"x\"}"}},{"function":{"name":"terminal","arguments":"{\"command\":\"cargo test\"}"}}]"#;
        let msgs = vec![
            msg("user", Some("write code"), None, None),
            msg("assistant", Some("editing"), Some(tool_calls), None),
            msg("tool", Some("ok"), None, Some("write_file")),
            msg("tool", Some("output"), None, Some("terminal")),
        ];
        let out = build_recap(&msgs, None, None);
        // Tie-break is alphabetical ascending — terminal precedes write_file.
        assert!(out.contains("Tools used: terminal×2, write_file×2"));
        assert!(out.contains("Files touched:"));
        assert!(out.contains("/proj/src/main.rs"));
        assert!(out.contains("Latest prompt: write code"));
    }

    #[test]
    fn recap_truncates_long_previews() {
        let long_prompt = "x".repeat(500);
        let msgs = vec![
            msg("user", Some(&long_prompt), None, None),
            msg("assistant", Some("short reply"), None, None),
        ];
        let out = build_recap(&msgs, None, None);
        assert!(out.contains("Latest prompt:"));
        assert!(out.contains('…'));
        assert!(out.contains("Latest reply: short reply"));
    }
}
