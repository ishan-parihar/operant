//! Loop tests (verbatim body of the former inline `mod tests`).
use super::{
    emergency_history_trim, estimate_history_tokens, fast_trim_tool_results,
    load_interactive_session_history, save_interactive_session_history, truncate_tool_result,
};
use crate::agent::history::{DEFAULT_MAX_HISTORY_MESSAGES, InteractiveSessionState};
use crate::agent::loop_support::glob_match;
use crate::agent::tool_execution::execute_one_tool;
use crate::approval::ApprovalManager;
use operant_providers::ChatMessage;
use operant_tool_call_parser::parse_tool_calls;
use tempfile::tempdir;

// ── truncate_tool_result tests ────────────────────────────────

#[test]
fn truncate_tool_result_short_passthrough() {
    let output = "short output";
    assert_eq!(truncate_tool_result(output, 100), output);
}

#[test]
fn truncate_tool_result_exact_boundary() {
    let output = "a".repeat(100);
    assert_eq!(truncate_tool_result(&output, 100), output);
}

#[test]
fn truncate_tool_result_zero_disables() {
    let output = "a".repeat(200_000);
    assert_eq!(truncate_tool_result(&output, 0), output);
}

#[test]
fn truncate_tool_result_truncates_with_marker() {
    let output = "a".repeat(200);
    let result = truncate_tool_result(&output, 100);
    assert!(result.contains("[... "));
    assert!(result.contains("characters truncated ...]\n\n"));
    // Head should be ~2/3 of 100 = 66, tail ~1/3 = 34
    assert!(result.starts_with("aaa"));
    assert!(result.ends_with("aaa"));
    // Result should be shorter than original
    assert!(result.len() < output.len());
}

#[test]
fn truncate_tool_result_preserves_head_tail_ratio() {
    let output: String = (0u32..1000)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let result = truncate_tool_result(&output, 300);
    // Head = 2/3 of 300 = 200 chars, tail = 100 chars
    // Find the marker
    let marker_start = result.find("[... ").unwrap();
    let marker_end = result.find("characters truncated ...]\n\n").unwrap()
        + "characters truncated ...]\n\n".len();
    let head = &result[..marker_start - 2]; // subtract \n\n
    let tail = &result[marker_end..];
    assert!(
        head.len() >= 190 && head.len() <= 210,
        "head len={}",
        head.len()
    );
    assert!(
        tail.len() >= 90 && tail.len() <= 110,
        "tail len={}",
        tail.len()
    );
}

#[test]
fn truncate_tool_result_utf8_boundary_safety() {
    // Create string with multi-byte chars: each emoji is 4 bytes
    let output = "🦀".repeat(100); // 400 bytes
    // This should not panic even with a limit that falls mid-char
    let result = truncate_tool_result(&output, 50);
    assert!(result.contains("[... "));
    // Verify the result is valid UTF-8 (would panic otherwise)
    let _ = result.len();
}

#[test]
fn truncate_tool_result_very_small_max() {
    let output = "abcdefghijklmnopqrstuvwxyz";
    // With max=5, head=3 tail=2 — result includes marker overhead
    // but should not panic and should contain truncation marker
    let result = truncate_tool_result(output, 5);
    assert!(result.contains("[... "));
    // Head (3 chars) + tail (2 chars) from original should be preserved
    assert!(result.starts_with("abc"));
    assert!(result.ends_with("yz"));
}

// ── truncate_tool_message tests ─────────────────────────────

#[test]
fn truncate_tool_message_preserves_json_structure() {
    use crate::agent::history::truncate_tool_message;
    let big_content = "x".repeat(5000);
    let msg = serde_json::json!({
        "tool_call_id": "call_abc123",
        "content": big_content,
    })
    .to_string();
    let result = truncate_tool_message(&msg, 2000);
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["tool_call_id"], "call_abc123");
    assert!(parsed["content"].as_str().unwrap().contains("[... "));
}

#[test]
fn truncate_tool_message_plain_text_fallback() {
    use crate::agent::history::truncate_tool_message;
    let plain = "a".repeat(5000);
    let result = truncate_tool_message(&plain, 2000);
    assert!(result.contains("[... "));
    assert!(result.len() < 5000);
}

#[test]
fn truncate_tool_message_short_passthrough() {
    use crate::agent::history::truncate_tool_message;
    let msg = r#"{"tool_call_id":"call_1","content":"ok"}"#;
    assert_eq!(truncate_tool_message(msg, 2000), msg);
}

// ── fast_trim_tool_results tests ────────────────────────────

#[test]
fn fast_trim_protects_recent_messages() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::tool("a".repeat(5000)),
        ChatMessage::tool("b".repeat(5000)),
        ChatMessage::user("recent user msg"),
        ChatMessage::tool("c".repeat(5000)), // recent, should be protected
    ];
    // protect_last_n = 2 → last 2 messages protected
    let saved = fast_trim_tool_results(&mut history, 2);
    assert!(saved > 0);
    // First two tool messages should be trimmed
    assert!(history[1].content.len() <= 2100);
    assert!(history[2].content.len() <= 2100);
    // Last tool message (protected) should be unchanged
    assert_eq!(history[4].content.len(), 5000);
}

#[test]
fn fast_trim_skips_non_tool_messages() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("a".repeat(5000)),
        ChatMessage::assistant("b".repeat(5000)),
    ];
    let saved = fast_trim_tool_results(&mut history, 0);
    assert_eq!(saved, 0);
    assert_eq!(history[1].content.len(), 5000);
    assert_eq!(history[2].content.len(), 5000);
}

#[test]
fn fast_trim_small_tool_results_unchanged() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::tool("short result"),
    ];
    let saved = fast_trim_tool_results(&mut history, 0);
    assert_eq!(saved, 0);
    assert_eq!(history[1].content, "short result");
}

// ── emergency_history_trim tests ──────────────────────────────

#[test]
fn emergency_trim_preserves_system() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("msg1"),
        ChatMessage::assistant("resp1"),
        ChatMessage::user("msg2"),
        ChatMessage::assistant("resp2"),
        ChatMessage::user("msg3"),
    ];
    let dropped = emergency_history_trim(&mut history, 2);
    assert!(dropped > 0);
    // System message should always be preserved
    assert_eq!(history[0].role, "system");
    assert_eq!(history[0].content, "sys");
    // Last 2 messages should be preserved
    let len = history.len();
    assert_eq!(history[len - 1].content, "msg3");
}

#[test]
fn emergency_trim_preserves_recent() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("old1"),
        ChatMessage::user("old2"),
        ChatMessage::user("recent1"),
        ChatMessage::user("recent2"),
    ];
    let dropped = emergency_history_trim(&mut history, 2);
    assert!(dropped > 0);
    // Last 2 should be preserved
    let len = history.len();
    assert_eq!(history[len - 1].content, "recent2");
    assert_eq!(history[len - 2].content, "recent1");
}

#[test]
fn emergency_trim_nothing_to_drop() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("only user msg"),
    ];
    // protect_last = 1, system is protected → only 1 droppable
    // target_drop = 2/3 = 0 → nothing dropped
    let dropped = emergency_history_trim(&mut history, 1);
    assert_eq!(dropped, 0);
}

// ── estimate_history_tokens tests ─────────────────────────────

#[test]
fn estimate_tokens_empty_history() {
    let history: Vec<ChatMessage> = vec![];
    assert_eq!(estimate_history_tokens(&history), 0);
}

#[test]
fn estimate_tokens_single_message() {
    // 40 chars → 40.div_ceil(4) + 4 = 10 + 4 = 14 tokens
    let msg = "a".repeat(40);
    let history = vec![ChatMessage::user(&msg)];
    let est = estimate_history_tokens(&history);
    assert_eq!(est, 14);
}

#[test]
fn estimate_tokens_multiple_messages() {
    let history = vec![
        ChatMessage::system("system prompt here"), // 18 chars → 18/4=4 +4=8 (div_ceil: 5+4=9)
        ChatMessage::user("hello"),                // 5 chars → 5/4=1 +4=5 (div_ceil: 2+4=6)
        ChatMessage::assistant("world"),           // 5 chars → 5/4=1 +4=5 (div_ceil: 2+4=6)
    ];
    let est = estimate_history_tokens(&history);
    // Each message: content_len.div_ceil(4) + 4
    // 18.div_ceil(4)=5, 5.div_ceil(4)=2, 5.div_ceil(4)=2 → 5+4 + 2+4 + 2+4 = 21
    assert_eq!(est, 21);
}

#[test]
fn estimate_tokens_large_tool_result() {
    let big = "x".repeat(40_000);
    let history = vec![ChatMessage::tool(&big)];
    let est = estimate_history_tokens(&history);
    // 40000.div_ceil(4) + 4 = 10000 + 4 = 10004
    assert_eq!(est, 10_004);
}

// ── shared_budget tests ───────────────────────────────────────

#[test]
fn shared_budget_decrement_logic() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let budget = Arc::new(AtomicUsize::new(3));

    // Simulate 3 iterations decrementing
    for i in 0..3 {
        let remaining = budget.load(Ordering::Relaxed);
        assert!(remaining > 0, "Budget should be >0 at iteration {i}");
        budget.fetch_sub(1, Ordering::Relaxed);
    }

    // Budget should now be 0
    assert_eq!(budget.load(Ordering::Relaxed), 0);
}

#[test]
fn shared_budget_none_has_no_effect() {
    // When shared_budget is None, the check is simply skipped
    let budget: Option<Arc<std::sync::atomic::AtomicUsize>> = None;
    assert!(budget.is_none());
}

// ── existing tests ────────────────────────────────────────────

#[test]
fn interactive_session_state_round_trips_history() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let history = vec![
        ChatMessage::system("system"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("hi"),
    ];

    save_interactive_session_history(&path, &history).unwrap();
    let restored = load_interactive_session_history(&path, "fallback").unwrap();

    assert_eq!(restored.len(), 3);
    assert_eq!(restored[0].role, "system");
    assert_eq!(restored[1].content, "hello");
    assert_eq!(restored[2].content, "hi");
}

#[test]
fn interactive_session_state_adds_missing_system_prompt() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let payload = serde_json::to_string_pretty(&InteractiveSessionState {
        version: 1,
        history: vec![ChatMessage::user("orphan")],
    })
    .unwrap();
    std::fs::write(&path, payload).unwrap();

    let restored = load_interactive_session_history(&path, "fallback system").unwrap();

    assert_eq!(restored[0].role, "system");
    assert_eq!(restored[0].content, "fallback system");
    assert_eq!(restored[1].content, "orphan");
}

#[test]
fn load_interactive_session_merges_non_leading_system_messages() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let payload = serde_json::to_string_pretty(&InteractiveSessionState {
        version: 1,
        history: vec![
            ChatMessage::system("base system"),
            ChatMessage::user("first question"),
            ChatMessage::assistant("first answer"),
            ChatMessage::system("late loop-detection guidance"),
            ChatMessage::user("follow-up"),
        ],
    })
    .unwrap();
    std::fs::write(&path, payload).unwrap();

    let restored = load_interactive_session_history(&path, "fallback").unwrap();

    assert_eq!(
        restored
            .iter()
            .filter(|message| message.role == "system")
            .count(),
        1,
        "loaded session must not contain non-leading system messages: {:?}",
        restored
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(restored[0].role, "system");
    assert!(restored[0].content.contains("base system"));
    assert!(restored[0].content.contains("late loop-detection guidance"));
    assert_eq!(
        restored
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        vec!["system", "user", "assistant", "user"]
    );
}

#[test]
fn load_interactive_session_replaces_empty_system_messages_with_fallback() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let payload = serde_json::to_string_pretty(&InteractiveSessionState {
        version: 1,
        history: vec![
            ChatMessage::system(""),
            ChatMessage::user("follow-up"),
            ChatMessage::system(""),
        ],
    })
    .unwrap();
    std::fs::write(&path, payload).unwrap();

    let restored = load_interactive_session_history(&path, "fallback system").unwrap();

    assert_eq!(
        restored
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect::<Vec<_>>(),
        vec![("system", "fallback system"), ("user", "follow-up")]
    );
}

/// Regression test for issue #5813: a persisted session whose assistant
/// (tool_use) was lost to compaction must self-heal on load so the next
/// API call doesn't fail with "unexpected tool_use_id found in tool_result
/// blocks".
#[test]
fn load_interactive_session_heals_orphaned_tool_result() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.json");
    let orphan_tool = ChatMessage::tool(
        r#"{"tool_call_id":"toolu_01OrphanFromCompaction","content":"stale result"}"#,
    );
    let payload = serde_json::to_string_pretty(&InteractiveSessionState {
        version: 1,
        history: vec![
            ChatMessage::system("sys"),
            orphan_tool,
            ChatMessage::user("next question"),
        ],
    })
    .unwrap();
    std::fs::write(&path, payload).unwrap();

    let restored = load_interactive_session_history(&path, "fallback").unwrap();

    assert!(
        !restored.iter().any(|m| m.role == "tool"),
        "orphaned tool_result should be removed on load; got roles {:?}",
        restored.iter().map(|m| &m.role).collect::<Vec<_>>()
    );
}

use super::*;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn scrub_credentials_redacts_bearer_token() {
    let input = "API_KEY=sk-1234567890abcdef; token: 1234567890; password=\"secret123456\"";
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("API_KEY=sk-1*[REDACTED]"));
    assert!(scrubbed.contains("token: 1234*[REDACTED]"));
    assert!(scrubbed.contains("password=\"secr*[REDACTED]\""));
    assert!(!scrubbed.contains("abcdef"));
    assert!(!scrubbed.contains("secret123456"));
}

#[test]
fn scrub_credentials_redacts_json_api_key() {
    let input = r#"{"api_key": "sk-1234567890", "other": "public"}"#;
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("\"api_key\": \"sk-1*[REDACTED]\""));
    assert!(scrubbed.contains("public"));
}

#[tokio::test]
async fn execute_one_tool_does_not_panic_on_utf8_boundary() {
    let call_arguments = (0..600)
        .map(|n| serde_json::json!({ "content": format!("{}：tail", "a".repeat(n)) }))
        .find(|args| {
            let raw = args.to_string();
            raw.len() > 300 && !raw.is_char_boundary(300)
        })
        .expect("should produce a sample whose byte index 300 is not a char boundary");

    let observer = NoopObserver;
    let result = execute_one_tool(
        "unknown_tool",
        call_arguments,
        &[],
        None,
        &observer,
        None,
        None,
    )
    .await;
    assert!(result.is_ok(), "execute_one_tool should not panic or error");

    let outcome = result.unwrap();
    assert!(!outcome.success);
    assert!(outcome.output.contains("Unknown tool: unknown_tool"));
}

#[tokio::test]
async fn execute_one_tool_resolves_unique_activated_tool_suffix() {
    let observer = NoopObserver;
    let invocations = Arc::new(AtomicUsize::new(0));
    let activated = Arc::new(std::sync::Mutex::new(crate::tools::ActivatedToolSet::new()));
    let activated_tool: Arc<dyn Tool> = Arc::new(CountingTool::new(
        "docker-mcp__extract_text",
        Arc::clone(&invocations),
    ));
    activated
        .lock()
        .unwrap()
        .activate("docker-mcp__extract_text".into(), activated_tool);

    let outcome = execute_one_tool(
        "extract_text",
        serde_json::json!({ "value": "ok" }),
        &[],
        Some(&activated),
        &observer,
        None,
        None, // receipt_generator
    )
    .await
    .expect("suffix alias should execute the unique activated tool");

    assert!(outcome.success);
    assert_eq!(outcome.output, "counted:ok");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn execute_one_tool_normalizes_empty_success_output() {
    let observer = NoopObserver;
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EmptySuccessTool)];

    let outcome = execute_one_tool(
        "empty_success",
        serde_json::json!({}),
        &tools,
        None,
        &observer,
        None,
        None, // receipt_generator
    )
    .await
    .expect("empty successful tool output should still execute");

    assert!(outcome.success);
    assert_eq!(outcome.output, "(no output)");
    assert!(outcome.error_reason.is_none());
}
use crate::observability::NoopObserver;
use operant_api::provider::{ProviderCapabilities, StreamChunk, StreamEvent, StreamOptions};
use operant_memory::{Memory, MemoryCategory, SqliteMemory};
use operant_providers::ChatResponse;
use tempfile::TempDir;

enum NativeStreamTurn {
    /// Emit a single text delta with associated reasoning content. Used by
    /// regression tests for issue #6059 (DeepSeek V4 thinking-mode replay).
    TextWithReasoning { text: String, reasoning: String },
}

struct StreamingNativeToolEventProvider {
    turns: Arc<Mutex<VecDeque<NativeStreamTurn>>>,
    stream_calls: Arc<AtomicUsize>,
    stream_tool_requests: Arc<AtomicUsize>,
    chat_calls: Arc<AtomicUsize>,
}

impl StreamingNativeToolEventProvider {
    fn with_turns(turns: Vec<NativeStreamTurn>) -> Self {
        Self {
            turns: Arc::new(Mutex::new(turns.into())),
            stream_calls: Arc::new(AtomicUsize::new(0)),
            stream_tool_requests: Arc::new(AtomicUsize::new(0)),
            chat_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Provider for StreamingNativeToolEventProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
            prompt_caching: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        anyhow::bail!(
            "chat_with_system should not be used in streaming native tool event provider tests"
        );
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("chat should not be called when native streaming events succeed")
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_streaming_tool_events(&self) -> bool {
        true
    }

    fn stream_chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: Option<f64>,
        options: StreamOptions,
    ) -> futures_util::stream::BoxStream<
        'static,
        operant_providers::traits::StreamResult<StreamEvent>,
    > {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        if request.tools.is_some_and(|tools| !tools.is_empty()) {
            self.stream_tool_requests.fetch_add(1, Ordering::SeqCst);
        }
        if !options.enabled {
            return Box::pin(futures_util::stream::empty());
        }

        let turn = self
            .turns
            .lock()
            .expect("turns lock should be valid")
            .pop_front()
            .expect("streaming turns should have scripted output");
        match turn {
            NativeStreamTurn::TextWithReasoning { text, reasoning } => {
                Box::pin(futures_util::stream::iter(vec![
                    Ok(StreamEvent::TextDelta(StreamChunk::reasoning(reasoning))),
                    Ok(StreamEvent::TextDelta(StreamChunk::delta(text))),
                    Ok(StreamEvent::Final),
                ]))
            }
        }
    }
}

struct CountingTool {
    name: String,
    invocations: Arc<AtomicUsize>,
}

impl CountingTool {
    fn new(name: &str, invocations: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            invocations,
        }
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Counts executions for loop-stability tests"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<crate::tools::ToolResult> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let value = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("counted:{value}"),
            error: None,
        })
    }
}

struct EmptySuccessTool;

#[async_trait]
impl Tool for EmptySuccessTool {
    fn name(&self) -> &str {
        "empty_success"
    }

    fn description(&self) -> &str {
        "Returns success with no stdout"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<crate::tools::ToolResult> {
        Ok(crate::tools::ToolResult {
            success: true,
            output: String::new(),
            error: None,
        })
    }
}

#[test]
fn should_execute_tools_in_parallel_returns_false_for_single_call() {
    let calls = vec![ParsedToolCall {
        name: "file_read".to_string(),
        arguments: serde_json::json!({"path": "a.txt"}),
        tool_call_id: None,
    }];

    assert!(!should_execute_tools_in_parallel(&calls, None));
}

#[test]
fn should_execute_tools_in_parallel_returns_false_when_approval_is_required() {
    let calls = vec![
        ParsedToolCall {
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "http_request".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            tool_call_id: None,
        },
    ];
    let approval_cfg = operant_config::schema::AutonomyConfig::default();
    let approval_mgr = ApprovalManager::from_config(&approval_cfg);

    assert!(!should_execute_tools_in_parallel(
        &calls,
        Some(&approval_mgr)
    ));
}

#[test]
fn should_execute_tools_in_parallel_returns_true_when_cli_has_no_interactive_approvals() {
    let calls = vec![
        ParsedToolCall {
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "http_request".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            tool_call_id: None,
        },
    ];
    let approval_cfg = operant_config::schema::AutonomyConfig {
        level: crate::security::AutonomyLevel::Full,
        ..operant_config::schema::AutonomyConfig::default()
    };
    let approval_mgr = ApprovalManager::from_config(&approval_cfg);

    assert!(should_execute_tools_in_parallel(
        &calls,
        Some(&approval_mgr)
    ));
}

#[test]
fn should_execute_tools_in_parallel_serializes_multiple_file_mutations() {
    // The old predicate returned true here — the batch ran concurrently and
    // two mutations to the same path could interleave.
    let calls = vec![
        ParsedToolCall {
            name: "file_write".to_string(),
            arguments: serde_json::json!({"path": "a.txt", "content": "x"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "file_edit".to_string(),
            arguments: serde_json::json!({"path": "b.txt"}),
            tool_call_id: None,
        },
    ];

    assert!(!should_execute_tools_in_parallel(&calls, None));
}

#[test]
fn should_execute_tools_in_parallel_keeps_single_mutation_parallel() {
    let calls = vec![
        ParsedToolCall {
            name: "file_write".to_string(),
            arguments: serde_json::json!({"path": "a.txt", "content": "x"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "http_request".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            tool_call_id: None,
        },
    ];

    assert!(should_execute_tools_in_parallel(&calls, None));
}

#[test]
fn resolve_display_text_hides_raw_payload_for_tool_only_turns() {
    let display = resolve_display_text(
        "<tool_call>{\"name\":\"memory_store\"}</tool_call>",
        "",
        true,
        false,
    );
    assert!(display.is_empty());
}

#[test]
fn resolve_display_text_keeps_plain_text_for_tool_turns() {
    let display = resolve_display_text(
        "<tool_call>{\"name\":\"shell\"}</tool_call>",
        "Let me check that.",
        true,
        false,
    );
    assert_eq!(display, "Let me check that.");
}

#[test]
fn resolve_display_text_uses_response_text_for_native_tool_turns() {
    let display = resolve_display_text("Task started.", "", true, true);
    assert_eq!(display, "Task started.");
}

#[test]
fn resolve_display_text_uses_response_text_for_final_turns() {
    let display = resolve_display_text("Final answer", "", false, false);
    assert_eq!(display, "Final answer");
}

#[test]
fn build_tool_instructions_includes_all_tools() {
    use crate::security::SecurityPolicy;
    let security = Arc::new(SecurityPolicy::from_config(
        &operant_config::schema::AutonomyConfig::default(),
        std::path::Path::new("/tmp"),
    ));
    let tools = tools::default_tools(security);
    let instructions = build_tool_instructions(&tools);

    assert!(instructions.contains("## Tool Use Protocol"));
    assert!(instructions.contains("<tool_call>"));
    assert!(instructions.contains("shell"));
    assert!(instructions.contains("file_read"));
    assert!(instructions.contains("file_write"));
}

#[test]
fn build_tool_instructions_empty_registry_returns_empty() {
    let tools: Vec<Box<dyn Tool>> = vec![];
    let instructions = build_tool_instructions(&tools);

    assert!(instructions.is_empty());
}

#[test]
fn tools_to_openai_format_produces_valid_schema() {
    use crate::security::SecurityPolicy;
    let security = Arc::new(SecurityPolicy::from_config(
        &operant_config::schema::AutonomyConfig::default(),
        std::path::Path::new("/tmp"),
    ));
    let tools = tools::default_tools(security);
    let formatted = tools_to_openai_format(&tools);

    assert!(!formatted.is_empty());
    for tool_json in &formatted {
        assert_eq!(tool_json["type"], "function");
        assert!(tool_json["function"]["name"].is_string());
        assert!(tool_json["function"]["description"].is_string());
        assert!(!tool_json["function"]["name"].as_str().unwrap().is_empty());
    }
    // Verify known tools are present
    let names: Vec<&str> = formatted
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
}

#[test]
fn trim_history_preserves_system_prompt() {
    let mut history = vec![ChatMessage::system("system prompt")];
    for i in 0..DEFAULT_MAX_HISTORY_MESSAGES + 20 {
        history.push(ChatMessage::user(format!("msg {i}")));
    }
    let original_len = history.len();
    assert!(original_len > DEFAULT_MAX_HISTORY_MESSAGES + 1);

    trim_history(&mut history, DEFAULT_MAX_HISTORY_MESSAGES);

    // System prompt preserved
    assert_eq!(history[0].role, "system");
    assert_eq!(history[0].content, "system prompt");
    // Trimmed to limit
    assert_eq!(history.len(), DEFAULT_MAX_HISTORY_MESSAGES + 1); // +1 for system
    // Most recent messages preserved
    let last = &history[history.len() - 1];
    assert_eq!(
        last.content,
        format!("msg {}", DEFAULT_MAX_HISTORY_MESSAGES + 19)
    );
}

#[test]
fn trim_history_noop_when_within_limit() {
    let mut history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("hi"),
    ];
    trim_history(&mut history, DEFAULT_MAX_HISTORY_MESSAGES);
    assert_eq!(history.len(), 3);
}

#[test]
fn autosave_memory_key_has_prefix_and_uniqueness() {
    let key1 = autosave_memory_key("user_msg");
    let key2 = autosave_memory_key("user_msg");

    assert!(key1.starts_with("user_msg_"));
    assert!(key2.starts_with("user_msg_"));
    assert_ne!(key1, key2);
}

#[tokio::test]
async fn autosave_memory_keys_preserve_multiple_turns() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();

    let key1 = autosave_memory_key("user_msg");
    let key2 = autosave_memory_key("user_msg");

    mem.store(&key1, "I'm Paul", MemoryCategory::Conversation, None)
        .await
        .unwrap();
    mem.store(&key2, "I'm 45", MemoryCategory::Conversation, None)
        .await
        .unwrap();

    assert_eq!(mem.count().await.unwrap(), 2);

    let recalled = mem.recall("45", 5, None, None, None).await.unwrap();
    assert!(recalled.iter().any(|entry| entry.content.contains("45")));
}

#[tokio::test]
async fn build_context_ignores_legacy_assistant_autosave_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    mem.store(
        "assistant_resp_poisoned",
        "User suffered a fabricated event",
        MemoryCategory::Daily,
        None,
    )
    .await
    .unwrap();
    mem.store(
        "user_preference",
        "User asked for concise status updates",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    let context = build_context(&mem, "status updates", 0.0, None, false).await;
    assert!(context.contains("user_preference"));
    assert!(!context.contains("assistant_resp_poisoned"));
    assert!(!context.contains("fabricated event"));
}

#[tokio::test]
async fn build_context_ignores_user_autosave_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    mem.store(
        "user_msg",
        "Original user message with full conversation history",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();
    mem.store(
        "user_msg_a1b2c3d4",
        "Follow-up user message embedding prior context verbatim",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();
    mem.store(
        "user_preference",
        "User prefers concise answers",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    let context = build_context(&mem, "answers", 0.0, None, false).await;
    assert!(context.contains("user_preference"));
    assert!(!context.contains("user_msg"));
    assert!(!context.contains("embedding prior context"));
}

/// Regression: cron / heartbeat runs must not surface chat-origin
/// `Conversation` memories — the leak path the #5456 prefix filter
/// missed because `agent::run` performs a second, unfiltered recall
/// inside `build_context`. See #5415.
#[tokio::test]
async fn build_context_excludes_conversation_when_flag_set() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    // A Conversation entry written by a chat channel with a non-autosave
    // key (autosave keys are already skipped by the existing filters).
    mem.store(
        "discord:guild:chan:msg-42",
        "Reminder for Alice: the API key is in 1Password vault Foo.",
        MemoryCategory::Conversation,
        Some("discord:guild:chan"),
    )
    .await
    .unwrap();
    // A non-Conversation memory that should still surface so we know the
    // function still does its job — only Conversation should be dropped.
    mem.store(
        "team_oncall",
        "Primary on-call rotates every Monday at 09:00 UTC.",
        MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let context = build_context(&mem, "Alice on-call", 0.0, None, true).await;
    assert!(
        !context.contains("Alice"),
        "Conversation memory leaked into scheduled context: {context}"
    );
    assert!(
        !context.contains("API key"),
        "Conversation memory leaked into scheduled context: {context}"
    );
    assert!(
        context.contains("team_oncall"),
        "Non-Conversation memory should still surface: {context}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Parsing Edge Cases
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn strip_think_tags_removes_single_block() {
    assert_eq!(strip_think_tags("<think>reasoning</think>Hello"), "Hello");
}

#[test]
fn strip_think_tags_removes_multiple_blocks() {
    assert_eq!(strip_think_tags("<think>a</think>X<think>b</think>Y"), "XY");
}

#[test]
fn strip_think_tags_handles_unclosed_block() {
    assert_eq!(strip_think_tags("visible<think>hidden"), "visible");
}

#[test]
fn strip_think_tags_preserves_text_without_tags() {
    assert_eq!(strip_think_tags("plain text"), "plain text");
}

#[test]
fn parse_tool_calls_strips_think_before_tool_call() {
    // Qwen regression: <think> tags before <tool_call> tags should be
    // stripped, allowing the tool call to be parsed correctly.
    let response = "<think>I need to list files to understand the project</think>\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}\n</tool_call>";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(
        calls.len(),
        1,
        "should parse tool call after stripping think tags"
    );
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "ls"
    );
    assert!(text.is_empty(), "think content should not appear as text");
}

#[test]
fn parse_tool_calls_strips_think_only_returns_empty() {
    // When response is only <think> tags with no tool calls, should
    // return empty text and no calls.
    let response = "<think>Just thinking, no action needed</think>";
    let (text, calls) = parse_tool_calls(response);
    assert!(calls.is_empty());
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_handles_qwen_think_with_multiple_tool_calls() {
    let response = "<think>I need to check two things</think>\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"date\"}}\n</tool_call>\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}\n</tool_call>";
    let (_, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "date"
    );
    assert_eq!(
        calls[1].arguments.get("command").unwrap().as_str().unwrap(),
        "pwd"
    );
}

#[test]
fn strip_tool_result_blocks_preserves_clean_text() {
    let input = "Hello, this is a normal response.";
    assert_eq!(strip_tool_result_blocks(input), input);
}

#[test]
fn strip_tool_result_blocks_returns_empty_for_only_tags() {
    let input = "<tool_result name=\"memory_recall\" status=\"ok\">\n{}\n</tool_result>";
    assert_eq!(strip_tool_result_blocks(input), "");
}

#[test]
fn parse_tool_calls_handles_empty_tool_calls_array() {
    // Recovery: Empty tool_calls array returns original response (no tool parsing)
    let response = r#"{"content": "Hello", "tool_calls": []}"#;
    let (text, calls) = parse_tool_calls(response);
    // When tool_calls is empty, the entire JSON is returned as text
    assert!(text.contains("Hello"));
    assert!(calls.is_empty());
}

#[test]
fn detect_tool_call_parse_issue_flags_malformed_payloads() {
    let response = "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}</tool_call>";
    let issue = detect_tool_call_parse_issue(response, &[]);
    assert!(
        issue.is_some(),
        "malformed tool payload should be flagged for diagnostics"
    );
}

#[test]
fn detect_tool_call_parse_issue_ignores_normal_text() {
    let issue = detect_tool_call_parse_issue("Thanks, done.", &[]);
    assert!(issue.is_none());
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - History Management
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn trim_history_with_no_system_prompt() {
    // Recovery: History without system prompt should trim correctly
    let mut history = vec![];
    for i in 0..DEFAULT_MAX_HISTORY_MESSAGES + 20 {
        history.push(ChatMessage::user(format!("msg {i}")));
    }
    trim_history(&mut history, DEFAULT_MAX_HISTORY_MESSAGES);
    assert_eq!(history.len(), DEFAULT_MAX_HISTORY_MESSAGES);
}

#[test]
fn trim_history_preserves_role_ordering() {
    // Recovery: After trimming, role ordering should remain consistent
    let mut history = vec![ChatMessage::system("system")];
    for i in 0..DEFAULT_MAX_HISTORY_MESSAGES + 10 {
        history.push(ChatMessage::user(format!("user {i}")));
        history.push(ChatMessage::assistant(format!("assistant {i}")));
    }
    trim_history(&mut history, DEFAULT_MAX_HISTORY_MESSAGES);
    assert_eq!(history[0].role, "system");
    assert_eq!(history[history.len() - 1].role, "assistant");
}

#[test]
fn trim_history_with_only_system_prompt() {
    // Recovery: Only system prompt should not be trimmed
    let mut history = vec![ChatMessage::system("system prompt")];
    trim_history(&mut history, DEFAULT_MAX_HISTORY_MESSAGES);
    assert_eq!(history.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Arguments Parsing
// ═══════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - JSON Extraction
// ═══════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Constants Validation
// ═══════════════════════════════════════════════════════════════════════

const _: () = {
    assert!(DEFAULT_MAX_HISTORY_MESSAGES > 0);
    assert!(DEFAULT_MAX_HISTORY_MESSAGES <= 1000);
};

#[test]
fn constants_bounds_are_compile_time_checked() {
    // Bounds are enforced by the const assertions above.
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Value Parsing

#[test]
fn parse_tool_calls_handles_unclosed_tool_call_tag() {
    let response = "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}\nDone";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "pwd");
    assert_eq!(text, "Done");
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): parse_tool_calls robustness — malformed/edge-case inputs
// Prevents: Pattern 4 issues #746, #418, #777, #848
// ─────────────────────────────────────────────────────────────────────

#[test]
fn parse_tool_calls_empty_input_returns_empty() {
    let (text, calls) = parse_tool_calls("");
    assert!(calls.is_empty(), "empty input should produce no tool calls");
    assert!(text.is_empty(), "empty input should produce no text");
}

#[test]
fn parse_tool_calls_whitespace_only_returns_empty_calls() {
    let (text, calls) = parse_tool_calls("   \n\t  ");
    assert!(calls.is_empty());
    assert!(text.is_empty() || text.trim().is_empty());
}

#[test]
fn parse_tool_calls_nested_xml_tags_handled() {
    // Double-wrapped tool call should still parse the inner call
    let response =
        r#"<tool_call><tool_call>{"name":"echo","arguments":{"msg":"hi"}}</tool_call></tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    // Should find at least one tool call
    assert!(
        !calls.is_empty(),
        "nested XML tags should still yield at least one tool call"
    );
}

#[test]
fn parse_tool_calls_truncated_json_no_panic() {
    // Incomplete JSON inside tool_call tags
    let response = r#"<tool_call>{"name":"shell","arguments":{"command":"ls"</tool_call>"#;
    let (_text, _calls) = parse_tool_calls(response);
    // Should not panic — graceful handling of truncated JSON
}

#[test]
fn parse_tool_calls_empty_json_object_in_tag() {
    let response = "<tool_call>{}</tool_call>";
    let (_text, calls) = parse_tool_calls(response);
    // Empty JSON object has no name field — should not produce valid tool call
    assert!(
        calls.is_empty(),
        "empty JSON object should not produce a tool call"
    );
}

#[test]
fn parse_tool_calls_closing_tag_only_returns_text() {
    let response = "Some text </tool_call> more text";
    let (text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "closing tag only should not produce calls"
    );
    assert!(
        !text.is_empty(),
        "text around orphaned closing tag should be preserved"
    );
}

#[test]
fn parse_tool_calls_very_large_arguments_no_panic() {
    let large_arg = "x".repeat(100_000);
    let response = format!(
        r#"<tool_call>{{"name":"echo","arguments":{{"message":"{}"}}}}</tool_call>"#,
        large_arg
    );
    let (_text, calls) = parse_tool_calls(&response);
    assert_eq!(calls.len(), 1, "large arguments should still parse");
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_special_characters_in_arguments() {
    let response = r#"<tool_call>{"name":"echo","arguments":{"message":"hello \"world\" <>&'\n\t"}}</tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_text_with_embedded_json_not_extracted() {
    // Raw JSON without any tags should NOT be extracted as a tool call
    let response = r#"Here is some data: {"name":"echo","arguments":{"message":"hi"}} end."#;
    let (_text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "raw JSON in text without tags should not be extracted"
    );
}

#[test]
fn parse_tool_calls_multiple_formats_mixed() {
    // Mix of text and properly tagged tool call
    let response = r#"I'll help you with that.

<tool_call>
{"name":"shell","arguments":{"command":"echo hello"}}
</tool_call>

Let me check the result."#;
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(
        calls.len(),
        1,
        "should extract one tool call from mixed content"
    );
    assert_eq!(calls[0].name, "shell");
    assert!(
        text.contains("help you"),
        "text before tool call should be preserved"
    );
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): scrub_credentials edge cases
// ─────────────────────────────────────────────────────────────────────

#[test]
fn scrub_credentials_empty_input() {
    let result = scrub_credentials("");
    assert_eq!(result, "");
}

#[test]
fn scrub_credentials_no_sensitive_data() {
    let input = "normal text without any secrets";
    let result = scrub_credentials(input);
    assert_eq!(
        result, input,
        "non-sensitive text should pass through unchanged"
    );
}

#[test]
fn scrub_credentials_multibyte_chars_no_panic() {
    // Regression test for #3024: byte index 4 is not a char boundary
    // when the captured value contains multi-byte UTF-8 characters.
    // The regex only matches quoted values for non-ASCII content, since
    // capture group 4 is restricted to [a-zA-Z0-9_\-\.].
    let input = "password=\"\u{4f60}\u{7684}WiFi\u{5bc6}\u{7801}ab\"";
    let result = scrub_credentials(input);
    assert!(
        result.contains("[REDACTED]"),
        "multi-byte quoted value should be redacted without panic, got: {result}"
    );
}

#[test]
fn scrub_credentials_short_values_not_redacted() {
    // Values shorter than 8 chars should not be redacted
    let input = r#"api_key="short""#;
    let result = scrub_credentials(input);
    assert_eq!(result, input, "short values should not be redacted");
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): trim_history edge cases
// ─────────────────────────────────────────────────────────────────────

#[test]
fn trim_history_empty_history() {
    let mut history: Vec<ChatMessage> = vec![];
    trim_history(&mut history, 10);
    assert!(history.is_empty());
}

#[test]
fn trim_history_system_only() {
    let mut history = vec![ChatMessage::system("system prompt")];
    trim_history(&mut history, 10);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, "system");
}

#[test]
fn trim_history_exactly_at_limit() {
    let mut history = vec![
        ChatMessage::system("system"),
        ChatMessage::user("msg 1"),
        ChatMessage::assistant("reply 1"),
    ];
    trim_history(&mut history, 2); // 2 non-system messages = exactly at limit
    assert_eq!(history.len(), 3, "should not trim when exactly at limit");
}

#[test]
fn trim_history_removes_oldest_non_system() {
    let mut history = vec![
        ChatMessage::system("system"),
        ChatMessage::user("old msg"),
        ChatMessage::assistant("old reply"),
        ChatMessage::user("new msg"),
        ChatMessage::assistant("new reply"),
    ];
    trim_history(&mut history, 2);
    assert_eq!(history.len(), 3); // system + 2 kept
    assert_eq!(history[0].role, "system");
    assert_eq!(history[1].content, "new msg");
}

/// When `build_system_prompt_with_mode` is called with `native_tools = true`,
/// the output must contain ZERO XML protocol artifacts and must not inject
/// the duplicate non-native tools summary.
#[test]
fn native_tools_system_prompt_contains_zero_xml() {
    use crate::agent::system_prompt::build_system_prompt_with_mode;

    let workspace = tempdir().unwrap();
    let tool_summaries: Vec<(&str, &str)> = vec![
        ("shell", "Execute shell commands"),
        ("file_read", "Read files"),
    ];

    let system_prompt = build_system_prompt_with_mode(
        workspace.path(),
        "test-model",
        &tool_summaries,
        &[],  // no skills
        None, // no identity config
        None, // no bootstrap_max_chars
        true, // native_tools
        operant_config::schema::SkillsPromptInjectionMode::Full,
        crate::security::AutonomyLevel::default(),
    );

    // Must contain zero XML protocol artifacts
    assert!(
        !system_prompt.contains("<tool_call>"),
        "Native prompt must not contain <tool_call>"
    );
    assert!(
        !system_prompt.contains("</tool_call>"),
        "Native prompt must not contain </tool_call>"
    );
    assert!(
        !system_prompt.contains("<tool_result>"),
        "Native prompt must not contain <tool_result>"
    );
    assert!(
        !system_prompt.contains("</tool_result>"),
        "Native prompt must not contain </tool_result>"
    );
    assert!(
        !system_prompt.contains("## Tool Use Protocol"),
        "Native prompt must not contain XML protocol header"
    );

    // Positive: native prompt should still contain native-task framing.
    assert!(
        !system_prompt.contains("## Tools"),
        "Native prompt should skip the duplicate tools summary"
    );
    assert!(
        system_prompt.contains("## Your Task"),
        "Native prompt should contain task instructions"
    );
}

#[test]
fn non_native_system_prompt_with_no_tools_contains_zero_tool_protocol() {
    use crate::agent::system_prompt::build_system_prompt_with_mode;

    let tool_summaries: Vec<(&str, &str)> = vec![];

    let system_prompt = build_system_prompt_with_mode(
        std::path::Path::new("/tmp"),
        "test-model",
        &tool_summaries,
        &[],
        None,
        None,
        false,
        operant_config::schema::SkillsPromptInjectionMode::Full,
        crate::security::AutonomyLevel::default(),
    );

    assert!(
        !system_prompt.contains("## Tools"),
        "No-tools prompt must not include a Tools section"
    );
    assert!(
        !system_prompt.contains("## Tool Use Protocol"),
        "No-tools prompt must not include tool protocol"
    );
    assert!(
        !system_prompt.contains("<tool_call>"),
        "No-tools prompt must not mention XML tool calls"
    );
    assert!(
        !system_prompt.contains("<tool_result>"),
        "No-tools prompt must not mention XML tool results"
    );
    assert!(
        !system_prompt.contains("Use the tools"),
        "No-tools prompt must not instruct the model to use unavailable tools"
    );
    assert!(
        system_prompt.contains("No tools are available for this turn"),
        "No-tools prompt should explicitly describe the current capability boundary"
    );
}

// ── Cross-Alias & GLM Shortened Body Tests ──────────────────────────

#[test]
fn parse_tool_calls_cross_alias_close_tag_with_json() {
    // <tool_call> opened but closed with </invoke> — JSON body
    let input = r#"<tool_call>{"name": "shell", "arguments": {"command": "ls"}}</invoke>"#;
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "ls");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_cross_alias_close_tag_with_glm_shortened() {
    // <tool_call>shell>uname -a</invoke> — GLM shortened inside cross-alias tags
    let input = "<tool_call>shell>uname -a</invoke>";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "uname -a");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_glm_shortened_body_in_matched_tags() {
    // <tool_call>shell>pwd</tool_call> — GLM shortened in matched tags
    let input = "<tool_call>shell>pwd</tool_call>";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "pwd");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_glm_yaml_style_in_tags() {
    // <tool_call>shell>\ncommand: date\napproved: true</invoke>
    let input = "<tool_call>shell>\ncommand: date\napproved: true</invoke>";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "date");
    assert_eq!(calls[0].arguments["approved"], true);
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_attribute_style_in_tags() {
    // <tool_call>shell command="date" /></tool_call>
    let input = r#"<tool_call>shell command="date" /></tool_call>"#;
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "date");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_file_read_shortened_in_cross_alias() {
    // <tool_call>file_read path=".env" /></invoke>
    let input = r#"<tool_call>file_read path=".env" /></invoke>"#;
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_read");
    assert_eq!(calls[0].arguments["path"], ".env");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_unclosed_glm_shortened_no_close_tag() {
    // <tool_call>shell>ls -la (no close tag at all)
    let input = "<tool_call>shell>ls -la";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "ls -la");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_text_before_cross_alias() {
    // Text before and after cross-alias tool call
    let input = "Let me check that.\n<tool_call>shell>uname -a</invoke>\nDone.";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "uname -a");
    assert!(text.contains("Let me check that."));
    assert!(text.contains("Done."));
}

// ═══════════════════════════════════════════════════════════════════════
// reasoning_content pass-through tests for history builders
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn build_native_assistant_history_includes_reasoning_content() {
    let calls = vec![ToolCall {
        id: "call_1".into(),
        name: "shell".into(),
        arguments: "{}".into(),
        extra_content: None,
    }];
    let result = build_native_assistant_history("answer", &calls, Some("thinking step"));
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert_eq!(parsed["reasoning_content"].as_str(), Some("thinking step"));
    assert!(parsed["tool_calls"].is_array());
}

#[test]
fn build_native_assistant_history_omits_reasoning_content_when_none() {
    let calls = vec![ToolCall {
        id: "call_1".into(),
        name: "shell".into(),
        arguments: "{}".into(),
        extra_content: None,
    }];
    let result = build_native_assistant_history("answer", &calls, None);
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert!(parsed.get("reasoning_content").is_none());
}

#[test]
fn build_native_assistant_history_from_parsed_calls_includes_reasoning_content() {
    let calls = vec![ParsedToolCall {
        name: "shell".into(),
        arguments: serde_json::json!({"command": "pwd"}),
        tool_call_id: Some("call_2".into()),
    }];
    let result =
        build_native_assistant_history_from_parsed_calls("answer", &calls, Some("deep thought"));
    assert!(result.is_some());
    let parsed: serde_json::Value = serde_json::from_str(result.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert_eq!(parsed["reasoning_content"].as_str(), Some("deep thought"));
    assert!(parsed["tool_calls"].is_array());
}

#[test]
fn build_native_assistant_history_from_parsed_calls_omits_reasoning_content_when_none() {
    let calls = vec![ParsedToolCall {
        name: "shell".into(),
        arguments: serde_json::json!({"command": "pwd"}),
        tool_call_id: Some("call_2".into()),
    }];
    let result = build_native_assistant_history_from_parsed_calls("answer", &calls, None);
    assert!(result.is_some());
    let parsed: serde_json::Value = serde_json::from_str(result.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert!(parsed.get("reasoning_content").is_none());
}

/// Regression test for issue #6059 — DeepSeek V4 thinking-mode tool-call
/// replay rejected with `400` because the assistant's prior
/// `reasoning_content` was missing from the next request.
///
/// Before the fix, the streaming consumer dropped reasoning chunks on the
/// floor (`chunk.delta.is_empty()` short-circuit + hardcoded
/// `reasoning_content: None` on the synthesized `ChatResponse`). After
/// the fix, reasoning deltas accumulate into `StreamedChatOutcome` and
/// surface on the response so the agent's history layer can persist them
/// and replay them on subsequent turns.
#[tokio::test]
async fn consume_provider_streaming_response_captures_reasoning_content() {
    let provider =
        StreamingNativeToolEventProvider::with_turns(vec![NativeStreamTurn::TextWithReasoning {
            text: "Listing the directory now.".to_string(),
            reasoning: "I need to call the shell tool to list files.".to_string(),
        }]);
    let messages = vec![ChatMessage::user(
        "List the folders in the current directory",
    )];

    let outcome = consume_provider_streaming_response(
        &provider,
        &messages,
        None,
        "deepseek-v4-pro",
        0.2,
        None,
        None,
    )
    .await
    .expect("streaming should succeed");

    assert_eq!(outcome.response_text, "Listing the directory now.");
    assert_eq!(
        outcome.reasoning_content,
        "I need to call the shell tool to list files."
    );
    assert!(
        outcome.tool_calls.is_empty(),
        "this turn does not emit native tool calls"
    );
}

#[tokio::test]
async fn consume_provider_streaming_response_accumulates_split_reasoning_chunks() {
    // Scripted multi-event stream: two reasoning chunks straddling a text
    // delta. The outcome should concatenate the reasoning chunks in order
    // and keep them out of the visible response text.
    struct MultiChunkProvider;

    #[async_trait]
    impl Provider for MultiChunkProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<String> {
            anyhow::bail!("not used in this test")
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used in this test")
        }

        fn supports_streaming(&self) -> bool {
            true
        }

        fn stream_chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: Option<f64>,
            _options: StreamOptions,
        ) -> futures_util::stream::BoxStream<
            'static,
            operant_providers::traits::StreamResult<StreamEvent>,
        > {
            Box::pin(futures_util::stream::iter(vec![
                Ok(StreamEvent::TextDelta(StreamChunk::reasoning("Step 1: "))),
                Ok(StreamEvent::TextDelta(StreamChunk::delta("Hello "))),
                Ok(StreamEvent::TextDelta(StreamChunk::reasoning(
                    "consider options.",
                ))),
                Ok(StreamEvent::TextDelta(StreamChunk::delta("there."))),
                Ok(StreamEvent::Final),
            ]))
        }
    }

    let provider = MultiChunkProvider;
    let messages = vec![ChatMessage::user("hi")];

    let outcome = consume_provider_streaming_response(
        &provider,
        &messages,
        None,
        "deepseek-v4-flash",
        0.2,
        None,
        None,
    )
    .await
    .expect("streaming should succeed");

    assert_eq!(outcome.response_text, "Hello there.");
    assert_eq!(outcome.reasoning_content, "Step 1: consider options.");
}

// ── glob_match tests ──────────────────────────────────────────────────────

#[test]
fn glob_match_exact_no_wildcard() {
    assert!(glob_match("mcp_browser_navigate", "mcp_browser_navigate"));
    assert!(!glob_match("mcp_browser_navigate", "mcp_browser_click"));
}

#[test]
fn glob_match_prefix_wildcard() {
    // Suffix pattern: mcp_browser_*
    assert!(glob_match("mcp_browser_*", "mcp_browser_navigate"));
    assert!(glob_match("mcp_browser_*", "mcp_browser_click"));
    assert!(!glob_match("mcp_browser_*", "mcp_filesystem_read"));

    // Prefix pattern: *_read
    assert!(glob_match("*_read", "mcp_filesystem_read"));
    assert!(!glob_match("*_read", "mcp_filesystem_write"));

    // Infix: mcp_*_navigate
    assert!(glob_match("mcp_*_navigate", "mcp_browser_navigate"));
    assert!(!glob_match("mcp_*_navigate", "mcp_browser_click"));
}

#[test]
fn glob_match_star_matches_everything() {
    assert!(glob_match("*", "anything_at_all"));
    assert!(glob_match("*", ""));
}

// ── filter_tool_specs_for_turn tests ──────────────────────────────────────

fn make_spec(name: &str) -> crate::tools::ToolSpec {
    crate::tools::ToolSpec {
        name: name.to_string(),
        description: String::new(),
        parameters: serde_json::json!({}),
    }
}

#[test]
fn filter_tool_specs_no_groups_returns_all() {
    let specs = vec![
        make_spec("shell_exec"),
        make_spec("mcp_browser_navigate"),
        make_spec("mcp_filesystem_read"),
    ];
    let result = filter_tool_specs_for_turn(specs, &[], "hello");
    assert_eq!(result.len(), 3);
}

#[test]
fn filter_tool_specs_always_group_includes_matching_mcp_tool() {
    use operant_config::schema::{ToolFilterGroup, ToolFilterGroupMode};

    let specs = vec![
        make_spec("shell_exec"),
        make_spec("mcp_browser_navigate"),
        make_spec("mcp_filesystem_read"),
    ];
    let groups = vec![ToolFilterGroup {
        mode: ToolFilterGroupMode::Always,
        tools: vec!["mcp_filesystem_*".into()],
        keywords: vec![],
        filter_builtins: false,
    }];
    let result = filter_tool_specs_for_turn(specs, &groups, "anything");
    let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
    // Built-in passes through, matched MCP passes, unmatched MCP excluded.
    assert!(names.contains(&"shell_exec"));
    assert!(names.contains(&"mcp_filesystem_read"));
    assert!(!names.contains(&"mcp_browser_navigate"));
}

#[test]
fn filter_tool_specs_dynamic_group_included_on_keyword_match() {
    use operant_config::schema::{ToolFilterGroup, ToolFilterGroupMode};

    let specs = vec![make_spec("shell_exec"), make_spec("mcp_browser_navigate")];
    let groups = vec![ToolFilterGroup {
        mode: ToolFilterGroupMode::Dynamic,
        tools: vec!["mcp_browser_*".into()],
        keywords: vec!["browse".into(), "website".into()],
        filter_builtins: false,
    }];
    let result = filter_tool_specs_for_turn(specs, &groups, "please browse this page");
    let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"shell_exec"));
    assert!(names.contains(&"mcp_browser_navigate"));
}

#[test]
fn filter_tool_specs_dynamic_group_excluded_on_no_keyword_match() {
    use operant_config::schema::{ToolFilterGroup, ToolFilterGroupMode};

    let specs = vec![make_spec("shell_exec"), make_spec("mcp_browser_navigate")];
    let groups = vec![ToolFilterGroup {
        mode: ToolFilterGroupMode::Dynamic,
        tools: vec!["mcp_browser_*".into()],
        keywords: vec!["browse".into(), "website".into()],
        filter_builtins: false,
    }];
    let result = filter_tool_specs_for_turn(specs, &groups, "read the file /etc/hosts");
    let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"shell_exec"));
    assert!(!names.contains(&"mcp_browser_navigate"));
}

#[test]
fn filter_tool_specs_dynamic_keyword_match_is_case_insensitive() {
    use operant_config::schema::{ToolFilterGroup, ToolFilterGroupMode};

    let specs = vec![make_spec("mcp_browser_navigate")];
    let groups = vec![ToolFilterGroup {
        mode: ToolFilterGroupMode::Dynamic,
        tools: vec!["mcp_browser_*".into()],
        keywords: vec!["Browse".into()],
        filter_builtins: false,
    }];
    let result = filter_tool_specs_for_turn(specs, &groups, "BROWSE the site");
    assert_eq!(result.len(), 1);
}

// ── Token-based compaction tests ──────────────────────────

#[test]
fn estimate_history_tokens_empty() {
    assert_eq!(super::estimate_history_tokens(&[]), 0);
}

#[test]
fn estimate_history_tokens_single_message() {
    let history = vec![ChatMessage::user("hello world")]; // 11 chars
    let tokens = super::estimate_history_tokens(&history);
    // 11.div_ceil(4) + 4 = 3 + 4 = 7
    assert_eq!(tokens, 7);
}

#[test]
fn estimate_history_tokens_multiple_messages() {
    let history = vec![
        ChatMessage::system("You are helpful."), // 16 chars → 4 + 4 = 8
        ChatMessage::user("What is Rust?"),      // 13 chars → 4 + 4 = 8
        ChatMessage::assistant("A language."),   // 11 chars → 3 + 4 = 7
    ];
    let tokens = super::estimate_history_tokens(&history);
    assert_eq!(tokens, 23);
}
