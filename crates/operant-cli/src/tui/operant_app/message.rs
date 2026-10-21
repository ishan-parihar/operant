// Vendored from jcode (crates/operant-message-types/src/lib.rs + crates/operant-base/src/message/notifications.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// operant-message-types/src/lib.rs ports whole (854 lines, self-contained);
// [port-decision] its `pub struct ToolCall` (:2-:17) is replaced by a re-export
// of crate::tui::operant_model::vendor_types::ToolCall (the W1 verbatim port of the
// same struct) so the two definitions can never drift. notifications.rs ports
// the background-task helpers the renderers call: strip_ansi_escape_sequences
// (:12), background_task_display_label (:174) + the private helpers it and the
// parsers share (:180-:241), ParsedBackgroundTaskProgressNotification (:348),
// parse_background_task_progress_notification_markdown (:387),
// ParsedBackgroundTaskNotification (:457),
// parse_background_task_notification_markdown (:469); plus compile_static_regex
// (operant-base/src/message.rs:32) re-rooted to operant_app::logging::error.
// [port-excision] notifications' InputShell/model-refresh/stalled formatters are
// not referenced by the ported tree and are not ported. See operant_app/mod.rs.
use regex::Regex;
use std::sync::OnceLock;

pub use crate::tui::operant_model::vendor_types::ToolCall;
// [port-decision] dedup: ToolCall's derive line was left behind when its
// definition moved to operant_model::vendor_types (which carries the same derive);
// deleted here since a `pub use` cannot take it.

/// Role in conversation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A message in the conversation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_duration_ms: Option<u64>,
}

/// Cache control metadata for prompt caching
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

impl CacheControl {
    pub fn ephemeral(ttl: Option<String>) -> Self {
        Self {
            kind: "ephemeral".to_string(),
            ttl,
        }
    }
}

/// Content block within a message
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    /// Hidden reasoning content used for providers that require it (not displayed)
    Reasoning {
        text: String,
    },
    /// History-only reasoning trace. Captured purely so the model's thinking is
    /// preserved in the transcript for later recall/debugging. Unlike
    /// `Reasoning`/`AnthropicThinking`/`OpenAIReasoning`, this block is never
    /// replayed back to any provider, so it carries no token cost on later turns
    /// and cannot trigger provider-side "unsigned thinking" rejections.
    ReasoningTrace {
        text: String,
    },
    /// Anthropic signed thinking content. Anthropic requires the signature when
    /// replaying thinking blocks in future request context.
    AnthropicThinking {
        thinking: String,
        signature: String,
    },
    /// OpenAI Responses reasoning item. When `store=false`, OpenAI returns
    /// encrypted reasoning content so clients can replay reasoning state in
    /// future turns by sending the native reasoning item back in `input`.
    OpenAIReasoning {
        id: String,
        summary: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        /// Gemini 3 "thought signature" for this function call. The Antigravity
        /// / Cloud Code backend requires the original signature to be replayed
        /// on the matching `functionCall` part in subsequent turns, otherwise it
        /// rejects the request ("Function call is missing a thought_signature").
        /// Empty/None for providers that do not use thought signatures.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Image {
        media_type: String,
        data: String,
    },
    /// Hidden OpenAI Responses compaction item used to preserve native
    /// compaction state across turns/saves when jcode explicitly triggers it.
    OpenAICompaction {
        encrypted_content: String,
    },
    /// Loads a deferred tool definition into the model's context at this
    /// point in the conversation, without touching the cached prompt prefix.
    ///
    /// Carried in the user message holding the `ToolResult` it belongs to
    /// (`tool_use_id`). Anthropic renders it as a `tool_reference` inside that
    /// tool_result; OpenAI Responses renders an `additional_tools` input item.
    /// Other providers ignore it.
    ToolReference {
        tool_use_id: String,
        tool_name: String,
    },
}

impl Message {
    pub fn user(text: &str) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            timestamp: Some(chrono::Utc::now()),
            tool_duration_ms: None,
        }
    }

    pub fn assistant_text(text: &str) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            timestamp: Some(chrono::Utc::now()),
            tool_duration_ms: None,
        }
    }
}

/// Project a message down to the fields that actually influence a provider's
/// KV-cache key, dropping harness-only / volatile metadata.
///
/// Providers never receive `Message.timestamp`, `Message.tool_duration_ms`,
/// history-only `ReasoningTrace` blocks, or the positional `cache_control`
/// ephemeral breakpoint markers. Hashing the raw `Message` therefore reports
/// spurious `harness:_prefix_changed` cache misses whenever one of those
/// fields differs on an already-sent boundary message even though the bytes
/// sent upstream are unchanged. A confirmed production instance: persisted
/// memory-injection messages are stored with a slightly later timestamp than
/// the in-flight copy hashed for the request signature, so the raw hash of
/// the same message differed across turns and falsely flagged a prefix edit.
///
/// For user messages the human-readable timestamp is already baked into the
/// text by `Message::with_timestamps` before this projection runs (and
/// system-reminder messages skip timestamp injection entirely), so removing
/// the struct-level `timestamp` field cannot hide a real content change.
pub fn cache_relevant_message_value(message: &Message) -> serde_json::Value {
    let mut value = serde_json::to_value(message).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut value {
        // Struct-level metadata that is never part of the prompt token stream.
        map.remove("timestamp");
        map.remove("tool_duration_ms");
        if let Some(serde_json::Value::Array(blocks)) = map.get_mut("content") {
            // `ReasoningTrace` is documented as history-only and is never
            // replayed to any provider, so it must not affect the cache key.
            blocks.retain(|block| {
                block.get("type").and_then(|kind| kind.as_str()) != Some("reasoning_trace")
            });
            for block in blocks.iter_mut() {
                if let serde_json::Value::Object(block) = block
                    && block.get("type").and_then(|kind| kind.as_str()) == Some("text")
                {
                    // The ephemeral cache breakpoint marker hops to the newest
                    // message each turn; it marks where caching ends, not the
                    // cached content itself.
                    block.remove("cache_control");
                }
            }
        }
    }
    value
}

/// Cache-relevant projections for a whole message list (see
/// [`cache_relevant_message_value`]).
pub fn cache_relevant_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    messages.iter().map(cache_relevant_message_value).collect()
}

/// Per-message hashes over the cache-relevant projection. These are the
/// hashes compared across turns to decide whether the conversation prefix was
/// mutated (a harness bug) or merely appended to (normal growth). Both the
/// local TUI path and the server event path must use this same projection so
/// prefix-change detection never keys off non-transmitted metadata.
pub fn cache_relevant_message_hashes(messages: &[Message]) -> Vec<u64> {
    messages
        .iter()
        .map(|message| {
            let encoded =
                serde_json::to_string(&cache_relevant_message_value(message)).unwrap_or_default();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&encoded, &mut hasher);
            std::hash::Hasher::finish(&hasher)
        })
        .collect()
}

fn is_fresh_user_text_message(message: &Message) -> bool {
    if message.role != Role::User {
        return false;
    }

    let mut saw_user_text = false;
    for block in &message.content {
        match block {
            ContentBlock::Text { text, .. } => {
                let trimmed = text.trim();
                if !trimmed.is_empty() && !trimmed.starts_with("<system-reminder>") {
                    saw_user_text = true;
                }
            }
            ContentBlock::Image { .. } => {}
            _ => return false,
        }
    }

    saw_user_text
}

fn dynamic_system_context_message(system_dynamic: &str) -> Option<Message> {
    let trimmed = system_dynamic.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(Message::user(&format!(
        "<system-reminder>\n{}\n</system-reminder>",
        trimmed
    )))
}

/// Insert dynamic system context after the latest fresh user prompt without
/// disturbing the stable cached history prefix.
pub fn messages_with_dynamic_system_context(
    messages: &[Message],
    system_dynamic: &str,
) -> Vec<Message> {
    let Some(dynamic_message) = dynamic_system_context_message(system_dynamic) else {
        return messages.to_vec();
    };

    let mut out = messages.to_vec();
    let insert_at = out
        .iter()
        .rposition(is_fresh_user_text_message)
        .map(|idx| idx + 1)
        .unwrap_or(out.len());
    out.insert(insert_at, dynamic_message);
    out
}

impl ToolCall {
    /// Build the persisted [`ContentBlock::ToolUse`] for this call.
    ///
    /// Every turn loop stores its completed tool calls this way. Doing it by
    /// hand at each site is how `thought_signature` came to be dropped in three
    /// of four copies: the field was captured off the stream, then hardcoded to
    /// `None` when the assistant message was built, so Gemini-3 saw a
    /// fully-unsigned history on the next turn and rejected it with
    /// `Function call is missing a thought_signature in functionCall parts`.
    /// Keep the construction here so a new field cannot be silently lost again.
    pub fn to_tool_use_block(&self) -> ContentBlock {
        ContentBlock::ToolUse {
            id: self.id.clone(),
            name: self.name.clone(),
            input: self.input.clone(),
            thought_signature: self.thought_signature.clone(),
        }
    }

    pub fn intent_from_input(input: &serde_json::Value) -> Option<String> {
        input
            .get("intent")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|intent| !intent.is_empty())
            .map(ToString::to_string)
    }
}

/// Connection phase for status bar transparency.
///
/// The five transport steps. No live transport in this build emits any of them
/// (the status bar shows `ProcessingStatus::Connecting` instead), so the whole
/// vocabulary is dead here; it is the ported status vocabulary and stays so the
/// first live transport has phases to report rather than a new ad-hoc enum.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: the transport's connection-phase vocabulary; no live transport emits a phase"
    )
)]
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionPhase {
    /// Refreshing OAuth token
    Authenticating,
    /// TCP + TLS / websocket transport setup to the API
    Connecting,
    /// Uploading the request (context) and waiting for response headers
    SendingRequest,
    /// Request accepted, waiting for the model's first output byte
    WaitingForResponse,
    /// First byte received, stream is active
    Streaming,
    /// Retrying after a transient error
    Retrying { attempt: u32, max: u32 },
}

impl std::fmt::Display for ConnectionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionPhase::Authenticating => write!(f, "authenticating"),
            ConnectionPhase::Connecting => write!(f, "connecting"),
            ConnectionPhase::SendingRequest => write!(f, "sending request"),
            ConnectionPhase::WaitingForResponse => write!(f, "waiting for response"),
            ConnectionPhase::Streaming => write!(f, "streaming"),
            ConnectionPhase::Retrying { attempt, max } => {
                write!(f, "retrying ({}/{})", attempt, max)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(message: &Message) -> &str {
        match message.content.first() {
            Some(ContentBlock::Text { text, .. }) => text,
            other => panic!("expected text block, got {:?}", other),
        }
    }

    fn assert_role_text(message: &Message, role: Role, text: &str) {
        assert_eq!(message.role, role);
        assert_eq!(text_of(message), text);
    }

    #[test]
    fn dynamic_context_is_inserted_after_current_user_prompt() {
        let messages = vec![
            Message::user("first user"),
            Message::assistant_text("assistant"),
            Message::user("current user"),
        ];

        let out =
            messages_with_dynamic_system_context(&messages, "# Environment\nTime: 10:00:00 UTC");

        assert_eq!(out.len(), 4);
        assert_eq!(text_of(&out[0]), "first user");
        assert_eq!(text_of(&out[1]), "assistant");
        assert_eq!(text_of(&out[2]), "current user");
        assert!(text_of(&out[3]).starts_with("<system-reminder>\n# Environment"));
    }

    #[test]
    fn dynamic_context_does_not_move_existing_history_prefix() {
        let messages = vec![
            Message::user("stable cached user"),
            Message::assistant_text("stable cached assistant"),
            Message::user("latest prompt"),
        ];

        let out_a = messages_with_dynamic_system_context(&messages, "Time: 10:00:00 UTC");
        let out_b = messages_with_dynamic_system_context(&messages, "Time: 10:00:01 UTC");

        assert_role_text(&out_a[0], Role::User, "stable cached user");
        assert_role_text(&out_a[1], Role::Assistant, "stable cached assistant");
        assert_role_text(&out_b[0], Role::User, "stable cached user");
        assert_role_text(&out_b[1], Role::Assistant, "stable cached assistant");
        assert_role_text(&out_a[2], Role::User, "latest prompt");
        assert_role_text(&out_b[2], Role::User, "latest prompt");
        assert_ne!(text_of(&out_a[3]), text_of(&out_b[3]));
    }

    #[test]
    fn empty_dynamic_context_leaves_messages_unchanged() {
        let messages = vec![Message::user("hello")];
        let out = messages_with_dynamic_system_context(&messages, "\n  \n");
        assert_eq!(out.len(), 1);
        assert_role_text(&out[0], Role::User, "hello");
    }

    #[test]
    fn dynamic_context_appends_when_no_fresh_user_prompt_exists() {
        let messages = vec![
            Message::assistant_text("assistant"),
            Message::user("<system-reminder>\ninternal\n</system-reminder>"),
        ];

        let out = messages_with_dynamic_system_context(&messages, "Time: 10:00:00 UTC");

        assert_eq!(out.len(), 3);
        assert_role_text(&out[0], Role::Assistant, "assistant");
        assert_role_text(
            &out[1],
            Role::User,
            "<system-reminder>\ninternal\n</system-reminder>",
        );
        assert!(text_of(&out[2]).contains("Time: 10:00:00 UTC"));
    }

    #[test]
    fn cache_relevant_hashes_ignore_non_transmitted_metadata() {
        // A persisted memory-injection message is re-serialized on the next
        // turn with a later struct-level timestamp (and possibly a backfilled
        // tool_duration_ms, a hopped cache_control breakpoint, or a
        // history-only ReasoningTrace block). None of that is sent upstream,
        // so the cache-relevant hash must be identical across both copies.
        let sent = Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "<system-reminder>\n# Memory\n</system-reminder>".to_string(),
                cache_control: None,
            }],
            timestamp: Some(chrono::Utc::now()),
            tool_duration_ms: None,
        };
        let persisted = Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "<system-reminder>\n# Memory\n</system-reminder>".to_string(),
                    cache_control: Some(CacheControl::ephemeral(None)),
                },
                ContentBlock::ReasoningTrace {
                    text: "history-only scratch".to_string(),
                },
            ],
            timestamp: Some(chrono::Utc::now() + chrono::Duration::seconds(7)),
            tool_duration_ms: Some(42),
        };

        assert_eq!(
            cache_relevant_message_hashes(std::slice::from_ref(&sent)),
            cache_relevant_message_hashes(std::slice::from_ref(&persisted)),
            "non-transmitted metadata must not change the cache-relevant hash"
        );
        assert_eq!(
            cache_relevant_messages(&[sent]),
            cache_relevant_messages(&[persisted]),
            "cache-relevant projections must be byte-identical"
        );
    }

    #[test]
    fn cache_relevant_hashes_detect_real_content_change() {
        let original = Message::user("original prompt");
        let mut edited = original.clone();
        if let Some(ContentBlock::Text { text, .. }) = edited.content.first_mut() {
            *text = "edited prompt".to_string();
        }

        assert_ne!(
            cache_relevant_message_hashes(&[original]),
            cache_relevant_message_hashes(&[edited]),
            "real content edits must still change the hash"
        );
    }
}

#[cfg(test)]
mod tool_use_block_tests {
    use super::*;

    /// A Gemini-3 tool call's `thought_signature` must survive the trip from the
    /// live stream into the stored assistant message.
    ///
    /// Regression: the turn loops each hand-built `ContentBlock::ToolUse` and
    /// three of the four copies hardcoded `thought_signature: None`. The stream
    /// captured the signature correctly, but it was dropped the moment the
    /// assistant message was persisted, so the *next* request replayed a
    /// fully-unsigned history and the backend rejected it with HTTP 400
    /// `Function call is missing a thought_signature in functionCall parts`.
    /// Reported as a failure on the second message of a Gemini session.
    #[test]
    fn to_tool_use_block_preserves_thought_signature() {
        let call = ToolCall {
            id: "toolu_1".to_string(),
            name: "websearch".to_string(),
            input: serde_json::json!({"query": "water testing"}),
            intent: None,
            thought_signature: Some("SIG_ABC".to_string()),
        };

        match call.to_tool_use_block() {
            ContentBlock::ToolUse {
                id,
                name,
                thought_signature,
                ..
            } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "websearch");
                assert_eq!(
                    thought_signature.as_deref(),
                    Some("SIG_ABC"),
                    "signature must be carried onto the stored block"
                );
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    /// The signature must also survive session serialization, otherwise a
    /// resumed session replays unsigned calls and hits the same 400.
    #[test]
    fn thought_signature_round_trips_through_session_json() {
        let block = ToolCall {
            id: "toolu_2".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
            intent: None,
            thought_signature: Some("SIG_PERSIST".to_string()),
        }
        .to_tool_use_block();

        let json = serde_json::to_string(&block).expect("serialize");
        let restored: ContentBlock = serde_json::from_str(&json).expect("deserialize");

        match restored {
            ContentBlock::ToolUse {
                thought_signature, ..
            } => assert_eq!(thought_signature.as_deref(), Some("SIG_PERSIST")),
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    /// Providers that do not use signatures must stay clean: no empty-string
    /// signature, and the field omitted from serialized sessions.
    #[test]
    fn absent_signature_stays_absent() {
        let block = ToolCall {
            id: "toolu_3".to_string(),
            name: "read".to_string(),
            input: serde_json::json!({}),
            intent: None,
            thought_signature: None,
        }
        .to_tool_use_block();

        let json = serde_json::to_string(&block).expect("serialize");
        assert!(
            !json.contains("thought_signature"),
            "absent signature must be omitted, got {json}"
        );
    }
}

// --- crates/operant-base/src/message/notifications.rs -------------------------------

/// Remove terminal escape sequences before captured command output is rendered.
///
/// Captured output is not attached to a terminal, so retaining control sequences
/// cannot provide useful styling. It can, however, leak SGR parameters as visible
/// text or allow OSC and other terminal commands to reach the renderer.
pub fn strip_ansi_escape_sequences(text: &str) -> String {
    fn consume_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
        for ch in chars.by_ref() {
            if ('@'..='~').contains(&ch) {
                break;
            }
        }
    }

    fn consume_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
        let mut saw_escape = false;
        for ch in chars.by_ref() {
            if ch == '\u{7}' || (saw_escape && ch == '\\') {
                break;
            }
            saw_escape = ch == '\u{1b}';
        }
    }

    fn consume_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                consume_csi(chars);
            }
            Some(']' | 'P' | 'X' | '^' | '_') => {
                chars.next();
                consume_string(chars);
            }
            Some(_) => {
                while chars
                    .peek()
                    .is_some_and(|ch| ('\u{20}'..='\u{2f}').contains(ch))
                {
                    chars.next();
                }
                if chars
                    .peek()
                    .is_some_and(|ch| ('\u{30}'..='\u{7e}').contains(ch))
                {
                    chars.next();
                }
            }
            None => {}
        }
    }

    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => consume_escape(&mut chars),
            '\u{9b}' => consume_csi(&mut chars),
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => consume_string(&mut chars),
            '\u{80}'..='\u{9f}' => {}
            _ => clean.push(ch),
        }
    }
    clean
}

fn background_task_display_name<'a>(
    tool_name: &'a str,
    display_name: Option<&'a str>,
) -> Option<&'a str> {
    display_name
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != tool_name)
}

pub fn background_task_display_label(tool_name: &str, display_name: Option<&str>) -> String {
    background_task_display_name(tool_name, display_name)
        .unwrap_or(tool_name)
        .to_string()
}

fn parse_background_task_header_label(label: &str) -> (String, Option<String>) {
    static NAMED_RE: OnceLock<Option<Regex>> = OnceLock::new();
    static TOOL_RE: OnceLock<Option<Regex>> = OnceLock::new();

    let named_re = NAMED_RE
        .get_or_init(|| {
            compile_static_regex(r"^`(?P<display_name>[^`]+)` \(`(?P<tool_name>[^`]+)`\)$")
        })
        .as_ref();
    if let Some(captures) = named_re.and_then(|re| re.captures(label)) {
        return (
            captures["tool_name"].to_string(),
            Some(captures["display_name"].to_string()),
        );
    }

    let tool_re = TOOL_RE
        .get_or_init(|| compile_static_regex(r"^`(?P<tool_name>[^`]+)`$"))
        .as_ref();
    if let Some(captures) = tool_re.and_then(|re| re.captures(label)) {
        return (captures["tool_name"].to_string(), None);
    }

    (label.trim().to_string(), None)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBackgroundTaskProgressNotification {
    pub task_id: String,
    pub tool_name: String,
    pub display_name: Option<String>,
    pub detail: String,
    pub summary: String,
    pub source: Option<String>,
    pub percent: Option<f32>,
}

pub fn parse_background_task_progress_notification_markdown(
    content: &str,
) -> Option<ParsedBackgroundTaskProgressNotification> {
    static HEADER_RE: OnceLock<Option<Regex>> = OnceLock::new();
    static INLINE_RE: OnceLock<Option<Regex>> = OnceLock::new();

    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();

    let header_re = HEADER_RE
        .get_or_init(|| {
            compile_static_regex(
                r"^\*\*Background task progress\*\* `(?P<task_id>[^`]+)` · (?P<label>.+)$",
            )
        })
        .as_ref()?;
    let inline_re = INLINE_RE
        .get_or_init(|| {
            compile_static_regex(
                r"^\*\*Background task progress\*\* `(?P<task_id>[^`]+)` · (?P<label>.+?) · (?P<detail>.+)$",
            )
        })
        .as_ref()?;

    let (task_id, tool_name, display_name, detail) =
        if let Some(captures) = inline_re.captures(trimmed) {
            let (tool_name, display_name) = parse_background_task_header_label(&captures["label"]);
            (
                captures["task_id"].to_string(),
                tool_name,
                display_name,
                captures["detail"].trim().to_string(),
            )
        } else {
            let mut lines = trimmed.lines();
            let header = lines.next()?.trim();
            let captures = header_re.captures(header)?;
            let (tool_name, display_name) = parse_background_task_header_label(&captures["label"]);
            let detail = lines
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if detail.is_empty() {
                return None;
            }
            (
                captures["task_id"].to_string(),
                tool_name,
                display_name,
                detail,
            )
        };

    let (summary_with_bar, source) = split_progress_source(&detail);
    let summary = strip_progress_bar_prefix(&summary_with_bar).to_string();
    let percent = parse_progress_percent(&summary);

    Some(ParsedBackgroundTaskProgressNotification {
        task_id,
        tool_name,
        display_name,
        detail,
        summary,
        source,
        percent,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBackgroundTaskNotification {
    pub task_id: String,
    pub tool_name: String,
    pub display_name: Option<String>,
    pub status: String,
    pub duration: String,
    pub exit_label: String,
    pub failure_summary: Option<String>,
    pub preview: Option<String>,
    pub full_output_command: String,
}

pub fn parse_background_task_notification_markdown(
    content: &str,
) -> Option<ParsedBackgroundTaskNotification> {
    static HEADER_RE: OnceLock<Option<Regex>> = OnceLock::new();
    static FULL_OUTPUT_RE: OnceLock<Option<Regex>> = OnceLock::new();

    let header_re = HEADER_RE
        .get_or_init(|| {
            compile_static_regex(
                r"^\*\*Background task\*\* `(?P<task_id>[^`]+)` · (?P<label>.+?) · (?P<status>.+?) · (?P<duration>[0-9]+(?:\.[0-9]+)?s) · (?P<exit_label>.+)$",
            )
        })
        .as_ref()?;
    let full_output_re = FULL_OUTPUT_RE
        .get_or_init(|| compile_static_regex(r#"^_Full output:_ `(?P<command>[^`]+)`$"#))
        .as_ref()?;

    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut sections = normalized.split("\n\n");
    let header = sections.next()?.trim();
    let captures = header_re.captures(header)?;
    let (tool_name, display_name) = parse_background_task_header_label(&captures["label"]);

    let mut preview: Option<String> = None;
    let mut failure_summary: Option<String> = None;
    let mut full_output_command: Option<String> = None;

    for section in sections {
        let trimmed = section.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(captures) = full_output_re.captures(trimmed) {
            full_output_command = Some(captures["command"].to_string());
            continue;
        }

        if let Some(summary) = trimmed.strip_prefix("_Failure:_ ") {
            failure_summary = Some(summary.to_string());
            continue;
        }

        if trimmed == "_No output captured._" {
            preview = None;
            continue;
        }

        if let Some(fenced) = trimmed
            .strip_prefix("```text\n")
            .and_then(|body| body.strip_suffix("\n```"))
        {
            preview = Some(fenced.to_string());
        }
    }

    Some(ParsedBackgroundTaskNotification {
        task_id: captures["task_id"].to_string(),
        tool_name,
        display_name,
        status: captures["status"].to_string(),
        duration: captures["duration"].to_string(),
        exit_label: captures["exit_label"].to_string(),
        failure_summary,
        preview,
        full_output_command: full_output_command?,
    })
}

// --- crates/operant-base/src/message.rs --------------------------------------------

fn compile_static_regex(pattern: &str) -> Option<Regex> {
    match Regex::new(pattern) {
        Ok(regex) => Some(regex),
        Err(err) => {
            crate::tui::operant_app::logging::error(&format!(
                "failed to compile static message regex: {err}"
            ));
            // (plan 01, S1b) the logging call above is the failure surface —
            // the eprintln duplicate used to corrupt a live frame.
            None
        }
    }
}

fn split_progress_source(detail: &str) -> (String, Option<String>) {
    for source in ["reported", "parsed", "estimated"] {
        let suffix = format!(" ({source})");
        if let Some(summary) = detail.strip_suffix(&suffix) {
            return (summary.trim().to_string(), Some(source.to_string()));
        }
    }
    (detail.trim().to_string(), None)
}

fn strip_progress_bar_prefix(summary: &str) -> &str {
    if summary.starts_with('[')
        && let Some((bar, rest)) = summary.split_once("] ")
        && bar.chars().all(|ch| matches!(ch, '[' | '#' | '-'))
    {
        return rest.trim();
    }
    summary.trim()
}

fn parse_progress_percent(summary: &str) -> Option<f32> {
    static PERCENT_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let percent_re = PERCENT_RE
        .get_or_init(|| compile_static_regex(r"(?P<percent>[0-9]+(?:\.[0-9]+)?)%"))
        .as_ref()?;
    let captures = percent_re.captures(summary)?;
    captures["percent"].parse::<f32>().ok()
}
