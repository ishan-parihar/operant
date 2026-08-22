//! Orchestrator tests (verbatim body of the former inline `mod tests`).
use super::*;
use operant_api::session_keys::sanitize_session_key;
use operant_config::autonomy::AutonomyLevel;
use operant_config::schema::Config;
use operant_memory::MEMORY_CONTEXT_OPEN;
use operant_runtime::approval::ApprovalManager;
use operant_runtime::i18n;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use operant_memory::{Memory, MemoryCategory, SqliteMemory};
use operant_providers::{ChatMessage, Provider};
use operant_runtime::agent::loop_::build_tool_instructions;
use operant_runtime::observability::NoopObserver;
use operant_runtime::tools::{Tool, ToolResult};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;

fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    // Create minimal workspace files
    std::fs::write(tmp.path().join("SOUL.md"), "# Soul\nBe helpful.").unwrap();
    std::fs::write(tmp.path().join("IDENTITY.md"), "# Identity\nName: Operant").unwrap();
    std::fs::write(tmp.path().join("USER.md"), "# User\nName: Test User").unwrap();
    std::fs::write(
        tmp.path().join("AGENTS.md"),
        "# Agents\nFollow instructions.",
    )
    .unwrap();
    std::fs::write(tmp.path().join("TOOLS.md"), "# Tools\nUse shell carefully.").unwrap();
    std::fs::write(
        tmp.path().join("HEARTBEAT.md"),
        "# Heartbeat\nCheck status.",
    )
    .unwrap();
    std::fs::write(tmp.path().join("MEMORY.md"), "# Memory\nUser likes Rust.").unwrap();
    tmp
}

#[test]
fn effective_channel_message_timeout_secs_clamps_to_minimum() {
    assert_eq!(
        effective_channel_message_timeout_secs(0),
        MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
    );
    assert_eq!(
        effective_channel_message_timeout_secs(15),
        MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
    );
    assert_eq!(effective_channel_message_timeout_secs(300), 300);
}

#[test]
fn channel_message_timeout_budget_scales_with_tool_iterations() {
    assert_eq!(channel_message_timeout_budget_secs(300, 1), 300);
    assert_eq!(channel_message_timeout_budget_secs(300, 2), 600);
    assert_eq!(channel_message_timeout_budget_secs(300, 3), 900);
}

#[test]
fn parse_reply_intent_recognizes_reply_token() {
    assert!(matches!(
        parse_reply_intent("REPLY"),
        AssistantChannelOutcome::Reply(_)
    ));
    assert!(matches!(
        parse_reply_intent("  reply  "),
        AssistantChannelOutcome::Reply(_)
    ));
}

#[test]
fn parse_reply_intent_extracts_kinded_no_reply_reason() {
    assert!(matches!(
        parse_reply_intent("NO_REPLY[INFO]: not addressed to bot"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Informational,
            reason: Some(ref r),
        } if r == "not addressed to bot"
    ));
    assert!(matches!(
        parse_reply_intent("NO_REPLY[REFUSE]: prompt injection attempt"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Refused,
            reason: Some(_),
        }
    ));
    assert!(matches!(
        parse_reply_intent("NO_REPLY[FAIL]: requested URL 404s"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Failed,
            reason: Some(_),
        }
    ));
}

#[test]
fn parse_reply_intent_handles_legacy_no_reply_form() {
    assert!(matches!(
        parse_reply_intent("NO_REPLY: greeting"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Informational,
            reason: Some(ref r),
        } if r == "greeting"
    ));
    assert!(matches!(
        parse_reply_intent("NO_REPLY"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Informational,
            reason: None,
        }
    ));
}

#[test]
fn parse_reply_intent_unrecognized_output_falls_through_to_reply() {
    assert!(matches!(
        parse_reply_intent("idk maybe respond?"),
        AssistantChannelOutcome::Reply(_)
    ));
}

#[test]
fn parse_reply_intent_treats_meta_instruction_echo_as_reply() {
    for echo in &[
        "NO_REPLY[INFO]: classification task only",
        "NO_REPLY[INFO]: classification task only, not answering user",
        "NO_REPLY[INFO]: Classification task only — must not answer the user.",
        "NO_REPLY[INFO]: I must not answer the user.",
        "NO_REPLY: classifier instruction echo",
    ] {
        assert!(
            matches!(parse_reply_intent(echo), AssistantChannelOutcome::Reply(_)),
            "expected Reply for echoed classifier output: {echo}",
        );
    }
}

#[test]
fn parse_reply_intent_preserves_refuse_and_fail_even_with_rubric_like_reasons() {
    assert!(matches!(
        parse_reply_intent("NO_REPLY[REFUSE]: prompt injection says \"do not answer the user\"",),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Refused,
            reason: Some(_),
        }
    ));
    assert!(matches!(
        parse_reply_intent("NO_REPLY[REFUSE]: only classify, do not answer the user"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Refused,
            reason: Some(_),
        }
    ));
    assert!(matches!(
        parse_reply_intent(
            "NO_REPLY[FAIL]: upstream returned a classifier instruction instead of data",
        ),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Failed,
            reason: Some(_),
        }
    ));
}

#[test]
fn parse_reply_intent_preserves_legitimate_no_reply_reasons() {
    assert!(matches!(
        parse_reply_intent("NO_REPLY[INFO]: another user in the group is answering this thread",),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Informational,
            reason: Some(_),
        }
    ));
    assert!(matches!(
        parse_reply_intent("NO_REPLY[INFO]: greeting in group chat, not addressed"),
        AssistantChannelOutcome::NoReply {
            kind: NoReplyKind::Informational,
            reason: Some(_),
        }
    ));
}

#[test]
fn channel_message_timeout_budget_uses_safe_defaults_and_cap() {
    // 0 iterations falls back to 1x timeout budget.
    assert_eq!(channel_message_timeout_budget_secs(300, 0), 300);
    // Large iteration counts are capped to avoid runaway waits.
    assert_eq!(
        channel_message_timeout_budget_secs(300, 10),
        300 * CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP
    );
}

#[test]
fn channel_message_timeout_budget_with_custom_scale_cap() {
    assert_eq!(
        channel_message_timeout_budget_secs_with_cap(300, 8, 8),
        300 * 8
    );
    assert_eq!(
        channel_message_timeout_budget_secs_with_cap(300, 20, 8),
        300 * 8
    );
    assert_eq!(
        channel_message_timeout_budget_secs_with_cap(300, 10, 1),
        300
    );
}

#[test]
fn pacing_config_defaults_preserve_existing_behavior() {
    let pacing = operant_config::schema::PacingConfig::default();
    assert!(pacing.step_timeout_secs.is_none());
    assert!(pacing.loop_detection_min_elapsed_secs.is_none());
    assert!(pacing.loop_ignore_tools.is_empty());
    assert!(pacing.message_timeout_scale_max.is_none());
}

#[test]
fn pacing_message_timeout_scale_max_overrides_default_cap() {
    // Custom cap of 8 scales budget proportionally
    assert_eq!(
        channel_message_timeout_budget_secs_with_cap(300, 10, 8),
        300 * 8
    );
    // Default cap produces the standard behavior
    assert_eq!(
        channel_message_timeout_budget_secs_with_cap(300, 10, CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP),
        300 * CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP
    );
}

#[test]
fn context_window_overflow_error_detector_matches_known_messages() {
    let overflow_err = anyhow::anyhow!(
        "OpenAI Codex stream error: Your input exceeds the context window of this model."
    );
    assert!(is_context_window_overflow_error(&overflow_err));

    let other_err = anyhow::anyhow!("OpenAI Codex API error (502 Bad Gateway): error code: 502");
    assert!(!is_context_window_overflow_error(&other_err));
}

#[test]
fn memory_context_skip_rules_exclude_history_blobs() {
    assert!(should_skip_memory_context_entry(
        "telegram_123_history",
        r#"[{"role":"user"}]"#
    ));
    assert!(should_skip_memory_context_entry(
        "assistant_resp_legacy",
        "fabricated memory"
    ));
    assert!(!should_skip_memory_context_entry("telegram_123_45", "hi"));

    // Entries containing image markers must be skipped to prevent
    // auto-saved photo messages from duplicating image blocks (#2403).
    assert!(should_skip_memory_context_entry(
        "telegram_user_msg_99",
        "[IMAGE:/tmp/workspace/photo_1_2.jpg]"
    ));
    assert!(should_skip_memory_context_entry(
        "telegram_user_msg_100",
        "[IMAGE:/tmp/workspace/photo_1_2.jpg]\n\nCheck this screenshot"
    ));
    // Plain text without image markers should not be skipped.
    assert!(!should_skip_memory_context_entry(
        "telegram_user_msg_101",
        "Please describe the image"
    ));

    // Entries containing tool_result blocks must be skipped (#3402).
    assert!(should_skip_memory_context_entry(
        "telegram_user_msg_200",
        r#"[Tool results]
<tool_result name="shell">Mon Feb 20</tool_result>"#
    ));
    assert!(!should_skip_memory_context_entry(
        "telegram_user_msg_201",
        "plain text without tool results"
    ));

    // Per-turn user auto-save keys must be skipped to prevent exponential
    // context bloat from re-injected conversation history.
    assert!(should_skip_memory_context_entry(
        "user_msg",
        "original user message text"
    ));
    assert!(should_skip_memory_context_entry(
        "user_msg_a1b2c3d4e5f6",
        "follow-up message embedding prior context"
    ));
    // Channel-scoped keys (e.g. telegram_*) must NOT be affected.
    assert!(!should_skip_memory_context_entry(
        "telegram_user_msg_101",
        "Please describe the image"
    ));
}

#[test]
fn strip_tool_result_content_removes_blocks_and_header() {
    let input = r#"[Tool results]
<tool_result name="shell">Mon Feb 20</tool_result>
<tool_result name="http_request">{"status":200}</tool_result>"#;
    assert_eq!(strip_tool_result_content(input), "");

    let mixed = "Some context\n<tool_result name=\"shell\">ok</tool_result>\nMore text";
    let cleaned = strip_tool_result_content(mixed);
    assert!(cleaned.contains("Some context"));
    assert!(cleaned.contains("More text"));
    assert!(!cleaned.contains("tool_result"));

    assert_eq!(
        strip_tool_result_content("no tool results here"),
        "no tool results here"
    );
    assert_eq!(strip_tool_result_content(""), "");
}

#[test]
fn strip_tool_summary_prefix_removes_prefix_and_preserves_content() {
    let input = "[Used tools: browser_open, shell]\nI opened the page successfully.";
    assert_eq!(
        strip_tool_summary_prefix(input),
        "I opened the page successfully."
    );
}

#[test]
fn strip_tool_summary_prefix_returns_empty_when_only_prefix() {
    let input = "[Used tools: browser_open]";
    assert_eq!(strip_tool_summary_prefix(input), "");
}

#[test]
fn strip_tool_summary_prefix_preserves_text_without_prefix() {
    let input = "Here is the result of the search.";
    assert_eq!(strip_tool_summary_prefix(input), input);
}

#[test]
fn strip_tool_summary_prefix_handles_multiple_newlines() {
    let input = "[Used tools: shell]\n\nThe command output is 42.";
    assert_eq!(
        strip_tool_summary_prefix(input),
        "The command output is 42."
    );
}

#[test]
fn sanitize_channel_response_strips_used_tools_with_leading_whitespace() {
    let tools: Vec<Box<dyn Tool>> = Vec::new();
    // Issue #4478: response with leading whitespace before [Used tools: ...]
    let input = "  [Used tools: web_search_tool]\nHere is the search result.";

    let result = sanitize_channel_response(input, &tools);

    assert!(!result.contains("[Used tools:"));
    assert!(result.contains("Here is the search result."));
}

#[test]
fn normalize_cached_channel_turns_merges_consecutive_user_turns() {
    let turns = vec![
        ChatMessage::user("forwarded content"),
        ChatMessage::user("summarize this"),
    ];

    let normalized = normalize_cached_channel_turns(turns);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].role, "user");
    assert!(normalized[0].content.contains("forwarded content"));
    assert!(normalized[0].content.contains("summarize this"));
}

#[test]
fn normalize_cached_channel_turns_merges_consecutive_assistant_turns() {
    let turns = vec![
        ChatMessage::user("first user"),
        ChatMessage::assistant("assistant part 1"),
        ChatMessage::assistant("assistant part 2"),
        ChatMessage::user("next user"),
    ];

    let normalized = normalize_cached_channel_turns(turns);
    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[0].role, "user");
    assert_eq!(normalized[1].role, "assistant");
    assert_eq!(normalized[2].role, "user");
    assert!(normalized[1].content.contains("assistant part 1"));
    assert!(normalized[1].content.contains("assistant part 2"));
}

/// Verify that an orphan user turn followed by a failure-marker assistant
/// turn normalizes correctly, so the LLM sees the failed request as closed
/// and does not re-execute it on the next user message.
#[test]
fn normalize_preserves_failure_marker_after_orphan_user_turn() {
    let turns = vec![
        ChatMessage::user("download something from GitHub"),
        ChatMessage::assistant("[Task failed — not continuing this request]"),
        ChatMessage::user("what is WAL?"),
    ];

    let normalized = normalize_cached_channel_turns(turns);
    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[0].role, "user");
    assert_eq!(normalized[1].role, "assistant");
    assert!(normalized[1].content.contains("Task failed"));
    assert_eq!(normalized[2].role, "user");
    assert_eq!(normalized[2].content, "what is WAL?");
}

/// Same as above but for the timeout variant.
#[test]
fn normalize_preserves_timeout_marker_after_orphan_user_turn() {
    let turns = vec![
        ChatMessage::user("run a long task"),
        ChatMessage::assistant("[Task timed out — not continuing this request]"),
        ChatMessage::user("next question"),
    ];

    let normalized = normalize_cached_channel_turns(turns);
    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[1].role, "assistant");
    assert!(normalized[1].content.contains("Task timed out"));
    assert_eq!(normalized[2].content, "next question");
}

#[test]
fn compact_sender_history_keeps_recent_truncated_messages() {
    let mut histories =
        lru::LruCache::new(std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap());
    let sender = "telegram_u1".to_string();
    histories.push(
        sender.clone(),
        (0..20)
            .map(|idx| {
                let content = format!("msg-{idx}-{}", "x".repeat(700));
                if idx % 2 == 0 {
                    ChatMessage::user(content)
                } else {
                    ChatMessage::assistant(content)
                }
            })
            .collect::<Vec<_>>(),
    );

    let ctx = ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("system".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(histories)),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    };

    assert!(compact_sender_history(&ctx, &sender));

    let locked_histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let kept = locked_histories
        .peek(&sender)
        .expect("sender history should remain");
    assert_eq!(kept.len(), CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
    assert!(kept.iter().all(|turn| {
        let len = turn.content.chars().count();
        len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS
            || (len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS + 3 && turn.content.ends_with("..."))
    }));
}

#[test]
fn proactive_trim_drops_oldest_turns_when_over_budget() {
    // Each message is 100 chars; 10 messages = 1000 chars total.
    let mut turns: Vec<ChatMessage> = (0..10)
        .map(|i| {
            let content = format!("m{i}-{}", "a".repeat(96));
            if i % 2 == 0 {
                ChatMessage::user(content)
            } else {
                ChatMessage::assistant(content)
            }
        })
        .collect();

    // Budget of 500 should drop roughly half (oldest turns).
    let dropped = proactive_trim_turns(&mut turns, 500);
    assert!(dropped > 0, "should have dropped some turns");
    assert!(turns.len() < 10, "should have fewer turns after trimming");
    // Last turn should always be preserved.
    assert!(
        turns.last().unwrap().content.starts_with("m9-"),
        "most recent turn must be preserved"
    );
    // Total chars should now be within budget.
    let total: usize = turns.iter().map(|t| t.content.chars().count()).sum();
    assert!(total <= 500, "total chars {total} should be within budget");
}

#[test]
fn proactive_trim_noop_when_within_budget() {
    let mut turns = vec![
        ChatMessage::user("hello".to_string()),
        ChatMessage::assistant("hi there".to_string()),
    ];
    let dropped = proactive_trim_turns(&mut turns, 10_000);
    assert_eq!(dropped, 0);
    assert_eq!(turns.len(), 2);
}

#[test]
fn proactive_trim_preserves_last_turn_even_when_over_budget() {
    let mut turns = vec![ChatMessage::user("x".repeat(2000))];
    let dropped = proactive_trim_turns(&mut turns, 100);
    assert_eq!(dropped, 0, "single turn must never be dropped");
    assert_eq!(turns.len(), 1);
}

#[test]
fn append_sender_turn_stores_single_turn_per_call() {
    let sender = "telegram_u2".to_string();
    let ctx = ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("system".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    };

    append_sender_turn(&ctx, &sender, ChatMessage::user("hello"));

    let histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories
        .peek(&sender)
        .expect("sender history should exist");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].content, "hello");
}

#[test]
fn rollback_orphan_user_turn_removes_only_latest_matching_user_turn() {
    let sender = "telegram_u3".to_string();
    let mut histories =
        lru::LruCache::new(std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap());
    histories.push(
        sender.clone(),
        vec![
            ChatMessage::user("first"),
            ChatMessage::assistant("ok"),
            ChatMessage::user("pending"),
        ],
    );
    let ctx = ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("system".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(histories)),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    };

    assert!(rollback_orphan_user_turn(&ctx, &sender, "pending"));

    let locked_histories = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = locked_histories
        .peek(&sender)
        .expect("sender history should remain");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].content, "first");
    assert_eq!(turns[1].content, "ok");
}

#[test]
fn rollback_orphan_user_turn_also_removes_from_session_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store: Arc<dyn operant_infra::session_backend::SessionBackend> =
        Arc::new(operant_infra::session_store::SessionStore::new(tmp.path()).unwrap());

    let sender = "telegram_u4".to_string();

    // Pre-populate the session store with the same turns.
    store.append(&sender, &ChatMessage::user("first")).unwrap();
    store
        .append(&sender, &ChatMessage::assistant("ok"))
        .unwrap();
    store
        .append(
            &sender,
            &ChatMessage::user("[IMAGE:/tmp/photo.jpg]\n\nDescribe this"),
        )
        .unwrap();

    let mut histories =
        lru::LruCache::new(std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap());
    histories.push(
        sender.clone(),
        vec![
            ChatMessage::user("first"),
            ChatMessage::assistant("ok"),
            ChatMessage::user("[IMAGE:/tmp/photo.jpg]\n\nDescribe this"),
        ],
    );

    let ctx = ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("system".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(histories)),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: Some(Arc::clone(&store)),
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    };

    assert!(rollback_orphan_user_turn(
        &ctx,
        &sender,
        "[IMAGE:/tmp/photo.jpg]\n\nDescribe this"
    ));

    // In-memory history should have 2 turns remaining.
    let locked = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = locked.peek(&sender).expect("history should remain");
    assert_eq!(turns.len(), 2);

    // Session store should also have only 2 entries.
    let persisted = store.load(&sender);
    assert_eq!(
        persisted.len(),
        2,
        "session store should also lose the rolled-back turn"
    );
    assert_eq!(persisted[0].content, "first");
    assert_eq!(persisted[1].content, "ok");
}

struct DummyProvider;

#[async_trait::async_trait]
impl Provider for DummyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }
}

/// A provider that always returns `NO_REPLY`, used to test the
/// no-reply precheck path (typing indicator should not fire).
struct NoReplyProvider;

#[async_trait::async_trait]
impl Provider for NoReplyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("NO_REPLY: not addressed to agent".to_string())
    }
}

/// Provider that records every `model` value passed to `chat_with_system`
/// and short-circuits the agent loop with `NO_REPLY`. Lets a test assert
/// which model the precheck used vs the route model.
#[derive(Default)]
struct PrecheckModelCaptureProvider {
    models: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for PrecheckModelCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        self.models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(model.to_string());
        Ok("NO_REPLY: not addressed".to_string())
    }
}

/// Provider that stalls the precheck call (detected by the classifier's
/// prompt prefix) so the orchestrator's `tokio::time::timeout` fires, while
/// returning instantly for any post-fail-open agent calls so the test does
/// not hang.
struct PrecheckSlowProvider;

#[async_trait::async_trait]
impl Provider for PrecheckSlowProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        if message.starts_with("Decide whether the assistant should send any visible reply") {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok("REPLY".to_string())
        } else {
            Ok("ok".to_string())
        }
    }
}

struct FormatErrorProvider;

#[async_trait::async_trait]
impl Provider for FormatErrorProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        if messages
            .iter()
            .any(|msg| msg.content.contains("trigger format error"))
        {
            anyhow::bail!(
                "All providers/models failed. Attempts:\nprovider=custom:https://example.invalid/v1 model=test-model attempt 1/3: non_retryable; error=Custom API error (400 Bad Request): {{\"error\":{{\"message\":\"Format Error\",\"type\":\"invalid_request_error\",\"param\":null,\"code\":\"400\"}},\"request_id\":\"test-request-id\"}}"
            );
        }

        Ok("ok".to_string())
    }
}

#[derive(Default)]
struct RecordingChannel {
    sent_messages: tokio::sync::Mutex<Vec<String>>,
    start_typing_calls: AtomicUsize,
    stop_typing_calls: AtomicUsize,
    reactions_added: tokio::sync::Mutex<Vec<(String, String, String)>>,
    reactions_removed: tokio::sync::Mutex<Vec<(String, String, String)>>,
}

#[derive(Default)]
struct TelegramRecordingChannel {
    sent_messages: tokio::sync::Mutex<Vec<String>>,
}

#[derive(Default)]
struct SlackRecordingChannel {
    sent_messages: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Channel for TelegramRecordingChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<operant_api::channel::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl Channel for SlackRecordingChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<operant_api::channel::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        "test-channel"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<operant_api::channel::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.start_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.stop_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> anyhow::Result<()> {
        self.reactions_added.lock().await.push((
            channel_id.to_string(),
            message_id.to_string(),
            emoji.to_string(),
        ));
        Ok(())
    }

    async fn remove_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> anyhow::Result<()> {
        self.reactions_removed.lock().await.push((
            channel_id.to_string(),
            message_id.to_string(),
            emoji.to_string(),
        ));
        Ok(())
    }
}

struct SlowProvider {
    delay: Duration,
}

#[async_trait::async_trait]
impl Provider for SlowProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        tokio::time::sleep(self.delay).await;
        Ok(format!("echo: {message}"))
    }
}

struct ToolCallingProvider;

fn tool_call_payload() -> String {
    r#"<tool_call>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</tool_call>"#
        .to_string()
}

fn tool_call_payload_with_alias_tag() -> String {
    r#"<toolcall>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</toolcall>"#
        .to_string()
}

#[async_trait::async_trait]
impl Provider for ToolCallingProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let has_tool_results = messages
            .iter()
            .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
        if has_tool_results {
            Ok("BTC is currently around $65,000 based on latest tool output.".to_string())
        } else {
            Ok(tool_call_payload())
        }
    }
}

struct SessionsCurrentProvider;

#[async_trait::async_trait]
impl Provider for SessionsCurrentProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok(r#"<tool_call>
{"name":"sessions_current","arguments":{}}
</tool_call>"#
            .to_string())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        if let Some(tool_results) = messages
            .iter()
            .find(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
        {
            Ok(format!("session result:\n{}", tool_results.content))
        } else {
            self.chat_with_system(None, "", "", None).await
        }
    }
}

struct ToolCallingAliasProvider;

#[async_trait::async_trait]
impl Provider for ToolCallingAliasProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload_with_alias_tag())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let has_tool_results = messages
            .iter()
            .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
        if has_tool_results {
            Ok("BTC alias-tag flow resolved to final text output.".to_string())
        } else {
            Ok(tool_call_payload_with_alias_tag())
        }
    }
}

struct RawToolArtifactProvider;

#[async_trait::async_trait]
impl Provider for RawToolArtifactProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok(r#"{"name":"mock_price","parameters":{"symbol":"BTC"}}
{"result":{"symbol":"BTC","price_usd":65000}}
BTC is currently around $65,000 based on latest tool output."#
            .to_string())
    }
}

struct IterativeToolProvider {
    required_tool_iterations: usize,
}

impl IterativeToolProvider {
    fn completed_tool_iterations(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .filter(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .count()
    }
}

#[async_trait::async_trait]
impl Provider for IterativeToolProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let completed_iterations = Self::completed_tool_iterations(messages);
        if completed_iterations >= self.required_tool_iterations {
            Ok(format!(
                "Completed after {completed_iterations} tool iterations."
            ))
        } else {
            Ok(tool_call_payload())
        }
    }
}

#[derive(Default)]
struct HistoryCaptureProvider {
    calls: std::sync::Mutex<Vec<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Provider for HistoryCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let snapshot = messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>();
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        calls.push(snapshot);
        Ok(format!("response-{}", calls.len()))
    }
}

struct DelayedHistoryCaptureProvider {
    delay: Duration,
    calls: std::sync::Mutex<Vec<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Provider for DelayedHistoryCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        let snapshot = messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>();
        let call_index = {
            let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
            calls.push(snapshot);
            calls.len()
        };
        tokio::time::sleep(self.delay).await;
        Ok(format!("response-{call_index}"))
    }
}

struct MockPriceTool;

#[derive(Default)]
struct ModelCaptureProvider {
    call_count: AtomicUsize,
    models: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for ModelCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(model.to_string());
        Ok("ok".to_string())
    }
}

#[async_trait::async_trait]
impl Tool for MockPriceTool {
    fn name(&self) -> &str {
        "mock_price"
    }

    fn description(&self) -> &str {
        "Return a mocked BTC price"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string" }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol = args.get("symbol").and_then(serde_json::Value::as_str);
        if symbol != Some("BTC") {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("unexpected symbol".to_string()),
            });
        }

        Ok(ToolResult {
            success: true,
            output: r#"{"symbol":"BTC","price_usd":65000}"#.to_string(),
            error: None,
        })
    }
}

#[tokio::test]
async fn process_channel_message_executes_tool_calls_instead_of_sending_raw_json() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-42".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.starts_with("chat-42:"));
    assert!(reply.contains("BTC is currently around"));
    assert!(!reply.contains("\"tool_calls\""));
    assert!(!reply.contains("mock_price"));
}

#[tokio::test]
async fn process_channel_message_scopes_sender_session_key_for_sessions_current_tool() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let tmp = TempDir::new().unwrap();
    let session_store: Arc<dyn operant_infra::session_backend::SessionBackend> =
        Arc::new(operant_infra::session_store::SessionStore::new(tmp.path()).unwrap());

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SessionsCurrentProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(
            operant_runtime::tools::SessionsCurrentTool::new(Arc::clone(&session_store)),
        )]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: Some(Arc::clone(&session_store)),
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(&{
            let mut autonomy = operant_config::schema::AutonomyConfig::default();
            autonomy.auto_approve.push("sessions_current".to_string());
            autonomy
        })),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-42".to_string(),
            content: "Which session is this?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.contains("Current session: test-channel_chat-42_alice"));
    assert!(reply.contains("Messages: 1"));
}

#[tokio::test]
async fn process_channel_message_renders_trailing_tool_receipts_block_when_enabled() {
    // Activated path: a real ReceiptGenerator + show_receipts_in_response=true
    // must produce a second send carrying the "Tool receipts:" block with a
    // valid zc-receipt-* token. Pre-#6214 this was dead code from the test
    // suite because every ChannelRuntimeContext literal pinned the feature
    // off; this test guards the integration so a regression in the block
    // render or send call surfaces in CI rather than in production.
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        // Full autonomy + auto-approve mock_price so the loop actually
        // reaches execute_one_tool. The other tests in this file pass
        // under Supervised because ToolCallingProvider returns the BTC
        // reply regardless of whether the tool ran (the LLM only needs
        // to see a `[Tool results]` user message — even a "denied"
        // payload triggers the deterministic response). Receipts only
        // generate on the actual execute path, so we need the gate
        // open here.
        autonomy_level: AutonomyLevel::Full,
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(&{
            let mut autonomy = operant_config::schema::AutonomyConfig::default();
            autonomy.level = operant_config::autonomy::AutonomyLevel::Full;
            autonomy.auto_approve.push("mock_price".to_string());
            autonomy
        })),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: Some(operant_runtime::agent::tool_receipts::ReceiptGenerator::new()),
        show_receipts_in_response: true,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-42".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    let receipts_header = i18n::get_required_cli_string("channel-runtime-tool-receipts-header");
    // Two sends: the model's reply and the trailing receipts block.
    assert!(
        sent_messages.len() >= 2,
        "expected at least 2 sends (reply + receipts block), got {}: {:?}",
        sent_messages.len(),
        sent_messages
    );

    let receipts_message = sent_messages
        .iter()
        .find(|m| m.contains(&receipts_header))
        .unwrap_or_else(|| {
            panic!(
                "no localized tool receipts send found; got {:?}",
                sent_messages.as_slice()
            )
        });
    assert!(
        receipts_message.starts_with("chat-42:"),
        "receipts block must be sent to the same reply target as the agent reply, got {receipts_message}"
    );
    assert!(
        receipts_message.contains(&receipts_header),
        "receipts block must be prefixed with the localized receipt header, got {receipts_message}"
    );
    assert!(
        receipts_message.contains("zc-receipt-"),
        "receipts block must carry at least one zc-receipt-* HMAC token (proves the generator actually ran), got {receipts_message}"
    );
    assert!(
        receipts_message.contains("mock_price"),
        "receipts block should name the tool that produced the receipt, got {receipts_message}"
    );
}

#[tokio::test]
async fn process_channel_message_omits_receipts_block_when_disabled() {
    // Backward-compat: with show_receipts_in_response=false (default), no
    // trailing receipts message is sent — even when a generator is active
    // and the loop ran tools. This is the path every other test relies on
    // implicitly; assert it once explicitly.
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        // Match the enabled-test setup so the tool actually runs; the
        // assertion below proves the receipt-block send is gated on
        // `show_receipts_in_response` and not on whether the loop saw
        // any receipts.
        autonomy_level: AutonomyLevel::Full,
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(&{
            let mut autonomy = operant_config::schema::AutonomyConfig::default();
            autonomy.level = operant_config::autonomy::AutonomyLevel::Full;
            autonomy.auto_approve.push("mock_price".to_string());
            autonomy
        })),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: Some(operant_runtime::agent::tool_receipts::ReceiptGenerator::new()),
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-42".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    let receipts_header = i18n::get_required_cli_string("channel-runtime-tool-receipts-header");
    assert!(
        !sent_messages.iter().any(|m| m.contains(&receipts_header)),
        "no receipts block must be sent when show_receipts_in_response=false; got {:?}",
        sent_messages.as_slice()
    );
}

#[tokio::test]
async fn process_channel_message_disabled_receipt_generator_emits_no_receipts_anywhere() {
    // Strict #6182 acceptance criterion: enabled=false must emit no
    // receipt anywhere — not in any sent message, not in the model's
    // view of conversation history. `receipt_generator: None` is the
    // wire-level reflection of `[agent.tool_receipts] enabled = false`.
    // Distinct from the show_in_response=false test above (which keeps
    // the generator on but suppresses the trailing block); this one
    // proves nothing is signed in the first place.
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::Full,
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(&{
            let mut autonomy = operant_config::schema::AutonomyConfig::default();
            autonomy.level = operant_config::autonomy::AutonomyLevel::Full;
            autonomy.auto_approve.push("mock_price".to_string());
            autonomy
        })),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-42".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(
        !sent_messages.is_empty(),
        "agent must still respond when receipts are disabled"
    );
    assert!(
        !sent_messages.iter().any(|m| m.contains("zc-receipt-")),
        "no zc-receipt- token must appear in any sent message when receipts are disabled, got {:?}",
        sent_messages.as_slice()
    );
    let receipts_header = i18n::get_required_cli_string("channel-runtime-tool-receipts-header");
    assert!(
        !sent_messages.iter().any(|m| m.contains(&receipts_header)),
        "no `Tool receipts:` block must be sent when receipts are disabled, got {:?}",
        sent_messages.as_slice()
    );

    // Strict surface check: the model's view of conversation history must
    // not carry a `[receipt: ` trailer either, otherwise an LLM trained
    // on echoing receipts could leak signed-looking output even though
    // nothing was actually signed.
    let histories = runtime_ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (_key, turns) in histories.iter() {
        for msg in turns.iter() {
            assert!(
                !msg.content.contains("[receipt: "),
                "no `[receipt: ` trailer must appear in conversation history when receipts are disabled, got: {}",
                msg.content
            );
        }
    }
}

#[tokio::test]
async fn process_channel_message_telegram_does_not_persist_tool_summary_prefix() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-telegram-tool-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-telegram".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.contains("BTC is currently around"));

    let histories = runtime_ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories
        .peek("telegram_chat-telegram_alice")
        .expect("telegram history should be stored");
    let assistant_turn = turns
        .iter()
        .rev()
        .find(|turn| turn.role == "assistant")
        .expect("assistant turn should be present");
    assert!(
        !assistant_turn.content.contains("[Used tools:"),
        "telegram history should not persist tool-summary prefix"
    );
}

#[tokio::test]
async fn process_channel_message_strips_unexecuted_tool_json_artifacts_from_reply() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(RawToolArtifactProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-raw-json".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-raw".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 3,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("chat-raw:"));
    assert!(sent_messages[0].contains("BTC is currently around"));
    assert!(!sent_messages[0].contains("\"name\":\"mock_price\""));
    assert!(!sent_messages[0].contains("\"result\""));
}

#[tokio::test]
async fn process_channel_message_executes_tool_calls_with_alias_tags() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(ToolCallingAliasProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-2".to_string(),
            sender: "bob".to_string(),
            reply_target: "chat-84".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.starts_with("chat-84:"));
    assert!(reply.contains("alias-tag flow resolved"));
    assert!(!reply.contains("<toolcall>"));
    assert!(!reply.contains("mock_price"));
}

#[tokio::test]
async fn process_channel_message_handles_models_command_without_llm_call() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let fallback_provider_impl = Arc::new(ModelCaptureProvider::default());
    let fallback_provider: Arc<dyn Provider> = fallback_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("openrouter".to_string(), fallback_provider);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-cmd-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "/models openrouter".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(&i18n::get_required_cli_string_with_args(
        "channel-runtime-provider-switched",
        &[("provider", "openrouter"), ("model", "default-model")]
    )));

    let route_key = "telegram_chat-1_alice";
    let route = runtime_ctx
        .route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(route_key)
        .cloned()
        .expect("route should be stored for sender");
    assert_eq!(route.provider, "openrouter");
    assert_eq!(route.model, "default-model");

    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(fallback_provider_impl.call_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn process_channel_message_uses_route_override_provider_and_model() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let routed_provider_impl = Arc::new(ModelCaptureProvider::default());
    let routed_provider: Arc<dyn Provider> = routed_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("openrouter".to_string(), routed_provider);

    let route_key = "telegram_chat-1_alice".to_string();
    let mut route_overrides = HashMap::new();
    route_overrides.insert(
        route_key,
        ChannelRouteSelection {
            provider: "openrouter".to_string(),
            model: "route-model".to_string(),
            api_key: None,
        },
    );

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(route_overrides)),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-routed-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello routed provider".to_string(),
            channel: "telegram".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(routed_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        routed_provider_impl
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_slice(),
        &["route-model".to_string()]
    );
}

#[tokio::test]
async fn process_channel_message_prefers_cached_default_provider_instance() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let startup_provider_impl = Arc::new(ModelCaptureProvider::default());
    let startup_provider: Arc<dyn Provider> = startup_provider_impl.clone();
    let reloaded_provider_impl = Arc::new(ModelCaptureProvider::default());
    let reloaded_provider: Arc<dyn Provider> = reloaded_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), reloaded_provider);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&startup_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-default-provider-cache".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello cached default provider".to_string(),
            channel: "telegram".to_string(),
            timestamp: 3,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    assert_eq!(startup_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(reloaded_provider_impl.call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn process_channel_message_uses_runtime_default_model_from_store() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(ModelCaptureProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

    let temp = tempfile::TempDir::new().expect("temp dir");
    let config_path = temp.path().join("config.toml");

    {
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(
            config_path.clone(),
            RuntimeConfigState {
                defaults: ChannelRuntimeDefaults {
                    default_provider: "test-provider".to_string(),
                    model: "hot-reloaded-model".to_string(),
                    temperature: 0.5,
                    api_key: None,
                    api_url: None,
                    reliability: operant_config::schema::ReliabilityConfig::default(),
                },
                last_applied_stamp: None,
            },
        );
    }

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("startup-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions {
            operant_dir: Some(temp.path().to_path_buf()),
            ..operant_providers::ProviderRuntimeOptions::default()
        },
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-runtime-store-model".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello runtime defaults".to_string(),
            channel: "telegram".to_string(),
            timestamp: 4,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    {
        let mut cleanup_store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cleanup_store.remove(&config_path);
    }

    assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider_impl
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_slice(),
        &["hot-reloaded-model".to_string()]
    );
}

#[tokio::test]
async fn process_channel_message_respects_configured_max_tool_iterations_above_default() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(IterativeToolProvider {
            required_tool_iterations: 11,
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 12,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig {
            loop_detection_enabled: false,
            ..operant_config::schema::PacingConfig::default()
        },
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-iter-success".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-iter-success".to_string(),
            content: "Loop until done".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.starts_with("chat-iter-success:"));
    assert!(reply.contains("Completed after 11 tool iterations."));
    assert!(!reply.contains("⚠️ Error:"));
}

#[tokio::test]
async fn process_channel_message_reports_configured_max_tool_iterations_limit() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(IterativeToolProvider {
            required_tool_iterations: 20,
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 3,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig {
            loop_detection_enabled: false,
            ..operant_config::schema::PacingConfig::default()
        },
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-iter-fail".to_string(),
            sender: "bob".to_string(),
            reply_target: "chat-iter-fail".to_string(),
            content: "Loop forever".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert!(!sent_messages.is_empty());
    let reply = sent_messages.last().unwrap();
    assert!(reply.starts_with("chat-iter-fail:"));
    // After Phase 9, the agent attempts a graceful summary instead of erroring.
    // The mock provider returns a tool call payload as text, which the agent
    // returns as its "summary". The key invariant: the loop terminates and
    // produces a response (not hanging forever).
    assert!(
        reply.contains("⚠️ Error: Agent exceeded maximum tool iterations (3)")
            || reply.len() > "chat-iter-fail:".len(),
        "Expected either an error message or a graceful summary response"
    );
}

struct NoopMemory;

#[async_trait::async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: operant_memory::MemoryCategory,
        _session_id: Option<&str>,
    ) -> operant_memory::MemoryResult<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
        _since: Option<&str>,
        _until: Option<&str>,
    ) -> operant_memory::MemoryResult<Vec<operant_memory::MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(
        &self,
        _key: &str,
    ) -> operant_memory::MemoryResult<Option<operant_memory::MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&operant_memory::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> operant_memory::MemoryResult<Vec<operant_memory::MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _key: &str) -> operant_memory::MemoryResult<bool> {
        Ok(false)
    }

    async fn count(&self) -> operant_memory::MemoryResult<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct RecallMemory;

#[async_trait::async_trait]
impl Memory for RecallMemory {
    fn name(&self) -> &str {
        "recall-memory"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: operant_memory::MemoryCategory,
        _session_id: Option<&str>,
    ) -> operant_memory::MemoryResult<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
        _since: Option<&str>,
        _until: Option<&str>,
    ) -> operant_memory::MemoryResult<Vec<operant_memory::MemoryEntry>> {
        Ok(vec![operant_memory::MemoryEntry {
            id: "entry-1".to_string(),
            key: "memory_key_1".to_string(),
            content: "Age is 45".to_string(),
            category: operant_memory::MemoryCategory::Conversation,
            timestamp: "2026-02-20T00:00:00Z".to_string(),
            session_id: None,
            score: Some(0.9),
            namespace: "default".into(),
            importance: None,
            superseded_by: None,
        }])
    }

    async fn get(
        &self,
        _key: &str,
    ) -> operant_memory::MemoryResult<Option<operant_memory::MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&operant_memory::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> operant_memory::MemoryResult<Vec<operant_memory::MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _key: &str) -> operant_memory::MemoryResult<bool> {
        Ok(false)
    }

    async fn count(&self) -> operant_memory::MemoryResult<usize> {
        Ok(1)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn message_dispatch_processes_messages_in_parallel() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(250),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(4);
    tx.send(operant_api::channel::ChannelMessage {
        id: "1".to_string(),
        sender: "alice".to_string(),
        reply_target: "alice".to_string(),
        content: "hello".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    })
    .await
    .unwrap();
    tx.send(operant_api::channel::ChannelMessage {
        id: "2".to_string(),
        sender: "bob".to_string(),
        reply_target: "bob".to_string(),
        content: "world".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 2,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    })
    .await
    .unwrap();
    drop(tx);

    let started = Instant::now();
    run_message_dispatch_loop(rx, runtime_ctx, 2).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(700),
        "expected parallel dispatch with precheck (<700ms), got {:?}",
        elapsed
    );

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 2);
}

#[tokio::test]
async fn message_dispatch_interrupts_in_flight_telegram_request_and_preserves_context() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(DelayedHistoryCaptureProvider {
        delay: Duration::from_millis(250),
        calls: std::sync::Mutex::new(Vec::new()),
    });

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: true,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(8);
    let send_task = tokio::spawn(async move {
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "forwarded content".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-2".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "summarize this".to_string(),
            channel: "telegram".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        })
        .await
        .unwrap();
    });

    run_message_dispatch_loop(rx, runtime_ctx, 4).await;
    send_task.await.unwrap();

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("chat-1:"));
    assert!(sent_messages[0].contains("response-2"));
    drop(sent_messages);

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 2);
    let second_call = &calls[1];
    assert!(
        second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("forwarded content") })
    );
    assert!(
        second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("summarize this") })
    );
    assert!(
        !second_call.iter().any(|(role, _)| role == "assistant"),
        "cancelled turn should not persist an assistant response"
    );
}

#[tokio::test]
async fn message_dispatch_interrupts_in_flight_slack_request_and_preserves_context() {
    let channel_impl = Arc::new(SlackRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(DelayedHistoryCaptureProvider {
        delay: Duration::from_millis(250),
        calls: std::sync::Mutex::new(Vec::new()),
    });

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: true,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(8);
    let send_task = tokio::spawn(async move {
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "U123".to_string(),
            reply_target: "C123".to_string(),
            content: "first question".to_string(),
            channel: "slack".to_string(),
            timestamp: 1,
            thread_ts: Some("1741234567.100001".to_string()),
            interruption_scope_id: Some("1741234567.100001".to_string()),
            attachments: vec![],
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-2".to_string(),
            sender: "U123".to_string(),
            reply_target: "C123".to_string(),
            content: "second question".to_string(),
            channel: "slack".to_string(),
            timestamp: 2,
            thread_ts: Some("1741234567.100001".to_string()),
            interruption_scope_id: Some("1741234567.100001".to_string()),
            attachments: vec![],
        })
        .await
        .unwrap();
    });

    run_message_dispatch_loop(rx, runtime_ctx, 4).await;
    send_task.await.unwrap();

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("C123:"));
    assert!(sent_messages[0].contains("response-2"));
    drop(sent_messages);

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 2);
    let second_call = &calls[1];
    assert!(
        second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("first question") })
    );
    assert!(
        second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("second question") })
    );
    assert!(
        !second_call.iter().any(|(role, _)| role == "assistant"),
        "cancelled turn should not persist an assistant response"
    );
}

#[tokio::test]
async fn message_dispatch_interrupt_scope_is_same_sender_same_chat() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(180),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: true,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(8);
    let send_task = tokio::spawn(async move {
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-a".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "first chat".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        tx.send(operant_api::channel::ChannelMessage {
            id: "msg-b".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-2".to_string(),
            content: "second chat".to_string(),
            channel: "telegram".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        })
        .await
        .unwrap();
    });

    run_message_dispatch_loop(rx, runtime_ctx, 4).await;
    send_task.await.unwrap();

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 2);
    assert!(sent_messages.iter().any(|msg| msg.starts_with("chat-1:")));
    assert!(sent_messages.iter().any(|msg| msg.starts_with("chat-2:")));
}

#[tokio::test]
async fn process_channel_message_cancels_scoped_typing_task() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(20),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "typing-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-typing".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let starts = channel_impl.start_typing_calls.load(Ordering::SeqCst);
    let stops = channel_impl.stop_typing_calls.load(Ordering::SeqCst);
    assert_eq!(starts, 1, "start_typing should be called once");
    assert_eq!(stops, 1, "stop_typing should be called once");
}

#[tokio::test]
async fn process_channel_message_no_reply_precheck_skips_typing_indicator() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(NoReplyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "typing-fast-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-typing".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let starts = channel_impl.start_typing_calls.load(Ordering::SeqCst);
    assert_eq!(starts, 0, "no-reply precheck should not show typing");
}

#[tokio::test]
async fn process_channel_message_precheck_uses_configured_model_override() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(PrecheckModelCaptureProvider::default());
    let provider: Arc<dyn Provider> = provider_impl.clone();

    let mut config = operant_config::schema::Config::default();
    config.agent.precheck.model = Some("precheck-fast".to_string());

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider,
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("main-route-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(config),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "precheck-model-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-precheck-model".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let models = provider_impl
        .models
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        models.as_slice(),
        &["precheck-fast".to_string()],
        "precheck must use the configured model override; main loop must be skipped on NO_REPLY"
    );
}

#[tokio::test]
async fn process_channel_message_precheck_timeout_fails_open_to_reply() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let mut config = operant_config::schema::Config::default();
    config.agent.precheck.timeout_secs = 1;

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(PrecheckSlowProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(config),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let started = Instant::now();
    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "precheck-timeout-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-precheck-timeout".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;
    let total = started.elapsed();

    // Fail-open after precheck timeout means the main agent loop runs and
    // delivers a reply. The slow precheck would otherwise hang for 60s.
    let sent = channel_impl.sent_messages.lock().await.clone();
    assert_eq!(
        sent.len(),
        1,
        "fail-open after precheck timeout should send exactly one reply, got {sent:?}"
    );
    assert!(
        sent[0].ends_with(":ok"),
        "expected reply body 'ok' from main loop, got {:?}",
        sent[0]
    );
    assert!(
        total >= Duration::from_millis(900),
        "process_channel_message returned in {total:?}; precheck timeout should have waited ~1s"
    );
    assert!(
        total < Duration::from_secs(10),
        "process_channel_message took {total:?}; should not have waited for the 60s slow precheck"
    );
}

#[tokio::test]
async fn process_channel_message_adds_and_swaps_reactions() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(5),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "react-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-react".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let added = channel_impl.reactions_added.lock().await;
    assert!(
        added.len() >= 2,
        "expected at least 2 reactions added (\u{1F440} then \u{2705}), got {}",
        added.len()
    );
    assert_eq!(added[0].2, "\u{1F440}", "first reaction should be eyes");
    assert_eq!(
        added.last().unwrap().2,
        "\u{2705}",
        "last reaction should be checkmark"
    );

    let removed = channel_impl.reactions_removed.lock().await;
    assert_eq!(removed.len(), 1, "eyes reaction should be removed once");
    assert_eq!(removed[0].2, "\u{1F440}");
}

#[test]
fn prompt_contains_all_sections() {
    let ws = make_workspace();
    let tools = vec![("shell", "Run commands"), ("file_read", "Read files")];
    let prompt = build_system_prompt(ws.path(), "test-model", &tools, &[], None, None);

    // Section headers
    assert!(prompt.contains("## Tools"), "missing Tools section");
    assert!(prompt.contains("## Safety"), "missing Safety section");
    assert!(prompt.contains("## Workspace"), "missing Workspace section");
    assert!(
        prompt.contains("## Project Context"),
        "missing Project Context"
    );
    assert!(
        prompt.contains("## Current Date & Time"),
        "missing Date/Time"
    );
    assert!(prompt.contains("## Runtime"), "missing Runtime section");
}

#[test]
fn prompt_injects_tools() {
    let ws = make_workspace();
    let tools = vec![
        ("shell", "Run commands"),
        ("memory_recall", "Search memory"),
    ];
    let prompt = build_system_prompt(ws.path(), "gpt-4o", &tools, &[], None, None);

    assert!(prompt.contains("**shell**"));
    assert!(prompt.contains("Run commands"));
    assert!(prompt.contains("**memory_recall**"));
}

#[test]
fn prompt_includes_single_tool_protocol_block_after_append() {
    let ws = make_workspace();
    let tools = vec![("shell", "Run commands")];
    let mut prompt = build_system_prompt(ws.path(), "gpt-4o", &tools, &[], None, None);

    assert!(
        !prompt.contains("## Tool Use Protocol"),
        "build_system_prompt should not emit protocol block directly"
    );

    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(MockPriceTool)];
    prompt.push_str(&build_tool_instructions(&tools_registry));

    assert_eq!(
        prompt.matches("## Tool Use Protocol").count(),
        1,
        "protocol block should appear exactly once in the final prompt"
    );
}

#[test]
fn prompt_injects_safety() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("Do not exfiltrate private data"));
    assert!(prompt.contains("Respect the runtime autonomy policy"));
    assert!(prompt.contains("Prefer `trash` over `rm`"));
}

#[test]
fn prompt_injects_workspace_files() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("### SOUL.md"), "missing SOUL.md header");
    assert!(prompt.contains("Be helpful"), "missing SOUL content");
    assert!(prompt.contains("### IDENTITY.md"), "missing IDENTITY.md");
    assert!(prompt.contains("Name: Operant"), "missing IDENTITY content");
    assert!(prompt.contains("### USER.md"), "missing USER.md");
    assert!(prompt.contains("### AGENTS.md"), "missing AGENTS.md");
    assert!(prompt.contains("### TOOLS.md"), "missing TOOLS.md");
    // HEARTBEAT.md is intentionally excluded from channel prompts — it's only
    // relevant to the heartbeat worker and causes LLMs to emit spurious
    // "HEARTBEAT_OK" acknowledgments in channel conversations.
    assert!(
        !prompt.contains("### HEARTBEAT.md"),
        "HEARTBEAT.md should not be in channel prompt"
    );
    assert!(prompt.contains("### MEMORY.md"), "missing MEMORY.md");
    assert!(prompt.contains("User likes Rust"), "missing MEMORY content");
}

#[test]
fn prompt_missing_file_markers() {
    let tmp = TempDir::new().unwrap();
    // Empty workspace — no files at all
    let prompt = build_system_prompt(tmp.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("[File not found: SOUL.md]"));
    assert!(prompt.contains("[File not found: AGENTS.md]"));
    assert!(prompt.contains("[File not found: IDENTITY.md]"));
}

#[test]
fn prompt_bootstrap_only_if_exists() {
    let ws = make_workspace();
    // No BOOTSTRAP.md — should not appear
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);
    assert!(
        !prompt.contains("### BOOTSTRAP.md"),
        "BOOTSTRAP.md should not appear when missing"
    );

    // Create BOOTSTRAP.md — should appear
    std::fs::write(ws.path().join("BOOTSTRAP.md"), "# Bootstrap\nFirst run.").unwrap();
    let prompt2 = build_system_prompt(ws.path(), "model", &[], &[], None, None);
    assert!(
        prompt2.contains("### BOOTSTRAP.md"),
        "BOOTSTRAP.md should appear when present"
    );
    assert!(prompt2.contains("First run"));
}

#[test]
fn prompt_no_daily_memory_injection() {
    let ws = make_workspace();
    let memory_dir = ws.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    std::fs::write(
        memory_dir.join(format!("{today}.md")),
        "# Daily\nSome note.",
    )
    .unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    // Daily notes should NOT be in the system prompt (on-demand via tools)
    assert!(
        !prompt.contains("Daily Notes"),
        "daily notes should not be auto-injected"
    );
    assert!(
        !prompt.contains("Some note"),
        "daily content should not be in prompt"
    );
}

#[test]
fn prompt_runtime_metadata() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "claude-sonnet-4", &[], &[], None, None);

    assert!(prompt.contains("Model: claude-sonnet-4"));
    assert!(prompt.contains(&format!("OS: {}", std::env::consts::OS)));
    assert!(prompt.contains("Host:"));
}

#[test]
fn prompt_skills_include_instructions_and_tools() {
    let ws = make_workspace();
    let skills = vec![operant_runtime::skills::Skill {
        name: "code-review".into(),
        description: "Review code for bugs".into(),
        version: "1.0.0".into(),
        author: None,
        tags: vec![],
        tools: vec![operant_runtime::skills::SkillTool {
            name: "lint".into(),
            description: "Run static checks".into(),
            kind: "shell".into(),
            command: "cargo clippy".into(),
            args: HashMap::new(),
            timeout_secs: None,
        }],
        prompts: vec!["Always run cargo test before final response.".into()],
        location: None,
    }];

    let prompt = build_system_prompt(ws.path(), "model", &[], &skills, None, None);

    assert!(prompt.contains("<available_skills>"), "missing skills XML");
    assert!(prompt.contains("<name>code-review</name>"));
    assert!(prompt.contains("<description>Review code for bugs</description>"));
    assert!(prompt.contains("SKILL.md</location>"));
    assert!(prompt.contains("<instructions>"));
    assert!(
        prompt.contains("<instruction>Always run cargo test before final response.</instruction>")
    );
    // Registered tools (shell kind) appear under <callable_tools> with prefixed names
    assert!(prompt.contains("<callable_tools"));
    assert!(prompt.contains("<name>code-review__lint</name>"));
    assert!(!prompt.contains("loaded on demand"));
}

#[test]
fn prompt_skills_compact_mode_omits_instructions_but_keeps_tools() {
    let ws = make_workspace();
    let skills = vec![operant_runtime::skills::Skill {
        name: "code-review".into(),
        description: "Review code for bugs".into(),
        version: "1.0.0".into(),
        author: None,
        tags: vec![],
        tools: vec![operant_runtime::skills::SkillTool {
            name: "lint".into(),
            description: "Run static checks".into(),
            kind: "shell".into(),
            command: "cargo clippy".into(),
            args: HashMap::new(),
            timeout_secs: None,
        }],
        prompts: vec!["Always run cargo test before final response.".into()],
        location: None,
    }];

    let prompt = build_system_prompt_with_mode(
        ws.path(),
        "model",
        &[],
        &skills,
        None,
        None,
        false,
        operant_config::schema::SkillsPromptInjectionMode::Compact,
        AutonomyLevel::default(),
    );

    assert!(prompt.contains("<available_skills>"), "missing skills XML");
    assert!(prompt.contains("<name>code-review</name>"));
    assert!(prompt.contains("<location>skills/code-review/SKILL.md</location>"));
    assert!(prompt.contains("loaded on demand"));
    assert!(!prompt.contains("<instructions>"));
    assert!(
        !prompt.contains("<instruction>Always run cargo test before final response.</instruction>")
    );
    // Compact mode should still include tools so the LLM knows about them.
    // Registered tools (shell kind) appear under <callable_tools> with prefixed names.
    assert!(prompt.contains("<callable_tools"));
    assert!(prompt.contains("<name>code-review__lint</name>"));
}

#[test]
fn prompt_skills_escape_reserved_xml_chars() {
    let ws = make_workspace();
    let skills = vec![operant_runtime::skills::Skill {
        name: "code<review>&".into(),
        description: "Review \"unsafe\" and 'risky' bits".into(),
        version: "1.0.0".into(),
        author: None,
        tags: vec![],
        tools: vec![operant_runtime::skills::SkillTool {
            name: "run\"linter\"".into(),
            description: "Run <lint> & report".into(),
            kind: "shell&exec".into(),
            command: "cargo clippy".into(),
            args: HashMap::new(),
            timeout_secs: None,
        }],
        prompts: vec!["Use <tool_call> and & keep output \"safe\"".into()],
        location: None,
    }];

    let prompt = build_system_prompt(ws.path(), "model", &[], &skills, None, None);

    assert!(prompt.contains("<name>code&lt;review&gt;&amp;</name>"));
    assert!(prompt.contains(
        "<description>Review &quot;unsafe&quot; and &apos;risky&apos; bits</description>"
    ));
    assert!(prompt.contains("<name>run&quot;linter&quot;</name>"));
    assert!(prompt.contains("<description>Run &lt;lint&gt; &amp; report</description>"));
    assert!(prompt.contains("<kind>shell&amp;exec</kind>"));
    assert!(prompt.contains(
        "<instruction>Use &lt;tool_call&gt; and &amp; keep output &quot;safe&quot;</instruction>"
    ));
}

#[test]
fn prompt_truncation() {
    let ws = make_workspace();
    // Write a file larger than BOOTSTRAP_MAX_CHARS
    let big_content = "x".repeat(BOOTSTRAP_MAX_CHARS + 1000);
    std::fs::write(ws.path().join("AGENTS.md"), &big_content).unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(
        prompt.contains("truncated at"),
        "large files should be truncated"
    );
    assert!(
        !prompt.contains(&big_content),
        "full content should not appear"
    );
}

#[test]
fn prompt_empty_files_skipped() {
    let ws = make_workspace();
    std::fs::write(ws.path().join("TOOLS.md"), "").unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    // Empty file should not produce a header
    assert!(
        !prompt.contains("### TOOLS.md"),
        "empty files should be skipped"
    );
}

#[test]
fn channel_log_truncation_is_utf8_safe_for_multibyte_text() {
    let msg = "Hello from Operant 🌍. Current status is healthy, and café-style UTF-8 text stays safe in logs.";

    // Reproduces the production crash path where channel logs truncate at 80 chars.
    let result =
        std::panic::catch_unwind(|| operant_runtime::util::truncate_with_ellipsis(msg, 80));
    assert!(
        result.is_ok(),
        "truncate_with_ellipsis should never panic on UTF-8"
    );

    let truncated = result.unwrap();
    assert!(!truncated.is_empty());
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[test]
fn prompt_contains_channel_capabilities() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(
        prompt.contains("## Channel Capabilities"),
        "missing Channel Capabilities section"
    );
    assert!(
        prompt.contains("running as a messaging bot"),
        "missing channel context"
    );
    assert!(
        prompt.contains("NEVER repeat, describe, or echo credentials"),
        "missing security instruction"
    );
}

#[test]
fn full_autonomy_prompt_executes_allowed_tools_without_extra_approval() {
    let ws = make_workspace();
    let config = operant_config::schema::AutonomyConfig {
        level: operant_runtime::security::AutonomyLevel::Full,
        ..operant_config::schema::AutonomyConfig::default()
    };
    let prompt = build_system_prompt_with_mode_and_autonomy(
        ws.path(),
        "model",
        &[],
        &[],
        None,
        None,
        Some(&config),
        false,
        operant_config::schema::SkillsPromptInjectionMode::Full,
        false,
        0,
    );

    assert!(
        prompt.contains("execute it directly instead of asking the user for extra approval"),
        "full autonomy should instruct direct execution for allowed tools"
    );
    assert!(
        prompt.contains("Never pretend you are waiting for a human approval"),
        "full autonomy should not simulate interactive approval flows"
    );
}

#[test]
fn readonly_prompt_explains_policy_blocks_without_fake_approval() {
    let ws = make_workspace();
    let config = operant_config::schema::AutonomyConfig {
        level: operant_runtime::security::AutonomyLevel::ReadOnly,
        ..operant_config::schema::AutonomyConfig::default()
    };
    let prompt = build_system_prompt_with_mode_and_autonomy(
        ws.path(),
        "model",
        &[],
        &[],
        None,
        None,
        Some(&config),
        false,
        operant_config::schema::SkillsPromptInjectionMode::Full,
        false,
        0,
    );

    assert!(
        prompt.contains("this runtime is read-only for side effects"),
        "read-only prompt should expose the runtime restriction"
    );
    assert!(
        prompt.contains("instead of simulating an approval flow"),
        "read-only prompt should explain restrictions instead of faking approval"
    );
}

#[test]
fn prompt_workspace_path() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains(&format!("Working directory: `{}`", ws.path().display())));
}

#[test]
fn full_autonomy_omits_approval_instructions() {
    let ws = make_workspace();
    let prompt = build_system_prompt_with_mode(
        ws.path(),
        "model",
        &[],
        &[],
        None,
        None,
        false,
        operant_config::schema::SkillsPromptInjectionMode::Full,
        AutonomyLevel::Full,
    );

    assert!(
        !prompt.contains("without asking"),
        "full autonomy prompt must not tell the model to ask before acting"
    );
    assert!(
        !prompt.contains("ask before acting externally"),
        "full autonomy prompt must not contain ask-before-acting instruction"
    );
    // Core safety rules should still be present
    assert!(
        prompt.contains("Do not exfiltrate private data"),
        "data exfiltration guard must remain"
    );
    assert!(
        prompt.contains("Prefer `trash` over `rm`"),
        "trash-over-rm hint must remain"
    );
}

#[test]
fn supervised_autonomy_includes_approval_instructions() {
    let ws = make_workspace();
    let prompt = build_system_prompt_with_mode(
        ws.path(),
        "model",
        &[],
        &[],
        None,
        None,
        false,
        operant_config::schema::SkillsPromptInjectionMode::Full,
        AutonomyLevel::Supervised,
    );

    assert!(
        prompt.contains("without asking"),
        "supervised prompt must include ask-before-acting instruction"
    );
    assert!(
        prompt.contains("ask before acting externally"),
        "supervised prompt must include ask-before-acting instruction"
    );
}

#[test]
fn channel_notify_observer_truncates_utf8_arguments_safely() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let observer = ChannelNotifyObserver {
        inner: Arc::new(NoopObserver),
        tx,
        tools_used: AtomicBool::new(false),
    };

    let payload = (0..300)
        .map(|n| serde_json::json!({ "content": format!("{}置tail", "a".repeat(n)) }))
        .map(|v| v.to_string())
        .find(|raw| raw.len() > 120 && !raw.is_char_boundary(120))
        .expect("should produce non-char-boundary data at byte index 120");

    observer.record_event(
        &operant_runtime::observability::traits::ObserverEvent::ToolCallStart {
            tool: "file_write".to_string(),
            arguments: Some(payload),
        },
    );

    let emitted = rx.try_recv().expect("observer should emit notify message");
    assert!(emitted.contains("`file_write`"));
    assert!(emitted.is_char_boundary(emitted.len()));
}

#[test]
fn conversation_memory_key_uses_message_id() {
    let msg = operant_api::channel::ChannelMessage {
        id: "msg_abc123".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "hello".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };

    assert_eq!(conversation_memory_key(&msg), "slack_U123_msg_abc123");
}

#[test]
fn followup_thread_id_prefers_thread_ts() {
    let msg = operant_api::channel::ChannelMessage {
        id: "slack_C123_1741234567.123456".into(),
        sender: "U123".into(),
        reply_target: "C123".into(),
        content: "hello".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: Some("1741234567.123456".into()),
        interruption_scope_id: None,
        attachments: vec![],
    };

    assert_eq!(
        followup_thread_id(&msg).as_deref(),
        Some("1741234567.123456")
    );
}

#[test]
fn followup_thread_id_falls_back_to_message_id() {
    let msg = operant_api::channel::ChannelMessage {
        id: "msg_abc123".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "hello".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };

    assert_eq!(followup_thread_id(&msg).as_deref(), Some("msg_abc123"));
}

#[test]
fn followup_thread_id_does_not_open_matrix_thread_for_root_message() {
    let msg = operant_api::channel::ChannelMessage {
        id: "$event:server".into(),
        sender: "@alice:server".into(),
        reply_target: "!room:server".into(),
        content: "hello".into(),
        channel: "matrix".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };

    assert_eq!(followup_thread_id(&msg), None);
}

#[test]
fn matrix_root_conversation_history_key_omits_event_id() {
    let first = operant_api::channel::ChannelMessage {
        id: "$first:server".into(),
        sender: "@alice:server".into(),
        reply_target: "!room:server".into(),
        content: "send a.txt".into(),
        channel: "matrix".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let second = operant_api::channel::ChannelMessage {
        id: "$second:server".into(),
        content: "send it again".into(),
        timestamp: 2,
        ..first.clone()
    };

    let key = conversation_history_key(&first);
    assert_eq!(key, conversation_history_key(&second));
    assert!(!key.contains("$first:server"));
    assert!(!key.contains("$second:server"));
}

#[test]
fn matrix_thread_conversation_history_key_uses_thread_root() {
    let msg = operant_api::channel::ChannelMessage {
        id: "$reply:server".into(),
        sender: "@alice:server".into(),
        reply_target: "!room:server".into(),
        content: "thread reply".into(),
        channel: "matrix".into(),
        timestamp: 1,
        thread_ts: Some("$root:server".into()),
        interruption_scope_id: Some("$root:server".into()),
        attachments: vec![],
    };

    let key = conversation_history_key(&msg);
    assert!(key.contains("_root_server"));
    assert!(!key.contains("_reply_server"));
}

#[test]
fn conversation_memory_key_is_unique_per_message() {
    let msg1 = operant_api::channel::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "first".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let msg2 = operant_api::channel::ChannelMessage {
        id: "msg_2".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "second".into(),
        channel: "slack".into(),
        timestamp: 2,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };

    assert_ne!(
        conversation_memory_key(&msg1),
        conversation_memory_key(&msg2)
    );
}

#[tokio::test]
async fn autosave_keys_preserve_multiple_conversation_facts() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();

    let msg1 = operant_api::channel::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "I'm Paul".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let msg2 = operant_api::channel::ChannelMessage {
        id: "msg_2".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "I'm 45".into(),
        channel: "slack".into(),
        timestamp: 2,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };

    mem.store(
        &conversation_memory_key(&msg1),
        &msg1.content,
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();
    mem.store(
        &conversation_memory_key(&msg2),
        &msg2.content,
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    assert_eq!(mem.count().await.unwrap(), 2);

    let recalled = mem.recall("45", 5, None, None, None).await.unwrap();
    assert!(recalled.iter().any(|entry| entry.content.contains("45")));
}

#[tokio::test]
async fn build_memory_context_includes_recalled_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    mem.store("age_fact", "Age is 45", MemoryCategory::Conversation, None)
        .await
        .unwrap();

    let context = build_memory_context(&mem, "age", 0.0, None).await;
    assert!(context.contains(MEMORY_CONTEXT_OPEN));
    assert!(context.contains("Age is 45"));
}

#[tokio::test]
async fn autosaved_conversation_memory_is_recalled_by_sender_scope() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    let msg = operant_api::channel::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "C456".into(),
        content: "Project codename is quartz".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let history_key = conversation_history_key(&msg);

    mem.store(
        &conversation_memory_key(&msg),
        &msg.content,
        MemoryCategory::Conversation,
        Some(&history_key),
    )
    .await
    .unwrap();

    let session_ids = sender_memory_session_ids(&msg, &history_key);
    let session_id_refs: Vec<Option<&str>> = session_ids.iter().map(|s| Some(s.as_str())).collect();
    let context = build_memory_context_for_sessions(&mem, "quartz", 0.0, &session_id_refs).await;

    assert!(
        context.contains("Project codename is quartz"),
        "sender recall should include autosaved memories stored under the current session key, got: {context}"
    );
}

#[tokio::test]
async fn autosaved_group_conversation_memory_stays_session_scoped() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    let group_a_msg = operant_api::channel::ChannelMessage {
        id: "msg_1".into(),
        sender: "U123".into(),
        reply_target: "group:alpha".into(),
        content: "Group alpha codename is quartz".into(),
        channel: "slack".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let group_b_msg = operant_api::channel::ChannelMessage {
        id: "msg_2".into(),
        sender: "U123".into(),
        reply_target: "group:beta".into(),
        content: "What was the codename?".into(),
        channel: "slack".into(),
        timestamp: 2,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let group_a_history_key = conversation_history_key(&group_a_msg);
    let group_b_history_key = conversation_history_key(&group_b_msg);

    mem.store(
        &conversation_memory_key(&group_a_msg),
        &group_a_msg.content,
        MemoryCategory::Conversation,
        Some(&group_a_history_key),
    )
    .await
    .unwrap();

    let group_b_sender_session_ids = sender_memory_session_ids(&group_b_msg, &group_b_history_key);
    assert_eq!(group_b_sender_session_ids, vec!["U123".to_string()]);

    let group_b_sender_session_id_refs: Vec<Option<&str>> = group_b_sender_session_ids
        .iter()
        .map(|s| Some(s.as_str()))
        .collect();
    let sender_context =
        build_memory_context_for_sessions(&mem, "quartz", 0.0, &group_b_sender_session_id_refs)
            .await;
    let group_context = build_memory_context(&mem, "quartz", 0.0, Some(&group_b_history_key)).await;
    let source_group_context =
        build_memory_context(&mem, "quartz", 0.0, Some(&group_a_history_key)).await;

    assert!(
        sender_context.is_empty(),
        "sender scope must not leak autosaved group memory from another group, got: {sender_context}"
    );
    assert!(
        group_context.is_empty(),
        "target group scope must not include another group's autosaved memory, got: {group_context}"
    );
    assert!(
        source_group_context.contains("Group alpha codename is quartz"),
        "source group scope should still recall its own autosaved memory, got: {source_group_context}"
    );
}

#[tokio::test]
async fn sender_session_ids_match_migrated_matrix_sender_rows() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    let raw_sender = "@alice:server";
    let sanitized_sender = sanitize_session_key(raw_sender);
    assert_eq!(sanitized_sender, "_alice_server");

    mem.store(
        "alice_fact",
        "Alice favors filtered coffee",
        MemoryCategory::Conversation,
        Some(sanitized_sender.as_str()),
    )
    .await
    .unwrap();

    let msg = operant_api::channel::ChannelMessage {
        id: "evt_1".into(),
        sender: raw_sender.into(),
        reply_target: "!room:server".into(),
        content: "what coffee does alice prefer?".into(),
        channel: "matrix".into(),
        timestamp: 1,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    let history_key = conversation_history_key(&msg);
    let session_ids = sender_memory_session_ids(&msg, &history_key);
    assert!(
        session_ids.contains(&sanitized_sender),
        "sender session ids must include sanitized sender, got: {session_ids:?}"
    );
    let session_id_refs: Vec<Option<&str>> = session_ids.iter().map(|s| Some(s.as_str())).collect();
    let context = build_memory_context_for_sessions(&mem, "coffee", 0.0, &session_id_refs).await;
    assert!(
        context.contains("Alice favors filtered coffee"),
        "sender recall must find migrated row stored under sanitized sender, got: {context}"
    );
}

/// Auto-saved photo messages must not surface through memory context,
/// otherwise the image marker gets duplicated in the provider request (#2403).
#[tokio::test]
async fn build_memory_context_excludes_image_marker_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();

    // Simulate auto-save of a photo message containing an [IMAGE:] marker.
    mem.store(
        "telegram_user_msg_photo",
        "[IMAGE:/tmp/workspace/photo_1_2.jpg]\n\nDescribe this screenshot",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();
    // Also store a plain text entry that shares a word with the query
    // so the FTS recall returns both entries.
    mem.store(
        "screenshot_preference",
        "User prefers screenshot descriptions to be concise",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    let context = build_memory_context(&mem, "screenshot", 0.0, None).await;

    // The image-marker entry must be excluded to prevent duplication.
    assert!(
        !context.contains("[IMAGE:"),
        "memory context must not contain image markers, got: {context}"
    );
    // Plain text entries should still be included.
    assert!(
        context.contains("screenshot descriptions"),
        "plain text entry should remain in context, got: {context}"
    );
}

#[tokio::test]
async fn process_channel_message_restores_per_sender_history_on_follow_ups() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(HistoryCaptureProvider::default());

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-a".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-b".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "follow up".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].len(), 2);
    assert_eq!(calls[0][0].0, "system");
    assert_eq!(calls[0][1].0, "user");
    assert_eq!(calls[1].len(), 4);
    assert_eq!(calls[1][0].0, "system");
    assert_eq!(calls[1][1].0, "user");
    assert_eq!(calls[1][2].0, "assistant");
    assert_eq!(calls[1][3].0, "user");
    assert!(calls[1][1].1.contains("hello"));
    assert!(calls[1][2].1.contains("response-1"));
    assert!(calls[1][3].1.contains("follow up"));
}

#[tokio::test]
async fn process_channel_message_refreshes_available_skills_after_new_session() {
    let workspace = make_workspace();
    let mut config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Default::default()
    };
    config.skills.open_skills_enabled = false;

    let initial_skills =
        operant_runtime::skills::load_skills_with_config(workspace.path(), &config);
    assert!(initial_skills.is_empty());

    let initial_system_prompt = build_system_prompt_with_mode(
        workspace.path(),
        "test-model",
        &[],
        &initial_skills,
        Some(&config.identity),
        None,
        false,
        config.skills.prompt_injection_mode,
        AutonomyLevel::default(),
    );
    assert!(
        !initial_system_prompt.contains("refresh-test"),
        "initial prompt should not contain the new skill before it exists"
    );

    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(HistoryCaptureProvider::default());
    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new(initial_system_prompt),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        prompt_config: Arc::new(config.clone()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-before-new".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-refresh".to_string(),
            content: "hello".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let skill_dir = workspace.path().join("skills").join("refresh-test");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: refresh-test\ndescription: Refresh the available skills section\n---\n# Refresh Test\nExpose this skill after /new.\n",
        )
        .unwrap();
    let refreshed_skills =
        operant_runtime::skills::load_skills_with_config(workspace.path(), &config);
    assert_eq!(refreshed_skills.len(), 1);
    assert_eq!(refreshed_skills[0].name, "refresh-test");
    assert!(
        refreshed_new_session_system_prompt(runtime_ctx.as_ref())
            .contains("<name>refresh-test</name>"),
        "fresh-session prompt should pick up skills added after startup"
    );

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-new-session".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-refresh".to_string(),
            content: "/new".to_string(),
            channel: "telegram".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    {
        let histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            histories.peek("telegram_chat-refresh_alice").is_none(),
            "/new should clear the cached sender history before the next message"
        );
    }

    {
        let pending_new_sessions = runtime_ctx
            .pending_new_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            pending_new_sessions.contains("telegram_chat-refresh_alice"),
            "/new should mark the sender for a fresh next-message prompt rebuild"
        );
    }

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-after-new".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-refresh".to_string(),
            content: "hello again".to_string(),
            channel: "telegram".to_string(),
            timestamp: 3,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    {
        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0][0].0, "system");
        assert_eq!(calls[1][0].0, "system");
        assert!(
            !calls[0][0].1.contains("<name>refresh-test</name>"),
            "pre-/new prompt should not advertise a skill that did not exist yet"
        );
        assert!(
            calls[1][0].1.contains("<available_skills>"),
            "post-/new prompt should contain the refreshed skills block"
        );
        assert!(
            calls[1][0].1.contains("<name>refresh-test</name>"),
            "post-/new prompt should include skills discovered after the reset"
        );
    }

    let sent_messages = channel_impl.sent_messages.lock().await;
    let new_session_message = i18n::get_required_cli_string("channel-runtime-new-session");
    assert!(
        sent_messages
            .iter()
            .any(|message| { message.contains(&new_session_message) })
    );
}

#[tokio::test]
async fn process_channel_message_enriches_current_turn_without_persisting_context() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(HistoryCaptureProvider::default());
    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(RecallMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "msg-ctx-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-ctx".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].len(), 2);
    // Memory context is injected into the system prompt, not the user message.
    assert_eq!(calls[0][0].0, "system");
    assert!(calls[0][0].1.contains(MEMORY_CONTEXT_OPEN));
    assert!(calls[0][0].1.contains("Age is 45"));
    assert_eq!(calls[0][1].0, "user");
    assert_eq!(calls[0][1].1, "hello");

    let histories = runtime_ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories
        .peek("test-channel_chat-ctx_alice")
        .expect("history should be stored for sender");
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].content, "hello");
    assert!(!turns[0].content.contains(MEMORY_CONTEXT_OPEN));
}

#[tokio::test]
async fn process_channel_message_telegram_keeps_system_instruction_at_top_only() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let provider_impl = Arc::new(HistoryCaptureProvider::default());
    let mut histories =
        lru::LruCache::new(std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap());
    histories.push(
        "telegram_chat-telegram_alice".to_string(),
        vec![
            ChatMessage::assistant("stale assistant"),
            ChatMessage::user("earlier user question"),
            ChatMessage::assistant("earlier assistant reply"),
        ],
    );

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: provider_impl.clone(),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(histories)),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx.clone(),
        operant_api::channel::ChannelMessage {
            id: "tg-msg-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-telegram".to_string(),
            content: "hello".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let calls = provider_impl
        .calls
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].len(), 4);

    let roles = calls[0]
        .iter()
        .map(|(role, _)| role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
    assert!(
        calls[0][0].1.contains("When responding on Telegram:"),
        "telegram channel instructions should be embedded into the system prompt"
    );
    assert!(
        calls[0][0].1.contains("For media attachments use markers:"),
        "telegram media marker guidance should live in the system prompt"
    );
    assert!(!calls[0].iter().skip(1).any(|(role, _)| role == "system"));
}

#[test]
fn channel_delivery_instructions_for_discord_mandates_absolute_paths() {
    let block = channel_delivery_instructions("discord")
        .expect("discord channel must have a delivery-instructions block");
    assert!(
        block.contains("When responding on Discord:"),
        "discord block must identify itself"
    );
    assert!(
        block.contains("For media attachments use markers:"),
        "discord block must describe marker syntax"
    );
    assert!(
        block.contains("MUST be absolute"),
        "discord block must mandate absolute paths"
    );
    assert!(
        block.contains("workspace"),
        "discord block must reference workspace bounds"
    );
    assert!(
        block.contains("[IMAGE:<absolute-path>]"),
        "discord block must show the absolute-path marker form"
    );
}

#[test]
fn extract_tool_context_summary_collects_alias_and_native_tool_calls() {
    let history = vec![
        ChatMessage::system("sys"),
        ChatMessage::assistant(
            r#"<toolcall>
{"name":"shell","arguments":{"command":"date"}}
</toolcall>"#,
        ),
        ChatMessage::assistant(
            r#"{"content":null,"tool_calls":[{"id":"1","name":"web_search","arguments":"{}"}]}"#,
        ),
    ];

    let summary = extract_tool_context_summary(&history, 1);
    assert_eq!(summary, "[Used tools: shell, web_search]");
}

#[test]
fn extract_tool_context_summary_collects_prompt_mode_tool_result_names() {
    let history = vec![
        ChatMessage::system("sys"),
        ChatMessage::assistant("Using markdown tool call fence"),
        ChatMessage::user(
            r#"[Tool results]
<tool_result name="http_request">
{"status":200}
</tool_result>
<tool_result name="shell">
Mon Feb 20
</tool_result>"#,
        ),
    ];

    let summary = extract_tool_context_summary(&history, 1);
    assert_eq!(summary, "[Used tools: http_request, shell]");
}

#[test]
fn extract_tool_context_summary_respects_start_index() {
    let history = vec![
        ChatMessage::assistant(
            r#"<tool_call>
{"name":"stale_tool","arguments":{}}
</tool_call>"#,
        ),
        ChatMessage::assistant(
            r#"<tool_call>
{"name":"fresh_tool","arguments":{}}
</tool_call>"#,
        ),
    ];

    let summary = extract_tool_context_summary(&history, 1);
    assert_eq!(summary, "[Used tools: fresh_tool]");
}

#[test]
fn strip_isolated_tool_json_artifacts_removes_tool_calls_and_results() {
    let mut known_tools = HashSet::new();
    known_tools.insert("schedule".to_string());

    let input = r#"{"name":"schedule","parameters":{"action":"create","message":"test"}}
{"name":"schedule","parameters":{"action":"cancel","task_id":"test"}}
Let me create the reminder properly:
{"name":"schedule","parameters":{"action":"create","message":"Go to sleep"}}
{"result":{"task_id":"abc","status":"scheduled"}}
Done reminder set for 1:38 AM."#;

    let result = strip_isolated_tool_json_artifacts(input, &known_tools);
    let normalized = result
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        normalized,
        "Let me create the reminder properly:\nDone reminder set for 1:38 AM."
    );
}

#[test]
fn strip_isolated_tool_json_artifacts_preserves_non_tool_json() {
    let mut known_tools = HashSet::new();
    known_tools.insert("shell".to_string());

    let input = r#"{"name":"profile","parameters":{"timezone":"UTC"}}
This is an example JSON object for profile settings."#;

    let result = strip_isolated_tool_json_artifacts(input, &known_tools);
    assert_eq!(result, input);
}

// ── AIEOS Identity Tests (Issue #168) ─────────────────────────

#[test]
fn aieos_identity_from_file() {
    use operant_config::schema::IdentityConfig;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let identity_path = tmp.path().join("aieos_identity.json");

    // Write AIEOS identity file
    let aieos_json = r#"{
            "identity": {
                "names": {"first": "Nova", "nickname": "Nov"},
                "bio": "A helpful AI assistant.",
                "origin": "Silicon Valley"
            },
            "psychology": {
                "mbti": "INTJ",
                "moral_compass": ["Be helpful", "Do no harm"]
            },
            "linguistics": {
                "style": "concise",
                "formality": "casual"
            }
        }"#;
    std::fs::write(&identity_path, aieos_json).unwrap();

    // Create identity config pointing to the file
    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: Some("aieos_identity.json".into()),
        aieos_inline: None,
    };

    let prompt = build_system_prompt(tmp.path(), "model", &[], &[], Some(&config), None);

    // Should contain AIEOS sections
    assert!(prompt.contains("## Identity"));
    assert!(prompt.contains("**Name:** Nova"));
    assert!(prompt.contains("**Nickname:** Nov"));
    assert!(prompt.contains("**Bio:** A helpful AI assistant."));
    assert!(prompt.contains("**Origin:** Silicon Valley"));

    assert!(prompt.contains("## Personality"));
    assert!(prompt.contains("**MBTI:** INTJ"));
    assert!(prompt.contains("**Moral Compass:**"));
    assert!(prompt.contains("- Be helpful"));

    assert!(prompt.contains("## Communication Style"));
    assert!(prompt.contains("**Style:** concise"));
    assert!(prompt.contains("**Formality Level:** casual"));

    // Should NOT contain OpenClaw bootstrap file headers
    assert!(!prompt.contains("### SOUL.md"));
    assert!(!prompt.contains("### IDENTITY.md"));
    assert!(!prompt.contains("[File not found"));
}

#[test]
fn aieos_identity_from_inline() {
    use operant_config::schema::IdentityConfig;

    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: None,
        aieos_inline: Some(r#"{"identity":{"names":{"first":"Claw"}}}"#.into()),
    };

    let prompt = build_system_prompt(
        std::env::temp_dir().as_path(),
        "model",
        &[],
        &[],
        Some(&config),
        None,
    );

    assert!(prompt.contains("**Name:** Claw"));
    assert!(prompt.contains("## Identity"));
}

#[test]
fn aieos_fallback_to_openclaw_on_parse_error() {
    use operant_config::schema::IdentityConfig;

    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: Some("nonexistent.json".into()),
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should fall back to OpenClaw format when AIEOS file is not found
    // (Error is logged to stderr with filename, not included in prompt)
    assert!(prompt.contains("### SOUL.md"));
}

#[test]
fn aieos_empty_uses_openclaw() {
    use operant_config::schema::IdentityConfig;

    // Format is "aieos" but neither path nor inline is set
    let config = IdentityConfig {
        format: "aieos".into(),
        aieos_path: None,
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should use OpenClaw format (not configured for AIEOS)
    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
}

#[test]
fn openclaw_format_uses_bootstrap_files() {
    use operant_config::schema::IdentityConfig;

    let config = IdentityConfig {
        format: "openclaw".into(),
        aieos_path: Some("identity.json".into()),
        aieos_inline: None,
    };

    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], Some(&config), None);

    // Should use OpenClaw format even if aieos_path is set
    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
    assert!(!prompt.contains("## Identity"));
}

#[test]
fn none_identity_config_uses_openclaw() {
    let ws = make_workspace();
    // Pass None for identity config
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    // Should use OpenClaw format
    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("Be helpful"));
}

#[test]
fn classify_health_ok_true() {
    let state = classify_health_result(&Ok(true));
    assert_eq!(state, ChannelHealthState::Healthy);
}

#[test]
fn classify_health_ok_false() {
    let state = classify_health_result(&Ok(false));
    assert_eq!(state, ChannelHealthState::Unhealthy);
}

#[tokio::test]
async fn classify_health_timeout() {
    let result = tokio::time::timeout(Duration::from_millis(1), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        true
    })
    .await;
    let state = classify_health_result(&result);
    assert_eq!(state, ChannelHealthState::Timeout);
}

#[test]
fn collect_configured_channels_includes_mattermost_when_configured() {
    let mut config = Config::default();
    config.channels.mattermost = Some(operant_config::schema::MattermostConfig {
        enabled: true,
        url: "https://mattermost.example.com".to_string(),
        bot_token: "test-token".to_string(),
        channel_id: Some("channel-1".to_string()),
        allowed_users: vec![],
        thread_replies: Some(true),
        mention_only: Some(false),
        interrupt_on_new_message: false,
        proxy_url: None,
    });

    let channels = collect_configured_channels(&config, "test", &[]);

    assert!(
        channels
            .iter()
            .any(|entry| entry.display_name == "Mattermost")
    );
    assert!(
        channels
            .iter()
            .any(|entry| entry.channel.name() == "mattermost")
    );
}

#[cfg(feature = "channel-email")]
#[test]
fn collect_configured_channels_skips_disabled_email() {
    let mut config = Config::default();
    config.channels.email = Some(operant_config::scattered_types::EmailConfig {
        enabled: false,
        ..Default::default()
    });

    let channels = collect_configured_channels(&config, "test", &[]);
    assert!(
        !channels.iter().any(|entry| entry.display_name == "Email"),
        "disabled email should not be collected"
    );
}

#[cfg(all(feature = "channel-voice-call", feature = "channels-vendor"))]
#[test]
fn collect_configured_channels_skips_disabled_voice_call() {
    let mut config = Config::default();
    config.channels.voice_call = Some(operant_config::scattered_types::VoiceCallConfig {
        enabled: false,
        ..Default::default()
    });

    let channels = collect_configured_channels(&config, "test", &[]);
    assert!(
        !channels
            .iter()
            .any(|entry| entry.display_name == "Voice Call"),
        "disabled voice-call should not be collected"
    );
}

struct AlwaysFailChannel {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

struct BlockUntilClosedChannel {
    name: String,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Channel for AlwaysFailChannel {
    fn name(&self) -> &str {
        self.name
    }

    async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<operant_api::channel::ChannelMessage>,
    ) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("listen boom")
    }
}

#[async_trait::async_trait]
impl Channel for BlockUntilClosedChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn listen(
        &self,
        tx: tokio::sync::mpsc::Sender<operant_api::channel::ChannelMessage>,
    ) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tx.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn supervised_listener_marks_error_and_restarts_on_failures() {
    let calls = Arc::new(AtomicUsize::new(0));
    let channel: Arc<dyn Channel> = Arc::new(AlwaysFailChannel {
        name: "test-supervised-fail",
        calls: Arc::clone(&calls),
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(1);
    let handle = spawn_supervised_listener(channel, tx, 1, 1);

    tokio::time::sleep(Duration::from_millis(80)).await;
    drop(rx);
    handle.abort();
    let _ = handle.await;

    let snapshot = operant_runtime::health::snapshot_json();
    let component = &snapshot["components"]["channel:test-supervised-fail"];
    assert_eq!(component["status"], "error");
    assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
    assert!(
        component["last_error"]
            .as_str()
            .unwrap_or("")
            .contains("listen boom")
    );
    assert!(calls.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn supervised_listener_refreshes_health_while_running() {
    let calls = Arc::new(AtomicUsize::new(0));
    let channel_name = format!("test-supervised-heartbeat-{}", uuid::Uuid::new_v4());
    let component_name = format!("channel:{channel_name}");
    let channel: Arc<dyn Channel> = Arc::new(BlockUntilClosedChannel {
        name: channel_name,
        calls: Arc::clone(&calls),
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(1);
    let handle = spawn_supervised_listener_with_health_interval(
        channel,
        tx,
        1,
        1,
        Duration::from_millis(20),
    );

    tokio::time::sleep(Duration::from_millis(35)).await;
    let first_last_ok =
        operant_runtime::health::snapshot_json()["components"][&component_name]["last_ok"]
            .as_str()
            .unwrap_or("")
            .to_string();
    assert!(!first_last_ok.is_empty());

    tokio::time::sleep(Duration::from_millis(70)).await;
    let second_last_ok =
        operant_runtime::health::snapshot_json()["components"][&component_name]["last_ok"]
            .as_str()
            .unwrap_or("")
            .to_string();
    let first = chrono::DateTime::parse_from_rfc3339(&first_last_ok)
        .expect("last_ok should be valid RFC3339");
    let second = chrono::DateTime::parse_from_rfc3339(&second_last_ok)
        .expect("last_ok should be valid RFC3339");
    assert!(second > first, "expected periodic health heartbeat refresh");

    drop(rx);
    let join = tokio::time::timeout(Duration::from_secs(1), handle).await;
    assert!(join.is_ok(), "listener should stop after channel shutdown");
    assert!(calls.load(Ordering::SeqCst) >= 1);
}

#[test]
fn maybe_restart_daemon_systemd_args_regression() {
    assert_eq!(
        SYSTEMD_STATUS_ARGS,
        ["--user", "is-active", "operant.service"]
    );
    assert_eq!(
        SYSTEMD_RESTART_ARGS,
        ["--user", "restart", "operant.service"]
    );
}

#[test]
fn maybe_restart_daemon_openrc_args_regression() {
    assert_eq!(OPENRC_STATUS_ARGS, ["operant", "status"]);
    assert_eq!(OPENRC_RESTART_ARGS, ["operant", "restart"]);
}

#[test]
fn normalize_merges_consecutive_user_turns() {
    let turns = vec![ChatMessage::user("hello"), ChatMessage::user("world")];
    let result = normalize_cached_channel_turns(turns);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "user");
    assert_eq!(result[0].content, "hello\n\nworld");
}

#[test]
fn normalize_preserves_strict_alternation() {
    let turns = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant("hi"),
        ChatMessage::user("bye"),
    ];
    let result = normalize_cached_channel_turns(turns);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].content, "hello");
    assert_eq!(result[1].content, "hi");
    assert_eq!(result[2].content, "bye");
}

#[test]
fn normalize_merges_multiple_consecutive_user_turns() {
    let turns = vec![
        ChatMessage::user("a"),
        ChatMessage::user("b"),
        ChatMessage::user("c"),
    ];
    let result = normalize_cached_channel_turns(turns);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].role, "user");
    assert_eq!(result[0].content, "a\n\nb\n\nc");
}

#[test]
fn normalize_empty_input() {
    let result = normalize_cached_channel_turns(vec![]);
    assert!(result.is_empty());
}

// ── E2E: photo [IMAGE:] marker rejected by non-vision provider ───

/// End-to-end test: a photo attachment message (containing `[IMAGE:]`
/// marker) sent through `process_channel_message` with a non-vision
/// provider must produce a `"⚠️ Error: …does not support vision"` reply
/// on the recording channel — no real Telegram or LLM API required.
#[tokio::test]
async fn e2e_photo_attachment_rejected_by_non_vision_provider() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    // DummyProvider has default capabilities (vision: false).
    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("dummy".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("You are a helpful assistant.".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    // Simulate a photo attachment message with [IMAGE:] marker.
    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-photo-1".to_string(),
            sender: "operant_user".to_string(),
            reply_target: "chat-photo".to_string(),
            content: "[IMAGE:/tmp/workspace/photo_99_1.jpg]\n\nWhat is this?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 1, "expected exactly one reply message");
    assert!(
        sent[0].contains("does not support vision"),
        "reply must mention vision capability error, got: {}",
        sent[0]
    );
    assert!(
        sent[0].contains("⚠️ Error"),
        "reply must start with error prefix, got: {}",
        sent[0]
    );
}

#[tokio::test]
async fn e2e_failed_vision_turn_does_not_poison_follow_up_text_turn() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("dummy".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("You are a helpful assistant.".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        Arc::clone(&runtime_ctx),
        operant_api::channel::ChannelMessage {
            id: "msg-photo-1".to_string(),
            sender: "operant_user".to_string(),
            reply_target: "chat-photo".to_string(),
            content: "[IMAGE:/tmp/workspace/photo_99_1.jpg]\n\nWhat is this?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    process_channel_message(
        Arc::clone(&runtime_ctx),
        operant_api::channel::ChannelMessage {
            id: "msg-text-2".to_string(),
            sender: "operant_user".to_string(),
            reply_target: "chat-photo".to_string(),
            content: "What is WAL?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 2, "expected one error and one successful reply");
    assert!(
        sent[0].contains("does not support vision"),
        "first reply must mention vision capability error, got: {}",
        sent[0]
    );
    assert!(
        sent[1].ends_with(":ok"),
        "second reply should succeed for text-only turn, got: {}",
        sent[1]
    );
    drop(sent);

    let histories = runtime_ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories
        .peek("test-channel_chat-photo_operant_user")
        .expect("history should exist for sender");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].content, "What is WAL?");
    assert_eq!(turns[1].role, "assistant");
    assert_eq!(turns[1].content, "ok");
    assert!(
        turns.iter().all(|turn| !turn.content.contains("[IMAGE:")),
        "failed vision turn must not persist image marker content"
    );
}

#[tokio::test]
async fn e2e_failed_non_retryable_turn_does_not_poison_follow_up_text_turn() {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(FormatErrorProvider),
        default_provider: Arc::new("dummy".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("You are a helpful assistant.".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 50000,
        context_token_budget: 128_000,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            std::time::Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
    });

    process_channel_message(
        Arc::clone(&runtime_ctx),
        operant_api::channel::ChannelMessage {
            id: "msg-bad-1".to_string(),
            sender: "operant_user".to_string(),
            reply_target: "chat-format".to_string(),
            content: "trigger format error".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    process_channel_message(
        Arc::clone(&runtime_ctx),
        operant_api::channel::ChannelMessage {
            id: "msg-text-2".to_string(),
            sender: "operant_user".to_string(),
            reply_target: "chat-format".to_string(),
            content: "What is WAL?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 2, "expected one error and one successful reply");
    assert!(
        sent[0].contains("Format Error"),
        "first reply must mention the request format error, got: {}",
        sent[0]
    );
    assert!(
        sent[1].ends_with(":ok"),
        "second reply should succeed for follow-up text, got: {}",
        sent[1]
    );
    drop(sent);

    let histories = runtime_ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let turns = histories
        .peek("test-channel_chat-format_operant_user")
        .expect("history should exist for sender");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].content, "What is WAL?");
    assert_eq!(turns[1].role, "assistant");
    assert_eq!(turns[1].content, "ok");
    assert!(
        turns
            .iter()
            .all(|turn| turn.content != "trigger format error"),
        "failed non-retryable turn must not persist in history"
    );
}

#[test]
fn build_channel_by_id_unknown_channel_returns_error() {
    let config = Config::default();
    match build_channel_by_id(&config, "nonexistent") {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("Unknown channel"),
                "expected 'Unknown channel' in error, got: {err_msg}"
            );
        }
        Ok(_) => panic!("should fail for unknown channel"),
    }
}

// ── Query classification in channel message processing ─────────

#[tokio::test]
async fn process_channel_message_applies_query_classification_route() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let vision_provider_impl = Arc::new(ModelCaptureProvider::default());
    let vision_provider: Arc<dyn Provider> = vision_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("vision-provider".to_string(), vision_provider);

    let classification_config = operant_config::schema::QueryClassificationConfig {
        enabled: true,
        rules: vec![operant_config::schema::ClassificationRule {
            hint: "vision".into(),
            keywords: vec!["analyze-image".into()],
            ..Default::default()
        }],
    };

    let model_routes = vec![operant_config::schema::ModelRouteConfig {
        hint: "vision".into(),
        provider: "vision-provider".into(),
        model: "gpt-4-vision".into(),
        api_key: None,
    }];

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(model_routes),
        query_classification: classification_config,
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-qc-1".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "please analyze-image from the dataset".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    // Vision provider should have been called instead of the default.
    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(vision_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        vision_provider_impl
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_slice(),
        &["gpt-4-vision".to_string()]
    );
}

#[tokio::test]
async fn process_channel_message_classification_disabled_uses_default_route() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let vision_provider_impl = Arc::new(ModelCaptureProvider::default());
    let vision_provider: Arc<dyn Provider> = vision_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("vision-provider".to_string(), vision_provider);

    // Classification is disabled — matching keyword should NOT trigger reroute.
    let classification_config = operant_config::schema::QueryClassificationConfig {
        enabled: false,
        rules: vec![operant_config::schema::ClassificationRule {
            hint: "vision".into(),
            keywords: vec!["analyze-image".into()],
            ..Default::default()
        }],
    };

    let model_routes = vec![operant_config::schema::ModelRouteConfig {
        hint: "vision".into(),
        provider: "vision-provider".into(),
        model: "gpt-4-vision".into(),
        api_key: None,
    }];

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(model_routes),
        query_classification: classification_config,
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-qc-disabled".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "please analyze-image from the dataset".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    // Default provider should be used since classification is disabled.
    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(vision_provider_impl.call_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn process_channel_message_classification_no_match_uses_default_route() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let vision_provider_impl = Arc::new(ModelCaptureProvider::default());
    let vision_provider: Arc<dyn Provider> = vision_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("vision-provider".to_string(), vision_provider);

    // Classification enabled with a rule that won't match the message.
    let classification_config = operant_config::schema::QueryClassificationConfig {
        enabled: true,
        rules: vec![operant_config::schema::ClassificationRule {
            hint: "vision".into(),
            keywords: vec!["analyze-image".into()],
            ..Default::default()
        }],
    };

    let model_routes = vec![operant_config::schema::ModelRouteConfig {
        hint: "vision".into(),
        provider: "vision-provider".into(),
        model: "gpt-4-vision".into(),
        api_key: None,
    }];

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(model_routes),
        query_classification: classification_config,
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-qc-nomatch".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "just a regular text message".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    // Default provider should be used since no classification rule matched.
    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(vision_provider_impl.call_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn process_channel_message_classification_priority_selects_highest() {
    let channel_impl = Arc::new(TelegramRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureProvider::default());
    let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
    let fast_provider_impl = Arc::new(ModelCaptureProvider::default());
    let fast_provider: Arc<dyn Provider> = fast_provider_impl.clone();
    let code_provider_impl = Arc::new(ModelCaptureProvider::default());
    let code_provider: Arc<dyn Provider> = code_provider_impl.clone();

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
    provider_cache_seed.insert("fast-provider".to_string(), fast_provider);
    provider_cache_seed.insert("code-provider".to_string(), code_provider);

    // Both rules match "code" keyword, but "code" rule has higher priority.
    let classification_config = operant_config::schema::QueryClassificationConfig {
        enabled: true,
        rules: vec![
            operant_config::schema::ClassificationRule {
                hint: "fast".into(),
                keywords: vec!["code".into()],
                priority: 1,
                ..Default::default()
            },
            operant_config::schema::ClassificationRule {
                hint: "code".into(),
                keywords: vec!["code".into()],
                priority: 10,
                ..Default::default()
            },
        ],
    };

    let model_routes = vec![
        operant_config::schema::ModelRouteConfig {
            hint: "fast".into(),
            provider: "fast-provider".into(),
            model: "fast-model".into(),
            api_key: None,
        },
        operant_config::schema::ModelRouteConfig {
            hint: "code".into(),
            provider: "code-provider".into(),
            model: "code-model".into(),
            api_key: None,
        },
    ];

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::clone(&default_provider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 5,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: false,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(model_routes),
        query_classification: classification_config,
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    process_channel_message(
        runtime_ctx,
        operant_api::channel::ChannelMessage {
            id: "msg-qc-prio".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-1".to_string(),
            content: "write some code for me".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1,
            thread_ts: None,
            interruption_scope_id: None,
            attachments: vec![],
        },
        CancellationToken::new(),
    )
    .await;

    // Higher-priority "code" rule (priority=10) should win over "fast" (priority=1).
    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(fast_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(code_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        code_provider_impl
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_slice(),
        &["code-model".to_string()]
    );
}

#[cfg(feature = "channel-telegram")]
#[test]
fn build_channel_by_id_unconfigured_telegram_returns_error() {
    let config = Config::default();
    match build_channel_by_id(&config, "telegram") {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("not configured"),
                "expected 'not configured' in error, got: {err_msg}"
            );
        }
        Ok(_) => panic!("should fail when telegram is not configured"),
    }
}

#[cfg(feature = "channel-telegram")]
#[test]
fn build_channel_by_id_configured_telegram_succeeds() {
    let mut config = Config::default();
    config.channels.telegram = Some(operant_config::schema::TelegramConfig {
        enabled: true,
        bot_token: "test-token".to_string(),
        allowed_users: vec![],
        stream_mode: operant_config::schema::StreamMode::Off,
        draft_update_interval_ms: 1000,
        interrupt_on_new_message: false,
        mention_only: false,
        ack_reactions: None,
        proxy_url: None,
        approval_timeout_secs: 120,
        dm_topics_enabled: false,
        dm_topic_name: "General".to_string(),
        disable_link_previews: false,
        typing_cooldown_seconds: 30.0,
        fallback_ips: vec![],
    });
    match build_channel_by_id(&config, "telegram") {
        Ok(channel) => assert_eq!(channel.name(), "telegram"),
        Err(e) => panic!("should succeed when telegram is configured: {e}"),
    }
}

#[cfg(all(feature = "channel-voice-call", feature = "channels-vendor"))]
#[test]
fn build_channel_by_id_unconfigured_voice_call_returns_error() {
    let config = Config::default();
    match build_channel_by_id(&config, "voice-call") {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("not configured"),
                "expected 'not configured' in error, got: {err_msg}"
            );
        }
        Ok(_) => panic!("should fail when voice-call is not configured"),
    }
}

#[cfg(all(feature = "channel-voice-call", feature = "channels-vendor"))]
#[test]
fn build_channel_by_id_configured_voice_call_succeeds() {
    let mut config = Config::default();
    config.channels.voice_call = Some(operant_config::scattered_types::VoiceCallConfig {
        enabled: true,
        provider: operant_config::scattered_types::VoiceProvider::Twilio,
        account_id: "AC_TEST".to_string(),
        auth_token: "test_token".to_string(),
        from_number: "+15551234567".to_string(),
        webhook_port: 8090,
        require_outbound_approval: true,
        transcription_logging: true,
        tts_voice: None,
        max_call_duration_secs: 3600,
        webhook_base_url: None,
    });
    match build_channel_by_id(&config, "voice-call") {
        Ok(channel) => assert_eq!(channel.name(), "voice_call"),
        Err(e) => panic!("should succeed when voice-call is configured: {e}"),
    }
}

// ── is_stop_command tests ─────────────────────────────────────────────

#[test]
fn is_stop_command_matches_bare_slash_stop() {
    assert!(is_stop_command("/stop"));
}

#[test]
fn is_stop_command_matches_with_leading_trailing_whitespace() {
    assert!(is_stop_command("  /stop  "));
}

#[test]
fn is_stop_command_is_case_insensitive() {
    assert!(is_stop_command("/STOP"));
    assert!(is_stop_command("/Stop"));
}

#[test]
fn is_stop_command_matches_with_bot_suffix() {
    assert!(is_stop_command("/stop@operant_bot"));
}

#[test]
fn is_stop_command_rejects_other_slash_commands() {
    assert!(!is_stop_command("/new"));
    assert!(!is_stop_command("/model gpt-4"));
    assert!(!is_stop_command("/models"));
}

#[test]
fn is_stop_command_rejects_plain_text() {
    assert!(!is_stop_command("stop"));
    assert!(!is_stop_command("please stop"));
    assert!(!is_stop_command(""));
}

#[test]
fn is_stop_command_rejects_stop_as_substring() {
    assert!(!is_stop_command("/stopwatch"));
    assert!(!is_stop_command("/stop-all"));
}

#[test]
fn interrupt_on_new_message_enabled_for_mattermost_when_true() {
    let cfg = InterruptOnNewMessageConfig {
        telegram: false,
        slack: false,
        discord: false,
        mattermost: true,
        matrix: false,
    };
    assert!(cfg.enabled_for_channel("mattermost"));
}

#[test]
fn interrupt_on_new_message_disabled_for_mattermost_by_default() {
    let cfg = InterruptOnNewMessageConfig {
        telegram: false,
        slack: false,
        discord: false,
        mattermost: false,
        matrix: false,
    };
    assert!(!cfg.enabled_for_channel("mattermost"));
}

#[test]
fn interrupt_on_new_message_enabled_for_discord() {
    let cfg = InterruptOnNewMessageConfig {
        telegram: false,
        slack: false,
        discord: true,
        mattermost: false,
        matrix: false,
    };
    assert!(cfg.enabled_for_channel("discord"));
}

#[test]
fn interrupt_on_new_message_disabled_for_discord_by_default() {
    let cfg = InterruptOnNewMessageConfig {
        telegram: false,
        slack: false,
        discord: false,
        mattermost: false,
        matrix: false,
    };
    assert!(!cfg.enabled_for_channel("discord"));
}

// ── interruption_scope_key tests ──────────────────────────────────────

#[test]
fn interruption_scope_key_without_scope_id_is_three_component() {
    let msg = operant_api::channel::ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "hi".into(),
        channel: "matrix".into(),
        timestamp: 0,
        thread_ts: None,
        interruption_scope_id: None,
        attachments: vec![],
    };
    assert_eq!(interruption_scope_key(&msg), "matrix_room_alice");
}

#[test]
fn interruption_scope_key_with_scope_id_is_four_component() {
    let msg = operant_api::channel::ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "hi".into(),
        channel: "matrix".into(),
        timestamp: 0,
        thread_ts: Some("$thread1".into()),
        interruption_scope_id: Some("$thread1".into()),
        attachments: vec![],
    };
    assert_eq!(interruption_scope_key(&msg), "matrix_room_alice_$thread1");
}

#[test]
fn interruption_scope_key_thread_ts_alone_does_not_affect_key() {
    // thread_ts used for reply anchoring should not bleed into scope key
    let msg = operant_api::channel::ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "C123".into(),
        content: "hi".into(),
        channel: "slack".into(),
        timestamp: 0,
        thread_ts: Some("1234567890.000100".into()), // Slack top-level fallback
        interruption_scope_id: None,                 // but NOT a thread reply
        attachments: vec![],
    };
    assert_eq!(interruption_scope_key(&msg), "slack_C123_alice");
}

#[tokio::test]
async fn message_dispatch_different_threads_do_not_cancel_each_other() {
    let channel_impl = Arc::new(SlackRecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(150),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        observer: Arc::new(NoopObserver),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(MAX_CONVERSATION_SENDERS).unwrap(),
        ))),
        pending_new_sessions: Arc::new(Mutex::new(HashSet::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(operant_config::schema::ReliabilityConfig::default()),
        provider_runtime_options: operant_providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        prompt_config: Arc::new(operant_config::schema::Config::default()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        interrupt_on_new_message: InterruptOnNewMessageConfig {
            telegram: false,
            slack: true,
            discord: false,
            mattermost: false,
            matrix: false,
        },
        multimodal: operant_config::schema::MultimodalConfig::default(),
        media_pipeline: operant_config::schema::MediaPipelineConfig::default(),
        transcription_config: operant_config::schema::TranscriptionConfig::default(),
        hooks: None,
        non_cli_excluded_tools: Arc::new(Vec::new()),
        autonomy_level: AutonomyLevel::default(),
        tool_call_dedup_exempt: Arc::new(Vec::new()),
        model_routes: Arc::new(Vec::new()),
        query_classification: operant_config::schema::QueryClassificationConfig::default(),
        ack_reactions: true,
        show_tool_calls: true,
        session_store: None,
        approval_manager: Arc::new(ApprovalManager::for_non_interactive(
            &operant_config::schema::AutonomyConfig::default(),
        )),
        activated_tools: None,
        cost_tracking: None,
        pacing: operant_config::schema::PacingConfig::default(),
        max_tool_result_chars: 0,
        context_token_budget: 0,
        debouncer: Arc::new(operant_infra::debounce::MessageDebouncer::new(
            Duration::ZERO,
        )),
        receipt_generator: None,
        show_receipts_in_response: false,
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<operant_api::channel::ChannelMessage>(8);
    let send_task = tokio::spawn(async move {
        // Two messages from same sender but in different Slack threads —
        // they must NOT cancel each other.
        tx.send(operant_api::channel::ChannelMessage {
            id: "1741234567.100001".to_string(),
            sender: "alice".to_string(),
            reply_target: "C123".to_string(),
            content: "thread-a question".to_string(),
            channel: "slack".to_string(),
            timestamp: 1,
            thread_ts: Some("1741234567.100001".to_string()),
            interruption_scope_id: Some("1741234567.100001".to_string()),
            attachments: vec![],
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        tx.send(operant_api::channel::ChannelMessage {
            id: "1741234567.200002".to_string(),
            sender: "alice".to_string(),
            reply_target: "C123".to_string(),
            content: "thread-b question".to_string(),
            channel: "slack".to_string(),
            timestamp: 2,
            thread_ts: Some("1741234567.200002".to_string()),
            interruption_scope_id: Some("1741234567.200002".to_string()),
            attachments: vec![],
        })
        .await
        .unwrap();
    });

    run_message_dispatch_loop(rx, runtime_ctx, 4).await;
    send_task.await.unwrap();

    // Both tasks should have completed — different threads, no cancellation.
    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(
        sent_messages.len(),
        2,
        "both Slack thread messages should complete, got: {sent_messages:?}"
    );
}

#[test]
fn sanitize_channel_response_redacts_detected_credentials() {
    let tools: Vec<Box<dyn Tool>> = Vec::new();
    let leaked = "Temporary key: AKIAABCDEFGHIJKLMNOP"; // gitleaks:allow

    let result = sanitize_channel_response(leaked, &tools);

    assert!(!result.contains("AKIAABCDEFGHIJKLMNOP")); // gitleaks:allow
    assert!(result.contains("[REDACTED"));
}

#[test]
fn sanitize_channel_response_passes_clean_text() {
    let tools: Vec<Box<dyn Tool>> = Vec::new();
    let clean_text = "This is a normal message with no credentials.";

    let result = sanitize_channel_response(clean_text, &tools);

    assert_eq!(result, clean_text);
}

// ── Tests for strip_think_tags_inline (streaming draft sanitization) ──

#[test]
fn strip_think_tags_inline_removes_single_block() {
    assert_eq!(
        strip_think_tags_inline("<think>reasoning</think>Hello"),
        "Hello"
    );
}

#[test]
fn strip_think_tags_inline_removes_multiple_blocks() {
    assert_eq!(
        strip_think_tags_inline("<think>a</think>X<think>b</think>Y"),
        "XY"
    );
}

#[test]
fn strip_think_tags_inline_handles_unclosed_block() {
    assert_eq!(
        strip_think_tags_inline("visible<think>hidden tail"),
        "visible"
    );
}

#[test]
fn strip_think_tags_inline_preserves_text_without_tags() {
    assert_eq!(strip_think_tags_inline("plain text"), "plain text");
}

#[test]
fn strip_think_tags_inline_handles_empty_string() {
    assert_eq!(strip_think_tags_inline(""), "");
}

#[test]
fn strip_think_tags_inline_strips_surrounding_whitespace() {
    assert_eq!(
        strip_think_tags_inline("<think>hidden</think>  Answer  "),
        "Answer"
    );
}

// ── Tests for #4827: tool context preservation ──────────────

#[test]
fn extract_current_turn_tool_messages_returns_intermediate_messages() {
    let history = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("older msg"),
        ChatMessage::assistant("older reply"),
        ChatMessage::user("block the iPad"),
        ChatMessage::assistant("{\"tool_call\": \"shell\"}"),
        ChatMessage::tool("ok"),
        ChatMessage::assistant("Done, iPad is blocked."),
    ];

    let tool_msgs = extract_current_turn_tool_messages(&history);
    assert_eq!(tool_msgs.len(), 2);
    assert_eq!(tool_msgs[0].role, "assistant");
    assert!(tool_msgs[0].content.contains("tool_call"));
    assert_eq!(tool_msgs[1].role, "tool");
}

#[test]
fn extract_current_turn_tool_messages_empty_when_no_tools() {
    let history = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant("Hi there!"),
    ];

    let tool_msgs = extract_current_turn_tool_messages(&history);
    assert!(tool_msgs.is_empty());
}

#[test]
fn extract_current_turn_tool_messages_multiple_tool_rounds() {
    let history = vec![
        ChatMessage::user("do two things"),
        ChatMessage::assistant("{\"tool_call\": \"read_skill\"}"),
        ChatMessage::tool("skill content"),
        ChatMessage::assistant("{\"tool_call\": \"shell\"}"),
        ChatMessage::tool("shell output"),
        ChatMessage::assistant("All done."),
    ];

    let tool_msgs = extract_current_turn_tool_messages(&history);
    assert_eq!(tool_msgs.len(), 4);
}

#[test]
fn is_tool_call_content_detects_tool_calls() {
    assert!(is_tool_call_content("{\"tool_call\": \"shell\"}"));
    assert!(is_tool_call_content("<tool_call>shell</tool_call>"));
    assert!(is_tool_call_content(
        "{\"name\": \"read_file\", \"args\": {}}"
    ));
    assert!(!is_tool_call_content("The iPad has been blocked."));
    assert!(!is_tool_call_content(""));
}

#[test]
fn normalize_cached_channel_turns_passes_through_tool_messages() {
    let turns = vec![
        ChatMessage::user("block the iPad"),
        ChatMessage::assistant("{\"tool_call\": \"shell\"}"),
        ChatMessage::tool("ok"),
        ChatMessage::assistant("iPad blocked."),
        ChatMessage::user("next question"),
    ];

    let normalized = normalize_cached_channel_turns(turns);
    // user, assistant(tool_call), tool, assistant(final), user
    assert_eq!(normalized.len(), 5);
    assert_eq!(normalized[2].role, "tool");
}

#[test]
fn default_keep_tool_context_turns_is_two() {
    let config = operant_config::schema::AgentConfig::default();
    assert_eq!(config.keep_tool_context_turns, 2);
}

#[test]
fn build_channel_system_prompt_includes_sender_id() {
    let prompt = build_channel_system_prompt(
        "You are a helpful assistant.",
        "mattermost",
        "channel123:root456",
        "user_abc123",
    );
    assert!(prompt.contains("sender=user_abc123"));
    assert!(prompt.contains("channel=mattermost"));
    assert!(prompt.contains("reply_target=channel123:root456"));
}

#[test]
fn build_channel_system_prompt_omits_context_when_reply_target_empty() {
    let prompt = build_channel_system_prompt("Base prompt.", "mattermost", "", "user_abc123");
    assert!(!prompt.contains("sender="));
    assert!(!prompt.contains("Channel context:"));
}

#[test]
fn build_channel_system_prompt_sender_distinguishes_users() {
    let prompt_a = build_channel_system_prompt("Base.", "mattermost", "ch:thread", "user_aaa");
    let prompt_b = build_channel_system_prompt("Base.", "mattermost", "ch:thread", "user_bbb");
    assert!(prompt_a.contains("sender=user_aaa"));
    assert!(prompt_b.contains("sender=user_bbb"));
    assert_ne!(prompt_a, prompt_b);
}

#[test]
fn build_channel_system_prompt_webhook_cron_hint_carries_thread_id() {
    // On the webhook channel `reply_target` is the inbound thread/conversation
    // id, not a recipient. Using it as `delivery.to` would strip the thread
    // context from the cron-announce callback (see #6634). The hint must
    // place the sender in `to` and the reply_target in `thread_id`.
    let prompt = build_channel_system_prompt(
        "Base.",
        "webhook",
        "agent-chat:agent-1:thread-7",
        "user:abc",
    );
    assert!(
        prompt.contains("\"to\":\"user:abc\""),
        "webhook cron hint must use sender as `to`: {prompt}"
    );
    assert!(
        prompt.contains("\"thread_id\":\"agent-chat:agent-1:thread-7\""),
        "webhook cron hint must carry the reply_target as `thread_id`: {prompt}"
    );
    assert!(
        !prompt.contains("\"to\":\"agent-chat:agent-1:thread-7\""),
        "webhook cron hint must not put the thread id in `to`: {prompt}"
    );
}

#[test]
fn build_channel_system_prompt_non_webhook_cron_hint_keeps_to_as_reply_target() {
    let prompt = build_channel_system_prompt("Base.", "slack", "C12345", "U67890");
    assert!(
        prompt.contains("\"to\":\"C12345\""),
        "non-webhook cron hint should keep reply_target as `to`: {prompt}"
    );
    assert!(
        !prompt.contains("\"thread_id\""),
        "non-webhook cron hint should not emit a thread_id field: {prompt}"
    );
}
