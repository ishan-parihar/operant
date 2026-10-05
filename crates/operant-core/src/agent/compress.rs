//! `compress` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{Message, Role, Usage};
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
    pub(crate) async fn compress_context_overflow(&self, messages: Vec<Message>) -> Vec<Message> {
        let tokens_before = self.estimate_current_tokens(&messages);
        let messages_before = messages.len();
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
        self.reinject_todos_after_compression(compressed)
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
