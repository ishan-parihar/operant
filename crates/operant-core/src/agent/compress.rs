//! `compress` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{Message, Role, Usage};
use std::sync::Arc;
use tracing::{info, warn};

use super::*;

impl OperantAgent {
    /// Compress context on overflow: try LLM summarization first, fall back
    /// to deterministic decay/eviction. Matches hermes-agent's compression
    /// pipeline: LLM compressor → fallback to manage_context.
    /// Compress an overflowing conversation, then fold the active todo list
    /// back into the compressed history so the model keeps its plan across
    /// compactions (hermes conversation_compression.py:
    /// `todo_snapshot = agent._todo_store.format_for_injection()`).
    pub(crate) async fn compress_context_overflow(&self, messages: Vec<Message>) -> Vec<Message> {
        let compressed = self.compress_context_overflow_inner(messages).await;
        self.reinject_todos_after_compression(compressed)
    }

    /// hermes parity: fold the active todo list back into the compressed
    /// history after compression. Any prior snapshot row is stripped first so
    /// repeated compactions refresh rather than accumulate (#26981 analog).
    pub(crate) fn reinject_todos_after_compression(
        &self,
        mut messages: Vec<Message>,
    ) -> Vec<Message> {
        let session_id = self.persistent_session_id.as_deref().unwrap_or("default");
        // The todo tool defaults to "default" when the model omits sessionId;
        // on gateway paths a persistent session id may be set while the model
        // still writes under the default key — look up both, preferring the
        // one that actually holds active todos.
        let snapshot =
            crate::tools::todo_tool::todo_injection_for_session(session_id).or_else(|| {
                if session_id != "default" {
                    crate::tools::todo_tool::todo_injection_for_session("default")
                } else {
                    None
                }
            });
        let Some(snapshot) = snapshot else {
            return messages;
        };

        messages.retain(|m| !crate::tools::todo_tool::is_todo_injection_row(&m.content));

        // Fold into a trailing REAL user message so compression never
        // introduces a synthetic user/user pair (hermes
        // conversation_compression.py); otherwise append as a new user turn.
        if let Some(tail) = messages.last_mut().filter(|m| m.role == Role::User) {
            tail.content.push_str("\n\n");
            tail.content.push_str(&snapshot);
            return messages;
        }
        messages.push(Message::user(snapshot));
        messages
    }

    pub(crate) async fn compress_context_overflow_inner(
        &self,
        messages: Vec<Message>,
    ) -> Vec<Message> {
        if let Some(ref compressor) = self.llm_compressor {
            // Bind database persistence on first compression attempt.
            // This ensures cooldown state survives process restarts —
            // matching hermes-agent's ContextCompressor cooldown persistence.
            // bind_persistence is idempotent and loads existing cooldown from DB.
            {
                let mut guard = compressor.lock().await;
                if guard.session_id().is_none()
                    && let Some(session_id) = self.persistent_session_id.as_ref()
                {
                    guard.bind_persistence(Arc::clone(&self.database), session_id.clone());
                }
            }

            // Check whether LLM compression is warranted (cheap, no await)
            {
                let guard = compressor.lock().await;
                if !guard.should_compress(self.estimate_current_tokens(&messages)) {
                    // Under threshold — deterministic fallback
                    let budget = self.config.context_window;
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
                // Anti-thrash: skip LLM compression if in cooldown after recent failure.
                if guard.is_in_cooldown() {
                    warn!("LLM compression in anti-thrash cooldown — using deterministic fallback");
                    let budget = self.config.context_window;
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
            }
            info!("Attempting LLM-based context compression");
            // Lock again for the async compress call (tokio::sync::Mutex is await-safe)
            let mut guard = compressor.lock().await;
            match guard.compress(messages.clone(), &self.client).await {
                Ok(result) => {
                    info!(
                        tokens_before = result.tokens_before,
                        tokens_after = result.tokens_after,
                        turns_summarized = result.turns_summarized,
                        "LLM compression succeeded"
                    );
                    drop(guard);
                    return result.messages;
                }
                Err(e) => {
                    warn!(error = %e, "LLM compression failed — falling back to deterministic");
                }
            }
            drop(guard);
        }
        // Deterministic fallback: decay + eviction
        let budget = self.config.context_window;
        crate::context_management::manage_context(messages, budget, 4096)
    }

    /// Access the underlying model client (useful for tools needing direct
    /// access to the concrete provider client).
    pub fn client(&self) -> &Arc<dyn ModelClient> {
        &self.client
    }

    /// Token estimate for the compression gate. Prefers the model-reported
    /// prompt-token count from the last request (source of truth, matching
    /// hermes context_engine), falling back to the char/4 heuristic when no
    /// request has completed yet this session.
    pub(crate) fn estimate_current_tokens(&self, messages: &[Message]) -> usize {
        prefer_reported(
            self.last_prompt_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            crate::context_management::estimate_total_tokens(messages),
        )
    }

    /// Emit `AgentEvent::Usage`/`AgentEvent::Cost` for a completed request
    /// and accumulate the session-level cost total. Shared by
    /// `process_response` (non-streaming) and `process_stream` (streaming,
    /// iter-247) now that both paths can produce a `Usage`.
    pub(crate) async fn emit_usage_and_cost(&self, usage: &Usage) {
        self.last_prompt_tokens.store(
            usage.prompt_tokens.try_into().unwrap_or(0),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.emit(AgentEvent::Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        })
        .await;

        // iter-132: emit a Cost event right after Usage. Look up the
        // model in models_dev to get cost-per-million, then multiply by
        // token counts. If the model isn't in the catalog, emit
        // cost_usd=None so the caller can show "cost unknown".
        //
        // We split the model name on '/' (provider/model format) to get
        // the provider and model parts. If there's no '/', we use the
        // whole string as the model and "" as the provider.
        let (provider, model_name) = match self.config.model.split_once('/') {
            Some((p, m)) => (p.to_string(), m.to_string()),
            None => (String::new(), self.config.model.clone()),
        };
        let cost_usd = crate::models_dev::get_model_capabilities(&provider, &model_name)
            .await
            .and_then(|caps| {
                let input_cost = caps
                    .cost_input_per_million
                    .map(|c| (usage.prompt_tokens as f64 / 1_000_000.0) * c);
                let output_cost = caps
                    .cost_output_per_million
                    .map(|c| (usage.completion_tokens as f64 / 1_000_000.0) * c);
                input_cost.zip(output_cost).map(|(i, o)| i + o)
            });
        self.emit(AgentEvent::Cost {
            cost_usd,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            model: self.config.model.clone(),
        })
        .await;

        if let Some(cost) = cost_usd
            && let Ok(mut total) = self.session_cost_usd.write()
        {
            *total += cost;
        }
    }
}
