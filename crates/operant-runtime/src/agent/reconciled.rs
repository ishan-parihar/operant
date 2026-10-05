//! `reconciled` — Wave 1 W1.4 facade: the runtime-side front door to the ONE
//! surviving compressor pair in operant-core (`LlmCompressor` summarization
//! + `context_management` decay). The deleted runtime engine
//! (`context_compressor.rs`) and its config (`ContextCompressionConfig`)
//! consolidated here: signatures keep the runtime's types
//! (`Vec<ChatMessage>` + `&dyn Provider`) and translate to core `Message`s
//! internally. The four production call sites (channels pre-call dispatch +
//! the interactive loop's error-recovery and post-turn paths) call these two
//! entries — no other compression engine exists in the runtime anymore.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use operant_api::provider::{ChatMessage, Provider};
use operant_core::agent::llm_compressor::{LlmCompressor, LlmCompressorConfig};
use operant_core::agent::{ChatRequest, ModelClient};
use operant_core::agent::{
    StreamChunk, fast_trim_tool_results, next_probe_tier, parse_context_limit_from_error,
    reinject_todos, repair_tool_pairs,
};
use operant_core::client::{ChatResponse, Choice, Message, MessageDelta, Role, Usage};
use operant_core::context_management::{estimate_total_tokens, manage_context};
use operant_memory::traits::{Memory, MemoryCategory};
use tracing::{info, warn};

/// Re-exported so channels (which has no direct operant-core dependency)
/// can construct the facade's config without a new edge in the dep graph.
pub use operant_core::agent::PreflightConfig;

/// Result of one compression run — same shape the deleted runtime engine
/// returned, so call-site match arms are unchanged.
#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub compressed: bool,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub passes_used: u32,
}

/// Low temperature for near-deterministic summarization; history compression
/// must faithfully reflect the source conversation, not invent or embellish.
const SUMMARIZER_TEMPERATURE: f64 = 0.1;

/// Tokens held back from the context window when the decay chain runs, so
/// the evicted history sizes against what is actually available.
const RESPONSE_RESERVE_TOKENS: usize = 4096;

// ---------------------------------------------------------------------------
// ChatMessage ↔ core Message translation
// ---------------------------------------------------------------------------

fn to_core(m: &ChatMessage) -> Message {
    let role = match m.role.as_str() {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    };
    Message::new(role, m.content.clone())
}

fn from_core(m: Message) -> ChatMessage {
    ChatMessage {
        role: m.role.as_str().to_string(),
        content: m.content,
    }
}

// ---------------------------------------------------------------------------
// Provider → ModelClient bridge
// ---------------------------------------------------------------------------

/// Adapts the runtime's `&dyn Provider` to core's `ModelClient` for the
/// summarization call. The core summarizer sends `[system, …user turns]`;
/// the bridge collapses that to the provider's
/// `chat_with_system(system, joined-user)` shape — the same call the old
/// engine's `compress_once` made.
struct ProviderBridge<'a> {
    provider: &'a dyn Provider,
}

#[async_trait]
impl ModelClient for ProviderBridge<'_> {
    async fn chat(&self, request: ChatRequest) -> operant_core::error::Result<ChatResponse> {
        let mut system: Option<String> = None;
        let mut user_parts: Vec<&str> = Vec::with_capacity(request.messages.len());
        for m in &request.messages {
            if m.role == Role::System && system.is_none() {
                system = Some(m.content.clone());
            } else {
                user_parts.push(m.content.as_str());
            }
        }
        let user_prompt = user_parts.join("\n\n");
        let temperature = request
            .temperature
            .map(f64::from)
            .unwrap_or(SUMMARIZER_TEMPERATURE);

        let text = self
            .provider
            .chat_with_system(
                system.as_deref(),
                &user_prompt,
                &request.model,
                Some(temperature),
            )
            .await
            .map_err(|e| operant_core::error::Error::Provider {
                status: 500,
                body: e.to_string(),
                retry_after: None,
            })?;

        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(ChatResponse {
            id: String::new(),
            object: "chat.completion".to_string(),
            created,
            model: request.model.clone(),
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(Role::Assistant),
                    content: Some(text),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
            },
        })
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> operant_core::error::Result<
        futures_util::stream::BoxStream<'static, operant_core::error::Result<StreamChunk>>,
    > {
        Err(operant_core::error::Error::ParseResponse(
            "streaming is not supported through the reconciled compression bridge".to_string(),
        ))
    }

    fn provider_name(&self) -> &str {
        "reconciled-bridge"
    }
}

// ---------------------------------------------------------------------------
// Facade entries
// ---------------------------------------------------------------------------

/// Compress `history` in place if it is over `context_window *
/// cfg.threshold_ratio`. Chain: fast-trim (c) → core summarization passes →
/// core decay on failure — the same summarization+decay chain the gateway
/// loop drives, plus the old engine's proactive ratio trigger. On any
/// effective compression the summary is persisted to `memory` (d) and the
/// active todo list is folded back in (semantic upgrade: the channels
/// pre-call path gains todo re-injection, previously Loop-A-only).
#[allow(clippy::too_many_lines)]
pub async fn compress_if_needed(
    history: &mut Vec<ChatMessage>,
    provider: &dyn Provider,
    model: &str,
    cfg: &PreflightConfig,
    context_window: usize,
    memory: Option<&Arc<dyn Memory>>,
    session_id: Option<&str>,
) -> Result<CompressionResult> {
    let tokens_before = estimate_total_tokens(&history.iter().map(to_core).collect::<Vec<_>>());
    if !cfg.enabled {
        return Ok(CompressionResult {
            compressed: false,
            tokens_before,
            tokens_after: tokens_before,
            passes_used: 0,
        });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let threshold = (context_window as f64 * cfg.threshold_ratio) as usize;
    if tokens_before <= threshold {
        return Ok(CompressionResult {
            compressed: false,
            tokens_before,
            tokens_after: tokens_before,
            passes_used: 0,
        });
    }

    let mut messages: Vec<Message> = history.iter().map(to_core).collect();

    // Fast-trim pass (c) — may resolve overflow without an LLM call.
    let chars_saved = fast_trim_tool_results(&mut messages, cfg);
    if chars_saved > 0 {
        info!(chars_saved, "Fast-trim saved chars from old tool results");
        let recheck = estimate_total_tokens(&messages);
        if recheck <= threshold {
            *history = messages.into_iter().map(from_core).collect();
            return Ok(CompressionResult {
                compressed: true,
                tokens_before,
                tokens_after: recheck,
                passes_used: 0,
            });
        }
    }

    // Summarization via the core chain, decaying deterministically on
    // failure — the reactive half of compress_context_overflow.
    let llm_cfg = LlmCompressorConfig {
        summarizer_model: cfg
            .summary_model
            .clone()
            .unwrap_or_else(|| model.to_string()),
        context_window,
        threshold_percent: cfg.threshold_ratio,
        protect_head_n: cfg.protect_first_n,
        protect_tail_n: Some(cfg.protect_last_n),
        source_max_chars: cfg.source_max_chars,
        summary_max_chars: cfg.summary_max_chars,
        enabled: true,
        ..Default::default()
    };
    let bridge = ProviderBridge { provider };
    let mut compressor = LlmCompressor::new(llm_cfg);

    let mut passes_used = 0u32;
    let mut changed = chars_saved > 0;
    let mut summary_text = String::new();
    for _ in 0..cfg.max_passes {
        let pass = tokio::time::timeout(
            Duration::from_secs(cfg.timeout_secs),
            compressor.compress(messages.clone(), &bridge),
        )
        .await;
        match pass {
            Ok(Ok(result)) => {
                let did_compress = result.turns_summarized > 0;
                if !result.summary_text.trim().is_empty() {
                    summary_text = result.summary_text;
                }
                messages = result.messages;
                if did_compress {
                    passes_used += 1;
                    changed = true;
                    // (e) clean up tool pairs orphaned by the splice.
                    repair_tool_pairs(&mut messages);
                }
                if !did_compress || estimate_total_tokens(&messages) <= threshold {
                    break;
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, "Summarization pass failed — decaying history");
                messages = manage_context(messages, context_window, RESPONSE_RESERVE_TOKENS);
                changed = true;
                break;
            }
            Err(_) => {
                warn!(
                    timeout_secs = cfg.timeout_secs,
                    "Summarization pass timed out — decaying history"
                );
                messages = manage_context(messages, context_window, RESPONSE_RESERVE_TOKENS);
                changed = true;
                break;
            }
        }
    }

    // (d) Persist the compression summary to memory before old turns are
    // forgotten, so compressed facts stay retrievable via memory recall.
    if changed
        && !summary_text.trim().is_empty()
        && let Some(memory) = memory
    {
        let facts_key = format!("compressed_context_{}", uuid::Uuid::new_v4());
        if let Err(e) = memory
            .store(&facts_key, &summary_text, MemoryCategory::Daily, None)
            .await
        {
            tracing::debug!("Failed to save compression summary to memory: {e}");
        } else {
            tracing::debug!("Saved compression summary to memory before discarding old turns");
        }
    }

    // Fold the active todo list back in — previously a Loop-A-only behavior;
    // the channels pre-call path gains it (W1.4).
    if changed {
        messages = reinject_todos(messages, session_id);
    }

    let tokens_after = estimate_total_tokens(&messages);
    *history = messages.into_iter().map(from_core).collect();
    Ok(CompressionResult {
        compressed: changed,
        tokens_before,
        tokens_after,
        passes_used,
    })
}

/// Reactive compression after a context_length_exceeded error. Parses the
/// actual limit from the error when it states one, otherwise steps down a
/// probe tier (b), then runs [`compress_if_needed`] against the adjusted
/// window. `context_window` is `&mut` so the caller's probe state can be
/// threaded through consecutive retries.
pub async fn compress_on_error(
    history: &mut Vec<ChatMessage>,
    provider: &dyn Provider,
    model: &str,
    error_msg: &str,
    cfg: &PreflightConfig,
    context_window: &mut usize,
    memory: Option<&Arc<dyn Memory>>,
    session_id: Option<&str>,
) -> Result<bool> {
    // (b) Try to extract the actual limit; else step down to the next tier.
    if let Some(limit) = parse_context_limit_from_error(error_msg) {
        *context_window = limit;
    } else {
        *context_window = next_probe_tier(*context_window);
    }
    info!(
        context_window = *context_window,
        "Context limit adjusted, re-compressing"
    );

    let result = compress_if_needed(
        history,
        provider,
        model,
        cfg,
        *context_window,
        memory,
        session_id,
    )
    .await?;
    Ok(result.compressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SummarizingProvider;

    #[async_trait]
    impl Provider for SummarizingProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<String> {
            Ok("compressed context summary".to_string())
        }

        async fn chat(
            &self,
            _request: operant_api::provider::ChatRequest<'_>,
            _model: &str,
            _temperature: Option<f64>,
        ) -> anyhow::Result<operant_api::provider::ChatResponse> {
            unreachable!("the reconciled bridge only calls chat_with_system")
        }

        fn supports_vision(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn compress_on_error_parses_limit_then_compresses_via_core_chain() {
        // Over any plausible threshold: 20 fat user turns ≈ 10k estimated
        // tokens; fast-trim cannot touch non-tool messages, so the core
        // summarization pass must fire.
        let mut history: Vec<ChatMessage> = (0..20)
            .map(|i| ChatMessage::user(format!("history turn {i}: {}", "x".repeat(2_000))))
            .collect();
        let mut window = 500_000usize;
        let compressed = compress_on_error(
            &mut history,
            &SummarizingProvider,
            "demo",
            "This model's maximum context length is 2048 tokens. However, your messages resulted in 15000 tokens.",
            &PreflightConfig::default(),
            &mut window,
            None,
            None,
        )
        .await
        .expect("compress_on_error must succeed");

        assert_eq!(
            window, 2_048,
            "the real limit must be parsed from the error"
        );
        assert!(compressed, "the core summarization pass must fire");
        assert!(
            history
                .iter()
                .any(|m| m.content.contains("compressed context summary")),
            "the core-spliced summary must be translated back into the runtime history"
        );
        assert!(
            history.iter().any(|m| m.role == "system"
                && m.content.contains("[CONTEXT COMPACTION — REFERENCE ONLY]")),
            "the summary must land as the core chain's marked system message"
        );
    }
}
