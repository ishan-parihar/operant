//! `compress` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{Message, Role, Usage};
use crate::schema::ToolSchema;
use std::sync::Arc;
use tracing::{info, warn};

use super::*;

/// Tokens held back from the context window for the model's own response, so
/// budgeted passes size the input against what is actually available.
const RESPONSE_RESERVE_TOKENS: usize = 4096;

// ---------------------------------------------------------------------------
// Preflight compression config (W1.4 port from the deleted runtime engine)
// ---------------------------------------------------------------------------

/// Knobs for the proactive pre-call / reactive post-error compression runs
/// driven through the runtime reconciled facade. Field-for-field successor
/// of the deleted `operant_config::scattered_types::ContextCompressionConfig`
/// (serde keys retired with it — no shipped config set them); defaults are the
/// old engine's shipped values so the consolidated path behaves identically.
#[derive(Debug, Clone)]
pub struct PreflightConfig {
    /// Whether preflight compression is enabled. Old default: `true`.
    pub enabled: bool,
    /// Fraction of the context window at which compression triggers.
    pub threshold_ratio: f64,
    /// Leading messages protected from every compression step.
    pub protect_first_n: usize,
    /// Trailing messages protected from fast-trim; also mapped onto the
    /// summarizer pass via `LlmCompressorConfig::protect_tail_n`.
    pub protect_last_n: usize,
    /// Maximum summarization passes per run.
    pub max_passes: u32,
    /// Character ceiling for a generated summary.
    pub summary_max_chars: usize,
    /// Character ceiling for the summarizer source input.
    pub source_max_chars: usize,
    /// Timeout (seconds) for one summarization pass.
    pub timeout_secs: u64,
    /// Model used to generate summaries; `None` reuses the route model.
    pub summary_model: Option<String>,
    /// Character budget for re-trimming old tool results (0 disables).
    pub tool_result_retrim_chars: usize,
    /// Tool-result substrings exempt from re-trimming.
    pub tool_result_trim_exempt: Vec<String>,
}

impl Default for PreflightConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_ratio: 0.50,
            protect_first_n: 3,
            protect_last_n: 4,
            max_passes: 3,
            summary_max_chars: 4_000,
            source_max_chars: 50_000,
            timeout_secs: 60,
            summary_model: None,
            tool_result_retrim_chars: 2_000,
            tool_result_trim_exempt: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error-probe tiers (b) — ported from the deleted runtime engine
// ---------------------------------------------------------------------------

const PROBE_TIERS: &[usize] = &[
    2_000_000, 1_000_000, 512_000, 200_000, 128_000, 64_000, 32_000,
];

/// Next smaller guess when an overflow error states no explicit limit.
pub fn next_probe_tier(current: usize) -> usize {
    PROBE_TIERS
        .iter()
        .copied()
        .find(|&tier| tier < current)
        .unwrap_or(32_000)
}

/// Try to extract the actual context window limit from a provider error
/// message. Patterns: "maximum context length is 128000", "limit of 200000
/// tokens", "context window of 131072", "available context size (8448
/// tokens)", Anthropic's "> 128000 maximum context length".
pub fn parse_context_limit_from_error(msg: &str) -> Option<usize> {
    let re_patterns: &[&str] = &[
        r"(?:max(?:imum)?|limit)\s*(?:context\s*)?(?:length|size|window)?\s*(?:is|of|:)?\s*(\d{4,})",
        r"context\s*(?:length|size|window)\s*(?:is|of|:)?\s*(\d{4,})",
        r"(\d{4,})\s*(?:tokens?\s*)?(?:context|limit)",
        r"available context size\s*\(\s*(\d{4,})",
        r">\s*(\d{4,})\s*(?:maximum|max)?\s*(?:context)?\s*(?:length|size|window|tokens?)",
    ];
    let lower = msg.to_lowercase();
    for pattern in re_patterns {
        if let Ok(re) = regex::Regex::new(pattern)
            && let Some(caps) = re.captures(&lower)
            && let Some(m) = caps.get(1)
            && let Ok(limit) = m.as_str().parse::<usize>()
            && (1024..=10_000_000).contains(&limit)
        {
            return Some(limit);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Inline tool-result trim (c) — ported from the deleted runtime engine
// ---------------------------------------------------------------------------

/// Fast-path: trim oversized tool results in non-protected messages.
/// Returns total characters saved. No LLM call needed.
pub fn fast_trim_tool_results(messages: &mut [Message], cfg: &PreflightConfig) -> usize {
    let max = cfg.tool_result_retrim_chars;
    if max == 0 {
        return 0;
    }
    let mut saved = 0;
    let protect_start = cfg.protect_first_n.min(messages.len());
    let protect_end = messages.len().saturating_sub(cfg.protect_last_n);

    if protect_start >= protect_end {
        return 0;
    }

    for msg in &mut messages[protect_start..protect_end] {
        if msg.role != Role::Tool {
            continue;
        }
        if msg.content.len() <= max {
            continue;
        }
        if cfg
            .tool_result_trim_exempt
            .iter()
            .any(|t| msg.content.contains(t.as_str()))
        {
            continue;
        }
        if msg.content.contains("data:image/") {
            continue;
        }
        let original_len = msg.content.len();
        msg.content = truncate_tool_content(&msg.content, max);
        saved += original_len - msg.content.len();
    }
    saved
}

/// Truncate a tool message body to `max_chars`, preserving the
/// `{"content":…,"tool_call_id":…}` JSON envelope runtime-side tool messages
/// carry inside `content` — bisecting it strands the `tool_call_id` and
/// upstream providers reject the orphaned result (#5425 analog). Plain
/// content is truncated head/tail with an explicit marker.
fn truncate_tool_content(content: &str, max_chars: usize) -> String {
    if let Ok(mut obj) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(content)
        && obj.contains_key("tool_call_id")
        && let Some(serde_json::Value::String(inner)) = obj.get("content")
    {
        let truncated = truncate_head_tail(inner, max_chars);
        obj.insert("content".to_string(), serde_json::Value::String(truncated));
        return serde_json::to_string(&obj).unwrap_or_else(|_| content.to_string());
    }
    truncate_head_tail(content, max_chars)
}

/// Head/tail-preserving truncation with an explicit marker (2/3 head, 1/3
/// tail, matching the deleted runtime engine's `truncate_tool_result`).
// ponytail: no [IMAGE:]-marker boundary nudging — markers bisected here are
// inert text in the text-only summarizer path; add the nudge if a vision
// summarizer ever consumes trimmed tool output.
fn truncate_head_tail(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars.saturating_sub(head_len);
    let mut head_end = head_len.min(s.len());
    while head_end > 0 && !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len().saturating_sub(tail_len);
    while tail_start < s.len() && !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    if head_end >= tail_start {
        let mut end = max_chars.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        return format!(
            "{}\n\n[... {} characters truncated ...]",
            &s[..end],
            s.len() - end
        );
    }
    format!(
        "{}\n\n[... {} characters truncated ...]\n\n{}",
        &s[..head_end],
        tail_start - head_end,
        &s[tail_start..]
    )
}

// ---------------------------------------------------------------------------
// Tool-pair repair (e) — ported from the deleted runtime engine
// ---------------------------------------------------------------------------

/// Remove tool results orphaned by a compression splice: any `tool` message
/// immediately following the summary message, and any leading `tool` run
/// whose owning assistant turn was summarized away. Without this, strict
/// providers reject the history with "unexpected tool_use_id in
/// tool_result blocks" (#5813 analog).
pub fn repair_tool_pairs(messages: &mut Vec<Message>) {
    let mut i = 0;
    while i < messages.len() {
        if messages[i]
            .content
            .contains(crate::agent::llm_compressor::SUMMARY_PREFIX)
        {
            while i + 1 < messages.len() && messages[i + 1].role == Role::Tool {
                messages.remove(i + 1);
            }
        }
        i += 1;
    }

    let start = usize::from(messages.first().is_some_and(|m| m.role == Role::System));
    while start < messages.len() && messages[start].role == Role::Tool {
        messages.remove(start);
    }
}

// ---------------------------------------------------------------------------
// Todo re-injection (shared by the agent method and the runtime facade)
// ---------------------------------------------------------------------------

/// hermes parity: fold the active todo list back into a compressed history.
/// Any prior snapshot row is stripped first so repeated compactions refresh
/// rather than accumulate (#26981 analog). Shared free fn so the runtime
/// reconciled facade triggers the same fold the agent loop does.
pub fn reinject_todos(messages: Vec<Message>, session_id: Option<&str>) -> Vec<Message> {
    let session_id = session_id.unwrap_or("default");
    // The todo tool defaults to "default" when the model omits sessionId;
    // on gateway paths a persistent session id may be set while the model
    // still writes under the default key — look up both, preferring the
    // one that actually holds active todos.
    let snapshot = crate::tools::todo_tool::todo_injection_for_session(session_id).or_else(|| {
        if session_id != "default" {
            crate::tools::todo_tool::todo_injection_for_session("default")
        } else {
            None
        }
    });
    let Some(snapshot) = snapshot else {
        return messages;
    };

    let mut messages = messages;
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

impl OperantAgent {
    /// Compress context on overflow: try LLM summarization first, fall back
    /// to deterministic decay/eviction. Matches hermes-agent's compression
    /// pipeline: LLM compressor → fallback to manage_context.
    /// Compress an overflowing conversation, then fold the active todo list
    /// back into the compressed history so the model keeps its plan across
    /// compactions (hermes conversation_compression.py:
    /// `todo_snapshot = agent._todo_store.format_for_injection()`).
    ///
    /// Reports whether the compression actually SHRANK the history.
    ///
    /// The provider rejected a request of `before` tokens. If the
    /// compressed request is not smaller, the retry is CERTAIN to overflow
    /// the same way — re-issuing it burns a model call and dies with the
    /// same raw provider error. So the caller is told when the compression
    /// could not help, and fails typed instead.
    ///
    /// Note this is deliberately NOT a "does the result fit the window"
    /// check: the reactive path is only reached when the request ALREADY
    /// fit the window (the build-time guard passed) and the provider
    /// overflowed anyway — so after compressing, the history is smaller,
    /// and a fit check would never fail. Such a check is dead code.
    pub(crate) async fn compress_context_overflow_reporting(
        &self,
        messages: Vec<Message>,
    ) -> (Vec<Message>, bool) {
        let tokens_before = self.estimate_current_tokens(&messages);
        let messages_before = messages.len();
        // The SIZE of the history the provider rejected, measured before
        // it is consumed — this is the number the retry has to beat.
        let rejected_tokens = crate::context_management::estimate_total_tokens(&messages);
        self.emit(AgentEvent::CompactionStarted { tokens_before })
            .await;
        let compressed = self.compress_context_overflow_inner(messages).await;
        // `estimate_current_tokens` prefers the last *reported* prompt count,
        // which is the pre-compaction value — use the char/4 heuristic so
        // `tokens_after` actually reflects the compressed history.
        self.emit(AgentEvent::CompactionCompleted {
            tokens_before,
            tokens_after: crate::context_management::estimate_total_tokens(&compressed),
            messages_before,
            messages_after: compressed.len(),
        })
        .await;
        let compressed = self.reinject_todos_after_compression(compressed);
        // Did the compression actually HELP?
        //
        // The condition that can fail is the one that matters: did the
        // compression REDUCE the history? The provider rejected a request
        // of `rejected_tokens` tokens. If the compressed request is not
        // smaller, the retry is CERTAIN to overflow the same way —
        // re-issuing it burns a model call and dies with the same raw
        // provider error.
        //
        // (The learned limit is not used here: `learn_context_limit` runs
        // BEFORE this and sets the learned limit to the size of the
        // request that JUST overflowed, so comparing against it would
        // guarantee failure.)
        let after = crate::context_management::estimate_total_tokens(&compressed);
        let within_budget = after < rejected_tokens;
        if !within_budget {
            warn!(
                rejected_tokens,
                after,
                "context compression did not reduce the history — the provider \\
                 rejected a request of this size, so a retry would overflow again"
            );
        }
        (compressed, within_budget)
    }

    /// hermes parity: fold the active todo list back into the compressed
    /// history after compression. Any prior snapshot row is stripped first so
    /// repeated compactions refresh rather than accumulate (#26981 analog).
    pub(crate) fn reinject_todos_after_compression(&self, messages: Vec<Message>) -> Vec<Message> {
        reinject_todos(messages, self.session_id().as_deref())
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
                    && let Some(session_id) = self.session_id()
                {
                    guard.bind_persistence(Arc::clone(&self.database), session_id);
                }
            }

            // Check whether LLM compression is warranted (cheap, no await)
            {
                let guard = compressor.lock().await;
                if !guard.should_compress(self.estimate_current_tokens(&messages)) {
                    // Under threshold — deterministic fallback
                    let budget = self.effective_context_window();
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
                // Anti-thrash: skip LLM compression if in cooldown after recent failure.
                if guard.is_in_cooldown() {
                    warn!("LLM compression in anti-thrash cooldown — using deterministic fallback");
                    let budget = self.effective_context_window();
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
            }
            info!("Attempting LLM-based context compression");
            // Lock again for the async compress call (tokio::sync::Mutex is await-safe)
            let mut guard = compressor.lock().await;
            match guard.compress(messages.clone(), self.client.as_ref()).await {
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
        let budget = self.effective_context_window();
        crate::context_management::manage_context(messages, budget, 4096)
    }

    /// Wave-4 preflight ladder rung 3: LLM summarize-before-evict.
    ///
    /// Same guards as the reactive path (`should_compress`, anti-thrash
    /// cooldown, persistence bind) but with NO deterministic fallback —
    /// the ladder's later rungs own decay and eviction. On failure or
    /// guard rejection the messages pass through untouched.
    pub(crate) async fn preflight_llm_summarize(&self, messages: Vec<Message>) -> Vec<Message> {
        let Some(ref compressor) = self.llm_compressor else {
            return messages;
        };
        {
            let mut guard = compressor.lock().await;
            if guard.session_id().is_none()
                && let Some(session_id) = self.session_id()
            {
                guard.bind_persistence(Arc::clone(&self.database), session_id);
            }
        }
        let estimated = self.estimate_current_tokens(&messages);
        let mut guard = compressor.lock().await;
        if !guard.should_compress(estimated) {
            return messages;
        }
        if guard.is_in_cooldown() {
            warn!("Preflight LLM summarize skipped — anti-thrash cooldown active");
            return messages;
        }
        match guard.compress(messages.clone(), self.client.as_ref()).await {
            Ok(result) => {
                info!(
                    tokens_before = result.tokens_before,
                    tokens_after = result.tokens_after,
                    turns_summarized = result.turns_summarized,
                    "Preflight LLM summarize succeeded (summarize-before-evict)"
                );
                result.messages
            }
            Err(e) => {
                warn!(error = %e, "Preflight LLM summarize failed — rung passes through");
                messages
            }
        }
    }

    /// Access the underlying model client (useful for tools needing direct
    /// access to the concrete provider client).
    pub fn client(&self) -> &Arc<dyn ModelClient> {
        &self.client
    }

    /// Token estimate for the compression gate and the output fit.
    ///
    /// The LARGER of two numbers:
    ///
    /// - the local char/4 heuristic over the CURRENT message list, which
    ///   counts the tool results, the reasoning and the tool-call
    ///   arguments — everything the request about to be sent will carry;
    /// - the model-reported prompt-token count from the last request,
    ///   which is the provider's own exact accounting.
    ///
    /// The reported count is exact, but it is exact for the PREVIOUS
    /// request: every tool result appended since is missing from it. So
    /// preferring it UNDERSTATES the prompt the agent is about to send —
    /// which is how the output fitter came to size `max_tokens` from a
    /// stale number and let a request exceed the window by exactly the
    /// accumulated tool output (RT-116).
    ///
    /// Taking the larger of the two is the safe rule: the heuristic can
    /// overcount slightly (and the fit has a headway term for that), and
    /// the reported count can undercount by a whole turn's tool output.
    /// Neither alone is safe; the maximum is.
    pub(crate) fn estimate_current_tokens(&self, messages: &[Message]) -> usize {
        self.last_prompt_tokens
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(crate::context_management::estimate_total_tokens(messages))
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Build the LLM request with the output-token cap fitted to the context
    /// window (omp `output-budget.ts` parity, iter-792).
    ///
    /// Chat-Completions providers reject a request whose prompt + `max_tokens`
    /// exceeds the window — a 400 BEFORE generation that the reactive
    /// overflow-compressor can never rescue (and side turns have no recovery
    /// at all). This sizes `max_tokens` to `window − prompt − headway` so the
    /// request is always admissible; a cap that already fits is left alone.
    /// `prompt_is_reported` follows `estimate_current_tokens`'s preference:
    /// the provider's own count is exact (no margin), the heuristic is padded
    /// by 10% so undercounting cannot reproduce the rejection.
    pub(crate) fn fitted_request(
        &self,
        model: &str,
        messages: &[Message],
        tools: Vec<ToolSchema>,
    ) -> ChatRequest {
        // The prompt size the fit must be sized against is the CURRENT
        // one — the size of the request about to be sent.
        //
        // `estimate_current_tokens` takes the LARGER of the current
        // estimate and the provider's last reported count. The reported
        // count is exact but STALE (it is exact for the PREVIOUS
        // request, and every tool result appended since is missing from
        // it), and `fit_output_tokens` skips its safety margin for a
        // reported count. So the fit is sized against an estimate, and the
        // margin always applies.
        let prompt = self.estimate_current_tokens(messages);
        let reported = false;
        // (plan 03) main turns carry the reasoning effort unless the model
        // rejected it this session. `Auto` shape resolves per endpoint.
        let effort = self
            .reasoning_effort
            .read()
            .expect("reasoning_effort RwLock poisoned — programmer error")
            .clone()
            .filter(|_| !self.effort_unsupported_for(model));
        let shape = match self.config.reasoning_effort_param {
            crate::config::ReasoningEffortParam::Auto => crate::client::ReasoningShape::Auto,
            crate::config::ReasoningEffortParam::ReasoningEffort => {
                crate::client::ReasoningShape::ReasoningEffort
            }
            crate::config::ReasoningEffortParam::ReasoningObject => {
                crate::client::ReasoningShape::ReasoningObject
            }
            crate::config::ReasoningEffortParam::AnthropicThinking => {
                crate::client::ReasoningShape::AnthropicThinking
            }
            crate::config::ReasoningEffortParam::None => crate::client::ReasoningShape::None,
        };
        let request = ChatRequest::new(model.to_string(), messages.to_vec())
            .with_tools(tools)
            .with_stream(self.config.stream)
            .with_reasoning_effort(effort);
        // (plan 03) the resolved wire shape travels with the request so the
        // adapter doesn't re-derive it; ChatRequest is provider-agnostic, so
        // piggyback on the builder — the adapter receives the shape from
        // the agent's config at the call site below.
        self.fitted_request_shaped(request, prompt, reported, shape)
    }

    fn fitted_request_shaped(
        &self,
        request: ChatRequest,
        prompt: usize,
        reported: bool,
        _shape: crate::client::ReasoningShape,
    ) -> ChatRequest {
        match crate::context_management::fit_output_tokens(
            request.max_tokens,
            self.effective_context_window(),
            prompt,
            reported,
        ) {
            Some(fitted) => ChatRequest {
                max_tokens: Some(fitted),
                ..request
            },
            None => request,
        }
    }

    /// Apply the tool-result spend guard to one tool output before it joins the
    /// LLM-bound message list.
    ///
    /// A single tool result can be enormous (a whole file, a long command's
    /// stdout) and would otherwise silently eat the context window. When it
    /// exceeds `agent.max_tool_result_share` of the usable budget, the bulk is
    /// withheld behind an explicit marker stating the size, the share and the
    /// estimated input price — the price comes from the same `models.dev`
    /// `cost_input_per_million` lookup `emit_usage_and_cost` already uses, and
    /// is only fetched when the guard actually fires.
    pub(crate) async fn guard_tool_output_spend(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        content: String,
    ) -> String {
        let budget = self
            .config
            .context_window
            .saturating_sub(RESPONSE_RESERVE_TOKENS);
        let max_share = self.config.max_tool_result_share;
        let limit = crate::context_management::tool_result_token_limit(budget, max_share);

        // Reuse the existing token counter for the threshold pre-check so the
        // (possibly networked) price lookup only runs when the guard fires.
        if crate::context_management::estimate_tokens(&content) <= limit {
            return content;
        }

        let (provider, model_name) = match self.config.model.split_once('/') {
            Some((p, m)) => (p.to_string(), m.to_string()),
            None => (String::new(), self.config.model.clone()),
        };
        let price = crate::models_dev::get_model_capabilities(&provider, &model_name)
            .await
            .and_then(|caps| caps.cost_input_per_million);

        let Some(guarded) = crate::context_management::guard_tool_result(
            tool_call_id,
            &content,
            budget,
            max_share,
            price,
        ) else {
            return content;
        };
        info!(
            tool = tool_name,
            original_chars = content.chars().count(),
            original_tokens = crate::context_management::estimate_tokens(&content),
            kept_chars = guarded.chars().count(),
            limit_tokens = limit,
            budget_tokens = budget,
            cost_usd_per_million = price,
            "oversized tool result: bulk withheld behind an explicit marker"
        );
        guarded
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

        // O-19 (W3): a Holon run's token budget is enforced here — the
        // provider reports what each call actually spent (input tokens
        // resend the context every turn, so the estimate-based meter
        // undercounted by ~turns). Over the cap: record the refusal (the
        // stage sees a budget blocker, not a silent death) and trigger the
        // interrupt — the loop's existing machinery ends the turn cleanly.
        // O-19: the in/out split feeds usage.json — provider-metered,
        // not estimated (input tokens resend the context every turn;
        // the chars/4 estimate undercounted by ~turns). Metered on every
        // Holon run; the cap comparison below is the only conditional.
        if crate::platform::holon_run_dir().is_some() {
            self.holon_tokens_in.fetch_add(
                u64::from(usage.prompt_tokens),
                std::sync::atomic::Ordering::Relaxed,
            );
            self.holon_tokens_out.fetch_add(
                u64::from(usage.completion_tokens),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        if let Some(run_dir) = crate::platform::holon_run_dir()
            && let Some(cap) = crate::org::holon_budget_cap(&run_dir)
        {
            let spent = self.holon_tokens_spent.fetch_add(
                u64::from(usage.total_tokens),
                std::sync::atomic::Ordering::Relaxed,
            ) + u64::from(usage.total_tokens);
            if spent > cap && !self.interrupt_flag.is_triggered() {
                let why = format!(
                    "token budget exhausted: {spent} spent, cap {cap} (policy.json budget)"
                );
                crate::org::record_refusal(&run_dir, "budget", &why, "policy.json:budget.tokens");
                self.interrupt_flag.trigger();
                self.emit(AgentEvent::Error { error: why }).await;
            }
        }
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

// ---------------------------------------------------------------------------
// Tests — ports of the deleted runtime engine's genuine-semantics tests
// (context_compressor.rs: parse/probe/repair/fast-trim). Constructor trivia,
// the engine's transcript builder and its private estimator are dropped:
// the core summarizer and `context_management::estimate_*` govern now.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::client::{Message, Role};

    fn msg(role: Role, content: &str) -> Message {
        Message::new(role, content.to_string())
    }

    fn cfg(protect_first_n: usize, protect_last_n: usize, retrim: usize) -> PreflightConfig {
        PreflightConfig {
            protect_first_n,
            protect_last_n,
            tool_result_retrim_chars: retrim,
            ..PreflightConfig::default()
        }
    }

    #[test]
    fn preflight_defaults_match_old_engine() {
        let c = PreflightConfig::default();
        assert!(c.enabled);
        assert_eq!(c.threshold_ratio, 0.50);
        assert_eq!(c.protect_first_n, 3);
        assert_eq!(c.protect_last_n, 4);
        assert_eq!(c.max_passes, 3);
        assert_eq!(c.summary_max_chars, 4_000);
        assert_eq!(c.source_max_chars, 50_000);
        assert_eq!(c.timeout_secs, 60);
        assert!(c.summary_model.is_none());
        assert_eq!(c.tool_result_retrim_chars, 2_000);
        assert!(c.tool_result_trim_exempt.is_empty());
    }

    // ── error-probe parsing (b) ────────────────────────────────────────

    #[test]
    fn parse_context_limit_anthropic() {
        let msg = "prompt is too long: 150000 tokens > 128000 maximum context length";
        assert_eq!(parse_context_limit_from_error(msg), Some(128_000));
    }

    #[test]
    fn parse_context_limit_openai() {
        let msg = "This model's maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens.";
        assert_eq!(parse_context_limit_from_error(msg), Some(128_000));
    }

    #[test]
    fn parse_context_limit_llamacpp() {
        let msg = "request (8968 tokens) exceeds the available context size (8448 tokens)";
        assert_eq!(parse_context_limit_from_error(msg), Some(8448));
    }

    #[test]
    fn parse_context_limit_none() {
        assert_eq!(parse_context_limit_from_error("some random error"), None);
    }

    #[test]
    fn parse_context_limit_rejects_small() {
        assert_eq!(parse_context_limit_from_error("limit is 100 tokens"), None);
    }

    #[test]
    fn next_probe_tier_steps_down() {
        assert_eq!(next_probe_tier(2_000_001), 2_000_000);
        assert_eq!(next_probe_tier(2_000_000), 1_000_000);
        assert_eq!(next_probe_tier(200_000), 128_000);
        assert_eq!(next_probe_tier(64_000), 32_000);
        assert_eq!(next_probe_tier(32_000), 32_000); // floor
    }

    // ── tool-pair repair (e) ───────────────────────────────────────────

    #[test]
    fn repair_tool_pairs_removes_orphaned() {
        let summary = format!(
            "{}\n\nstuff\n\n{}",
            crate::agent::llm_compressor::SUMMARY_PREFIX,
            crate::agent::llm_compressor::SUMMARY_END_MARKER
        );
        let mut messages = vec![
            msg(Role::System, "sys"),
            msg(Role::Assistant, &summary),
            msg(Role::Tool, "orphaned result"),
            msg(Role::User, "next question"),
        ];
        repair_tool_pairs(&mut messages);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].role, Role::User);
    }

    #[test]
    fn repair_tool_pairs_no_false_positives() {
        let mut messages = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "q"),
            msg(Role::Assistant, "calling tool"),
            msg(Role::Tool, "result"),
            msg(Role::User, "thanks"),
        ];
        repair_tool_pairs(&mut messages);
        assert_eq!(messages.len(), 5); // no change
    }

    // ── inline tool-result trim (c) ────────────────────────────────────

    #[test]
    fn fast_trim_protects_first_and_last_n() {
        let config = cfg(2, 2, 100);
        let big = "x".repeat(5_000);
        let mut history = vec![
            msg(Role::System, "sys"),
            msg(Role::Tool, &big), // index 1 — protected (first 2)
            msg(Role::User, "q"),
            msg(Role::Tool, &big),   // index 3 — trimmable
            msg(Role::User, "next"), // index 4 — protected (last 2)
            msg(Role::Tool, &big),   // index 5 — protected (last 2)
        ];
        let saved = fast_trim_tool_results(&mut history, &config);
        assert!(saved > 0);
        assert_eq!(history[1].content.len(), 5_000);
        assert_eq!(history[5].content.len(), 5_000);
        assert!(history[3].content.len() <= 200); // 100 + marker overhead
    }

    #[test]
    fn fast_trim_skips_images() {
        let config = cfg(0, 0, 100);
        let img = format!("data:image/{}", "x".repeat(5_000));
        let mut history = vec![msg(Role::Tool, &img)];
        assert_eq!(fast_trim_tool_results(&mut history, &config), 0);
        assert!(history[0].content.len() > 5_000);
    }

    #[test]
    fn fast_trim_skips_exempt_tools() {
        let config = PreflightConfig {
            tool_result_trim_exempt: vec!["KEEPME".to_string()],
            ..cfg(0, 0, 100)
        };
        let content = format!("KEEPME {}", "x".repeat(5_000));
        let mut history = vec![msg(Role::Tool, &content)];
        assert_eq!(fast_trim_tool_results(&mut history, &config), 0);
    }

    #[test]
    fn fast_trim_skips_small_results() {
        let config = cfg(0, 0, 2_000);
        let mut history = vec![msg(Role::Tool, "small result")];
        assert_eq!(fast_trim_tool_results(&mut history, &config), 0);
    }

    #[test]
    fn fast_trim_skips_non_tool_messages() {
        let config = cfg(0, 0, 100);
        let big = "x".repeat(5_000);
        let mut history = vec![
            msg(Role::User, &big),
            msg(Role::Assistant, &big),
            msg(Role::System, &big),
        ];
        assert_eq!(fast_trim_tool_results(&mut history, &config), 0);
        assert_eq!(history[0].content.len(), 5_000);
    }

    #[test]
    fn fast_trim_disabled_when_zero() {
        let config = cfg(0, 0, 0);
        let mut history = vec![msg(Role::Tool, &"x".repeat(5_000))];
        assert_eq!(fast_trim_tool_results(&mut history, &config), 0);
    }

    #[test]
    fn fast_trim_preserves_json_envelope() {
        // #5425 analog: the tool_call_id envelope must survive the trim.
        let config = cfg(0, 0, 1_000);
        let envelope = serde_json::json!({
            "content": "y".repeat(8_000),
            "tool_call_id": "call_y",
        })
        .to_string();
        let mut history = vec![msg(Role::Tool, &envelope)];
        let saved = fast_trim_tool_results(&mut history, &config);
        assert!(saved > 0);
        let parsed: serde_json::Value =
            serde_json::from_str(&history[0].content).expect("envelope must stay valid JSON");
        assert_eq!(parsed["tool_call_id"], "call_y");
        let inner = parsed["content"].as_str().unwrap();
        assert!(inner.contains("characters truncated"));
        assert!(inner.len() <= 1_100);
    }
}
