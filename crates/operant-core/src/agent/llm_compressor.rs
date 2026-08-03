//! LLM-based conversation compression — summarize middle turns via auxiliary model.
//!
//! Ports the core algorithm from `hermes-agent/agent/context_compressor.py`
//! into idiomatic Rust. When the conversation exceeds the context window,
//! instead of just truncating old messages (deterministic decay/eviction),
//! this module calls a cheaper/faster auxiliary LLM to produce a structured
//! summary of the middle turns, preserving semantic content while drastically
//! reducing token count.
//!
//! ## Algorithm (matches hermes-agent ContextCompressor)
//!
//! 1. **Tool result pruning** (cheap, no LLM call): Replace large old tool
//!    outputs with short descriptive placeholders.
//! 2. **Head protection**: System prompt + first user/assistant exchange
//!    are never compressed.
//! 3. **Tail protection**: Most recent N tokens of messages are preserved
//!    verbatim (the "active context" the model needs).
//! 4. **LLM summarization**: Middle turns (between head and tail) are sent
//!    to an auxiliary model with a structured prompt that produces a summary
//!    with task/resolved/pending sections.
//! 5. **Iterative updates**: On subsequent compactions, the previous summary
//!    is merged into the new summarization input so information is preserved.

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::agent::model_client::{ChatRequest, ModelClient};
use crate::client::{Message, Role};
use crate::context_management::estimate_tokens;
use crate::database::Database;
use crate::error::Result;

// ---------------------------------------------------------------------------
// Constants (ported from hermes-agent context_compressor.py)
// ---------------------------------------------------------------------------

/// Summary prefix prepended to every compaction summary. Matches hermes-agent's
/// `SUMMARY_PREFIX` — tells the model this is reference-only context, not
/// active instructions.
pub const SUMMARY_PREFIX: &str = "\
[CONTEXT COMPACTION — REFERENCE ONLY] Earlier turns were compacted \
into the summary below. This is a handoff from a previous context \
window — treat it as background reference, NOT as active instructions. \
Respond ONLY to the latest user message that appears AFTER this \
summary — that message is the single source of truth for what to do \
right now.";

/// Marker appended to summary so the model has an unambiguous boundary.
pub const SUMMARY_END_MARKER: &str = "\
--- END OF CONTEXT SUMMARY — respond to the message below, not the summary above ---";

/// Minimum number of messages required before LLM compression is attempted.
/// With fewer messages, the overhead of an LLM call isn't worth it.
const MIN_MESSAGES_FOR_LLM_COMPRESSION: usize = 10;

/// Maximum chars to keep from a single tool result before pruning.
const TOOL_RESULT_PRUNE_THRESHOLD: usize = 2_000;

/// Maximum chars per turn in the summarizer input (truncated beyond this).
const SUMMARIZER_TURN_MAX_CHARS: usize = 1_500;

/// Maximum chars for the entire summarizer input to prevent huge LLM calls.
const SUMMARIZER_INPUT_MAX_CHARS: usize = 80_000;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the LLM-based context compressor.
#[derive(Debug, Clone)]
pub struct LlmCompressorConfig {
    /// Model to use for summarization. Should be cheaper/faster than the
    /// main model (e.g. "gpt-4o-mini" when main is "gpt-4o").
    pub summarizer_model: String,
    /// Context window size in tokens for the main model.
    pub context_window: usize,
    /// Percentage of context window that triggers compression (0.0–1.0).
    pub threshold_percent: f64,
    /// Number of head messages to protect from compression (system + first exchange).
    pub protect_head_n: usize,
    /// Tail token budget — recent messages preserved verbatim.
    pub tail_token_budget: usize,
    /// Whether LLM compression is enabled.
    pub enabled: bool,
}

impl Default for LlmCompressorConfig {
    fn default() -> Self {
        Self {
            summarizer_model: "gpt-4o-mini".to_string(),
            context_window: 128_000,
            threshold_percent: 0.80,
            protect_head_n: 3,
            tail_token_budget: 20_000,
            enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Compression result
// ---------------------------------------------------------------------------

/// Result of an LLM compression pass.
#[derive(Debug, Clone)]
pub struct CompressionResult {
    /// The compressed message list (head + summary + tail).
    pub messages: Vec<Message>,
    /// Number of tokens before compression.
    pub tokens_before: usize,
    /// Number of tokens after compression.
    pub tokens_after: usize,
    /// The summary text that was generated.
    #[expect(
        dead_code,
        reason = "exposed for tests/inspection of the compression outcome"
    )]
    pub summary_text: String,
    /// Number of turns that were summarized.
    pub turns_summarized: usize,
}

// ---------------------------------------------------------------------------
// LLM Compressor
// ---------------------------------------------------------------------------

/// LLM-based context compressor.
///
/// Uses an auxiliary model to summarize old conversation turns when the
/// context window fills up, preserving semantic content while reducing
/// token count. Falls back to deterministic truncation if the LLM call fails.
pub struct LlmCompressor {
    config: LlmCompressorConfig,
    /// The previous summary text, for iterative updates across compactions.
    previous_summary: Option<String>,
    /// Number of compressions performed in this session.
    compression_count: usize,
    /// Timestamp (seconds since epoch) after which compression is allowed again.
    /// Set on compression failure to implement anti-thrash cooldown.
    cooldown_until: f64,
    /// Number of consecutive compression failures (resets on success).
    consecutive_failures: usize,
    /// Optional database for persisting cooldown state across restarts.
    /// When set, cooldown_until and consecutive_failures are persisted
    /// to session_metadata so they survive process restarts. Matches
    /// hermes-agent's ContextCompressor persistence pattern.
    database: Option<Arc<Database>>,
    /// Session ID for database persistence. Required when database is set.
    session_id: Option<String>,
}

impl LlmCompressor {
    /// Create a new LLM compressor with the given configuration.
    pub fn new(config: LlmCompressorConfig) -> Self {
        Self {
            config,
            previous_summary: None,
            compression_count: 0,
            cooldown_until: 0.0,
            consecutive_failures: 0,
            database: None,
            session_id: None,
        }
    }

    /// Bind a database and session ID for cooldown persistence.
    /// When set, compression failure cooldowns are persisted to
    /// session_metadata so they survive process restarts.
    ///
    /// Idempotent: safe to call multiple times. Loads existing cooldown
    /// state from the database on first bind.
    pub fn bind_persistence(&mut self, database: Arc<Database>, session_id: String) {
        self.database = Some(database);
        self.session_id = Some(session_id);
        // Load existing cooldown state from DB so restarts don't
        // immediately re-trigger compression after a recent failure.
        self.load_cooldown_from_db();
    }

    /// Returns the currently-bound session ID, if any.
    /// Used by the agent to check whether persistence is already bound
    /// before calling `bind_persistence` again.
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Load cooldown state from the database (called by bind_persistence).
    /// Restores `cooldown_until` and `consecutive_failures` from
    /// session_metadata if they were persisted by a previous run.
    fn load_cooldown_from_db(&mut self) {
        let db = match self.database.as_ref() {
            Some(db) => Arc::clone(db),
            None => return,
        };
        let session_id = match self.session_id.as_ref() {
            Some(id) => id.clone(),
            None => return,
        };

        // Load cooldown_until
        if let Some(val) = db.get_session_metadata(&session_id, "compression_cooldown_until") {
            if let Ok(ts) = val.parse::<f64>() {
                self.cooldown_until = ts;
                debug!(
                    cooldown_until = ts,
                    "Loaded compression cooldown from database"
                );
            }
        }

        // Load consecutive_failures
        if let Some(val) = db.get_session_metadata(&session_id, "compression_consecutive_failures")
        {
            if let Ok(count) = val.parse::<usize>() {
                self.consecutive_failures = count;
                debug!(
                    consecutive_failures = count,
                    "Loaded compression failure count from database"
                );
            }
        }
    }

    /// Anti-thrash: return true if compression is in cooldown after a recent failure.
    /// Cooldown scales with failure count: 5s, 10s, 20s, 40s (capped at 60s).
    pub fn is_in_cooldown(&self) -> bool {
        if self.cooldown_until <= 0.0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        now < self.cooldown_until
    }

    /// Record a compression failure and arm the cooldown.
    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        let base = 5.0_f64;
        let delay = (base * 2.0_f64.powi(self.consecutive_failures as i32 - 1)).min(60.0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.cooldown_until = now + delay;
        warn!(
            failure = self.consecutive_failures,
            cooldown_secs = delay,
            "Compression failure — anti-thrash cooldown armed"
        );
        // Persist to database so cooldown survives restarts
        self.persist_cooldown_to_db();
    }

    /// Persist cooldown state to the database.
    /// Stores `cooldown_until` and `consecutive_failures` in session_metadata
    /// so they survive process restarts. Matches hermes-agent's
    /// ContextCompressor cooldown persistence pattern.
    fn persist_cooldown_to_db(&self) {
        let db = match self.database.as_ref() {
            Some(db) => Arc::clone(db),
            None => return,
        };
        let session_id = match self.session_id.as_ref() {
            Some(id) => id.clone(),
            None => return,
        };

        if let Err(e) = db.set_session_metadata(
            &session_id,
            "compression_cooldown_until",
            &self.cooldown_until.to_string(),
        ) {
            warn!(error = %e, "Failed to persist compression cooldown to database");
        }
        if let Err(e) = db.set_session_metadata(
            &session_id,
            "compression_consecutive_failures",
            &self.consecutive_failures.to_string(),
        ) {
            warn!(error = %e, "Failed to persist compression failure count to database");
        }
    }

    /// Clear the cooldown (called on success or session reset).
    fn clear_cooldown(&mut self) {
        self.consecutive_failures = 0;
        self.cooldown_until = 0.0;
        // Persist cleared state to database
        self.persist_cooldown_to_db();
    }

    /// Reset per-session state (call on /new or /reset).
    pub fn reset(&mut self) {
        self.previous_summary = None;
        self.compression_count = 0;
        self.clear_cooldown();
    }

    /// Check whether compression should be triggered based on token estimates.
    pub fn should_compress(&self, estimated_tokens: usize) -> bool {
        if !self.config.enabled {
            return false;
        }
        let threshold = self.config.context_window as f64 * self.config.threshold_percent;
        estimated_tokens > threshold as usize
    }

    /// Compress the conversation using LLM summarization.
    ///
    /// This is the main entry point. It:
    /// 1. Prunes old tool results
    /// 2. Splits messages into head / middle / tail
    /// 3. Calls the auxiliary LLM to summarize the middle
    /// 4. Reassembles: head + summary message + tail
    pub async fn compress(
        &mut self,
        messages: Vec<Message>,
        client: &Arc<dyn ModelClient>,
    ) -> Result<CompressionResult> {
        // Anti-thrash: skip LLM compression if we're in cooldown after recent failures.
        if self.is_in_cooldown() {
            debug!(
                cooldown_until = self.cooldown_until,
                failures = self.consecutive_failures,
                "LLM compression in anti-thrash cooldown — skipping"
            );
            let tokens_before = crate::context_management::estimate_total_tokens(&messages);
            return Ok(CompressionResult {
                messages,
                tokens_before,
                tokens_after: tokens_before,
                summary_text: String::new(),
                turns_summarized: 0,
            });
        }

        let tokens_before = crate::context_management::estimate_total_tokens(&messages);

        if messages.len() < MIN_MESSAGES_FOR_LLM_COMPRESSION {
            debug!(
                messages = messages.len(),
                min = MIN_MESSAGES_FOR_LLM_COMPRESSION,
                "Not enough messages for LLM compression"
            );
            return Ok(CompressionResult {
                messages,
                tokens_before,
                tokens_after: tokens_before,
                summary_text: String::new(),
                turns_summarized: 0,
            });
        }

        // Step 1: Prune old tool results (cheap, no LLM call)
        let pruned = self.prune_tool_results(&messages);

        // Step 2: Split into head / middle / tail
        let head_n = self.config.protect_head_n.min(pruned.len());
        let (head, middle_and_tail) = pruned.split_at(head_n);

        // Find the tail boundary: walk backward from the end, accumulating
        // tokens until we exceed tail_token_budget.
        let tail_start = self.find_tail_start(middle_and_tail);
        let (middle, tail) = middle_and_tail.split_at(tail_start);

        if middle.is_empty() {
            debug!("Nothing to compress after head/tail protection");
            return Ok(CompressionResult {
                messages: pruned,
                tokens_before,
                tokens_after: tokens_before,
                summary_text: String::new(),
                turns_summarized: 0,
            });
        }

        let turns_summarized = middle.len();
        info!(
            head = head.len(),
            middle = middle.len(),
            tail = tail.len(),
            "LLM compression: summarizing middle turns"
        );

        // Step 3: Build summarizer input and call LLM
        let summary = match self
            .generate_summary(client, middle, &self.previous_summary)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "LLM summarization failed — falling back to truncation");
                self.record_failure();
                self.fallback_summary(middle)
            }
        };

        // Step 4: Build the summary message
        let summary_content = format!(
            "{}\n\n{}\n\n{}",
            SUMMARY_PREFIX, summary, SUMMARY_END_MARKER
        );
        let summary_msg = Message::system(&summary_content);

        // Step 5: Reassemble
        let mut result: Vec<Message> = Vec::new();
        result.extend_from_slice(head);
        result.push(summary_msg.clone());
        result.extend_from_slice(tail);

        let tokens_after = crate::context_management::estimate_total_tokens(&result);

        // Store for iterative updates
        self.previous_summary = Some(summary.clone());
        self.compression_count += 1;
        self.clear_cooldown();

        info!(
            tokens_before,
            tokens_after,
            saved_pct = if tokens_before > 0 {
                ((tokens_before - tokens_after) * 100) / tokens_before
            } else {
                0
            },
            turns_summarized,
            "LLM compression complete"
        );

        Ok(CompressionResult {
            messages: result,
            tokens_before,
            tokens_after,
            summary_text: summary,
            turns_summarized,
        })
    }

    // -----------------------------------------------------------------------
    // Tool result pruning
    // -----------------------------------------------------------------------

    /// Replace large old tool results with short placeholders.
    ///
    /// Only prunes tool messages that are NOT in the recency reserve
    /// (last 6 messages). Matches hermes-agent's pre-compression pruning pass.
    fn prune_tool_results(&self, messages: &[Message]) -> Vec<Message> {
        let n = messages.len();
        let recency_reserve = 6.min(n);

        messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                if msg.role == Role::Tool && (n - i) > recency_reserve {
                    let content_len = msg.content.chars().count();
                    if content_len > TOOL_RESULT_PRUNE_THRESHOLD {
                        // Truncate but preserve the first few lines for context
                        let preview: String = msg.content.chars().take(200).collect();
                        Message {
                            content: format!(
                                "{}\n\n[...truncated from {} chars]",
                                preview, content_len
                            ),
                            ..msg.clone()
                        }
                    } else {
                        msg.clone()
                    }
                } else {
                    msg.clone()
                }
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Tail boundary detection
    // -----------------------------------------------------------------------

    /// Find where the tail starts: walk backward from the end of `msgs`,
    /// accumulating token estimates until we exceed `tail_token_budget`.
    fn find_tail_start(&self, msgs: &[Message]) -> usize {
        let budget = self.config.tail_token_budget;
        let mut tokens = 0usize;

        for (i, msg) in msgs.iter().enumerate().rev() {
            tokens += estimate_tokens(&msg.content) + 4; // +4 for role overhead
            if tokens >= budget {
                // Return the index of the first message in the tail
                return i;
            }
        }

        // All messages fit in the tail budget — nothing to compress
        0
    }

    // -----------------------------------------------------------------------
    // LLM summarization
    // -----------------------------------------------------------------------

    /// Generate a structured summary of the middle turns using the auxiliary LLM.
    async fn generate_summary(
        &self,
        client: &Arc<dyn ModelClient>,
        middle: &[Message],
        previous_summary: &Option<String>,
    ) -> Result<String> {
        let summarizer_input = self.build_summarizer_input(middle, previous_summary);

        let request = ChatRequest::new(self.config.summarizer_model.clone(), summarizer_input)
            .with_stream(false);

        let response = client.chat(request).await?;
        let summary = response
            .choices
            .iter()
            .find_map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(summary.trim().to_string())
    }

    /// Build the message list for the summarizer LLM call.
    ///
    /// The summarizer gets:
    /// 1. A system prompt with structured output instructions
    /// 2. The middle turns (truncated to fit budget)
    /// 3. Optionally, the previous summary for iterative updates
    fn build_summarizer_input(
        &self,
        middle: &[Message],
        previous_summary: &Option<String>,
    ) -> Vec<Message> {
        let mut system_prompt = String::from(
            "You are a conversation summarizer. Your task is to compress the following \
             conversation turns into a concise, structured summary that preserves all \
             key information.\n\n\
             Format your summary as:\n\
             ## Task\n\
             Brief description of what the user asked and what was being done.\n\n\
             ## Key Decisions\n\
             Important decisions made, constraints, or requirements established.\n\n\
             ## Progress\n\
             What was accomplished (tools called, files changed, tests run).\n\n\
             ## Context\n\
             Any other important context the next turns need to know.\n\n\
             Be concise but complete. Preserve file paths, function names, error messages, \
             and specific technical details. Do NOT include filler or pleasantries.",
        );

        if previous_summary.is_some() {
            system_prompt.push_str(
                "\n\nIMPORTANT: A previous summary exists. Merge the new information above \
                 with the previous summary. Do NOT repeat information already captured in \
                 the previous summary — only add new developments.",
            );
        }

        let mut messages = vec![Message::system(&system_prompt)];

        // Add previous summary as a user message for context
        if let Some(prev) = previous_summary {
            messages.push(Message::user(format!(
                "[Previous summary for reference]\n{}",
                prev
            )));
        }

        // Add the middle turns, truncated to fit the input budget
        let mut total_chars = 0usize;
        for msg in middle {
            let truncated_content = if msg.content.chars().count() > SUMMARIZER_TURN_MAX_CHARS {
                let truncated: String = msg
                    .content
                    .chars()
                    .take(SUMMARIZER_TURN_MAX_CHARS)
                    .collect();
                format!("{} [...]", truncated)
            } else {
                msg.content.clone()
            };

            total_chars += truncated_content.len();
            if total_chars > SUMMARIZER_INPUT_MAX_CHARS {
                debug!(
                    "Summarizer input truncated at {} chars (budget: {})",
                    total_chars, SUMMARIZER_INPUT_MAX_CHARS
                );
                break;
            }

            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool Result",
                Role::System => "System",
            };

            messages.push(Message::user(format!(
                "[{}]\n{}",
                role_label, truncated_content
            )));
        }

        messages.push(Message::user(
            "Now produce the structured summary of the conversation above.".to_string(),
        ));

        messages
    }

    // -----------------------------------------------------------------------
    // Fallback (deterministic truncation when LLM fails)
    // -----------------------------------------------------------------------

    /// Deterministic fallback summary when the LLM call fails.
    ///
    /// Produces a simple "continuity anchor" that preserves the most recent
    /// user request and any file paths / function names mentioned, without
    /// requiring an LLM call.
    fn fallback_summary(&self, middle: &[Message]) -> String {
        let mut summary =
            String::from("[Deterministic fallback — LLM summarization unavailable]\n\n");

        // Find the most recent user message and the most recent assistant message
        let last_user = middle
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| &m.content);

        let last_assistant = middle
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| &m.content);

        if let Some(user_msg) = last_user {
            let truncated: String = user_msg.chars().take(500).collect();
            summary.push_str(&format!("Last user request: {}\n\n", truncated));
        }

        if let Some(assistant_msg) = last_assistant {
            let truncated: String = assistant_msg.chars().take(500).collect();
            summary.push_str(&format!("Last assistant response (excerpt): {}", truncated));
        }

        summary
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: Role, content: impl Into<String>) -> Message {
        Message::new(role, content.into())
    }

    #[test]
    fn test_should_compress_below_threshold() {
        let config = LlmCompressorConfig {
            context_window: 128_000,
            threshold_percent: 0.80,
            ..Default::default()
        };
        let compressor = LlmCompressor::new(config);
        assert!(!compressor.should_compress(50_000));
        assert!(!compressor.should_compress(100_000));
        assert!(compressor.should_compress(110_000));
        assert!(compressor.should_compress(128_000));
    }

    #[test]
    fn test_should_compress_disabled() {
        let config = LlmCompressorConfig {
            enabled: false,
            ..Default::default()
        };
        let compressor = LlmCompressor::new(config);
        assert!(!compressor.should_compress(200_000));
    }

    #[test]
    fn test_prune_tool_results_truncates_large_results() {
        let config = LlmCompressorConfig::default();
        let compressor = LlmCompressor::new(config);

        // Need 13+ messages so index 3 falls outside the recency reserve (last 6).
        let mut messages = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "let me check"),
            make_msg(Role::Tool, "x".repeat(5000)), // large, OLD tool result (index 3)
        ];
        // Fill middle with enough messages to push index 3 outside the reserve
        for i in 0..8 {
            messages.push(make_msg(Role::User, format!("msg {}", i)));
            messages.push(make_msg(Role::Assistant, format!("reply {}", i)));
        }
        // Final large tool result in the recency reserve
        messages.push(make_msg(Role::Tool, "y".repeat(3000))); // index ~20, in reserve
        messages.push(make_msg(Role::Assistant, "answer"));

        let pruned = compressor.prune_tool_results(&messages);

        // First tool result (index 3) should be pruned (outside recency reserve)
        assert!(pruned[3].content.contains("truncated"));
        // Second tool result (near end, in recency reserve) should be preserved
        let last_tool_idx = pruned.iter().rposition(|m| m.role == Role::Tool).unwrap();
        assert_eq!(pruned[last_tool_idx].content, "y".repeat(3000));
    }

    #[test]
    fn test_prune_tool_results_preserves_small_results() {
        let config = LlmCompressorConfig::default();
        let compressor = LlmCompressor::new(config);

        let messages = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Tool, "short result"),
            make_msg(Role::Assistant, "ok"),
        ];

        let pruned = compressor.prune_tool_results(&messages);
        assert_eq!(pruned[2].content, "short result");
    }

    #[test]
    fn test_find_tail_start() {
        let config = LlmCompressorConfig {
            tail_token_budget: 100, // ~400 chars
            ..Default::default()
        };
        let compressor = LlmCompressor::new(config);

        // 8 messages, each ~100 chars = ~25 tokens each
        let messages: Vec<Message> = (0..8)
            .map(|_i| make_msg(Role::User, format!("message {}", "x".repeat(100))))
            .collect();

        let tail_start = compressor.find_tail_start(&messages);
        // With budget of 100 tokens (~400 chars), we should keep ~4 messages
        assert!(tail_start > 0);
        assert!(tail_start < messages.len());
    }

    #[test]
    fn test_find_tail_start_all_fit() {
        let config = LlmCompressorConfig {
            tail_token_budget: 100_000,
            ..Default::default()
        };
        let compressor = LlmCompressor::new(config);

        let messages = vec![
            make_msg(Role::User, "short"),
            make_msg(Role::Assistant, "response"),
        ];

        let tail_start = compressor.find_tail_start(&messages);
        assert_eq!(tail_start, 0); // all messages fit in tail
    }

    #[test]
    fn test_fallback_summary() {
        let config = LlmCompressorConfig::default();
        let compressor = LlmCompressor::new(config);

        let middle = vec![
            make_msg(Role::User, "please implement the login feature"),
            make_msg(Role::Assistant, "I'll create auth.rs"),
            make_msg(Role::Tool, "file created"),
            make_msg(Role::User, "now add tests"),
        ];

        let summary = compressor.fallback_summary(&middle);
        assert!(summary.contains("Deterministic fallback"));
        assert!(summary.contains("now add tests"));
        assert!(summary.contains("I'll create auth.rs"));
    }

    #[test]
    fn test_summarizer_input_builds_correctly() {
        let config = LlmCompressorConfig::default();
        let compressor = LlmCompressor::new(config);

        let middle = vec![
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi there"),
            make_msg(Role::Tool, "result"),
            make_msg(Role::User, "do something"),
        ];

        let input = compressor.build_summarizer_input(&middle, &None);
        // System prompt + 4 middle turns + 1 instruction = 6 messages
        assert_eq!(input.len(), 6);
        assert_eq!(input[0].role, Role::System);
        assert!(input[0].content.contains("conversation summarizer"));
    }

    #[test]
    fn test_summarizer_input_with_previous_summary() {
        let config = LlmCompressorConfig::default();
        let compressor = LlmCompressor::new(config);

        let middle = vec![make_msg(Role::User, "hello")];
        let prev = Some("Previous summary text".to_string());

        let input = compressor.build_summarizer_input(&middle, &prev);
        // System + prev summary + 1 middle + instruction = 4 messages
        assert_eq!(input.len(), 4);
        // System prompt should mention merging
        assert!(input[0].content.contains("previous summary"));
        // Second message should contain the previous summary
        assert!(input[1].content.contains("Previous summary text"));
    }

    #[test]
    fn test_reset_clears_state() {
        let config = LlmCompressorConfig::default();
        let mut compressor = LlmCompressor::new(config);
        compressor.previous_summary = Some("old summary".to_string());
        compressor.compression_count = 5;

        compressor.reset();

        assert!(compressor.previous_summary.is_none());
        assert_eq!(compressor.compression_count, 0);
    }

    #[test]
    fn test_persistence_round_trip() {
        // Validates that cooldown state survives across separate compressor
        // instances via the database, matching hermes-agent's persistence
        // pattern where compression failure cooldowns persist to SQLite.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::Database::init(db_path.clone()).unwrap());
        let session_id = "test-session-persist".to_string();

        // Create a session row first (FK constraint)
        db.save_session(
            &session_id,
            None,
            "test",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();

        // --- Instance 1: record a failure ---
        let config = LlmCompressorConfig::default();
        let mut compressor1 = LlmCompressor::new(config.clone());
        assert!(!compressor1.is_in_cooldown());

        compressor1.bind_persistence(Arc::clone(&db), session_id.clone());
        // Directly trigger the failure path by calling record_failure (private)
        // We simulate it by manually setting the fields and persisting.
        compressor1.consecutive_failures = 1;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        compressor1.cooldown_until = now + 300.0; // 5 minutes from now
        compressor1.persist_cooldown_to_db();

        assert!(compressor1.is_in_cooldown());

        // --- Instance 2: load from DB (simulates restart) ---
        let mut compressor2 = LlmCompressor::new(config);
        assert!(!compressor2.is_in_cooldown()); // not yet loaded

        compressor2.bind_persistence(Arc::clone(&db), session_id);
        // bind_persistence calls load_cooldown_from_db internally
        assert!(compressor2.is_in_cooldown()); // restored from DB!
        assert_eq!(compressor2.consecutive_failures, 1);
    }

    #[test]
    fn test_persistence_clear_round_trip() {
        // Validates that clearing cooldown also persists to DB.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_clear.db");
        let db = Arc::new(crate::database::Database::init(db_path).unwrap());
        let session_id = "test-session-clear".to_string();
        db.save_session(
            &session_id,
            None,
            "test",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();

        let config = LlmCompressorConfig::default();
        let mut compressor = LlmCompressor::new(config.clone());
        compressor.bind_persistence(Arc::clone(&db), session_id.clone());

        // Record a failure
        compressor.consecutive_failures = 2;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        compressor.cooldown_until = now + 600.0;
        compressor.persist_cooldown_to_db();
        assert!(compressor.is_in_cooldown());

        // Clear cooldown
        compressor.clear_cooldown();
        assert!(!compressor.is_in_cooldown());
        assert_eq!(compressor.consecutive_failures, 0);

        // New instance should see cleared state
        let mut compressor2 = LlmCompressor::new(config);
        compressor2.bind_persistence(Arc::clone(&db), session_id);
        assert!(!compressor2.is_in_cooldown());
        assert_eq!(compressor2.consecutive_failures, 0);
    }
}
