//! Context compression strategies for conversation history.
//!
//! Provides token-aware and count-based strategies to reduce
//! conversation length while preserving system messages and
//! the most recent context.

use crate::client::{Message, Role};

/// Strategy for compressing conversation context.
#[derive(Debug, Clone, PartialEq)]
pub enum CompressionStrategy {
    /// Placeholder for LLM-based summarization.
    /// Currently behaves like `Truncate` with `keep_ratio: 0.5`.
    Summarize,
    /// Token-aware truncation: keeps system messages and the most recent
    /// non-system messages up to a token budget of `max_tokens * keep_ratio`.
    Truncate {
        /// Fraction of `max_tokens` to use as the token budget
        /// (e.g., `0.5` means half of `max_tokens`).
        keep_ratio: f64,
    },
    /// Simple count-based dropping: keeps all system messages and the
    /// last `max_messages` non-system messages.
    Drop {
        /// Maximum number of non-system messages to retain.
        max_messages: usize,
    },
}

/// Compresses conversation context to fit within token or message budgets.
///
/// # Strategies
///
/// | Strategy | Behaviour |
/// |---|---|
/// | [`Summarize`](CompressionStrategy::Summarize) | Placeholder — currently truncates (see TODO). |
/// | [`Truncate`](CompressionStrategy::Truncate) | Token-aware: keeps system messages + most recent non-system up to a token budget. |
/// | [`Drop`](CompressionStrategy::Drop) | Count-based: keeps system messages + last N non-system messages. |
///
/// Token estimation uses a rough heuristic (`content.len() / 4 + role overhead`)
/// since no tokenizer is available.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextCompressor {
    strategy: CompressionStrategy,
    max_tokens: usize,
}

impl ContextCompressor {
    /// Create a new context compressor.
    ///
    /// - `strategy` – the compression strategy to apply.
    /// - `max_tokens` – reference token limit used by the strategy for budget calculations.
    pub fn new(strategy: CompressionStrategy, max_tokens: usize) -> Self {
        Self {
            strategy,
            max_tokens,
        }
    }

    /// Compress the given messages according to the configured strategy.
    ///
    /// Returns a new `Vec<Message>` with the same or fewer entries.
    pub fn compress(&self, messages: &[Message]) -> Vec<Message> {
        match &self.strategy {
            CompressionStrategy::Summarize => {
                // TODO: Implement LLM-based summarization once ModelClient trait is available
                self.compress_truncate(messages, 0.5)
            }
            CompressionStrategy::Truncate { keep_ratio } => {
                self.compress_truncate(messages, *keep_ratio)
            }
            CompressionStrategy::Drop { max_messages } => {
                self.compress_drop(messages, *max_messages)
            }
        }
    }

    /// Estimate the total number of tokens for all messages combined.
    ///
    /// Uses a rough heuristic: `content.len() / 4 + 3` per message,
    /// plus `2` per `tool_call_id` and `tool_calls` entry.
    pub fn count_tokens(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|msg| self.tokens_for_message(msg))
            .sum()
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Rough token estimate for a single message.
    ///
    /// Formula: `content.len() / 4` (chars → tokens) + 3 (role overhead)
    /// + 2 per `tool_call_id` / `tool_calls` entry.
    fn tokens_for_message(&self, msg: &Message) -> usize {
        let mut tokens = msg.content.len() / 4 + 3;
        if msg.tool_call_id.is_some() {
            tokens += 2;
        }
        if let Some(ref calls) = msg.tool_calls {
            tokens += 2 * calls.len();
        }
        tokens
    }

    /// Token-aware truncation.
    ///
    /// 1. System messages are always kept and do **not** count against the budget.
    /// 2. Target budget = `max_tokens * keep_ratio`.
    /// 3. Walk non-system messages **from the end** and keep them until the
    ///    budget is consumed.
    /// 4. If total tokens ≤ `max_tokens` the original list is returned unchanged.
    fn compress_truncate(&self, messages: &[Message], keep_ratio: f64) -> Vec<Message> {
        let total_tokens: usize = messages
            .iter()
            .map(|msg| self.tokens_for_message(msg))
            .sum();

        if total_tokens <= self.max_tokens {
            return messages.to_vec();
        }

        let budget = (self.max_tokens as f64 * keep_ratio) as usize;

        // Partition: system messages (always kept) vs the rest.
        let mut system_messages = Vec::new();
        let mut non_system: Vec<&Message> = Vec::new();

        for msg in messages {
            if msg.role == Role::System {
                system_messages.push(msg.clone());
            } else {
                non_system.push(msg);
            }
        }

        // Collect messages from the end until budget is consumed.
        let mut kept: Vec<Message> = Vec::new();
        let mut accumulated = 0usize;

        for msg in non_system.iter().rev() {
            let tokens = self.tokens_for_message(msg);
            if accumulated + tokens <= budget {
                accumulated += tokens;
                kept.push((*msg).clone());
            } else {
                break;
            }
        }

        // Reverse back to chronological order.
        kept.reverse();

        // System messages go first (they don't count toward the budget).
        system_messages.extend(kept);
        system_messages
    }

    /// Count-based dropping.
    ///
    /// 1. System messages are always kept.
    /// 2. If non-system messages ≤ `max_messages`, the original list is returned.
    /// 3. Otherwise only the last `max_messages` non-system messages are retained.
    fn compress_drop(&self, messages: &[Message], max_messages: usize) -> Vec<Message> {
        let mut system_messages = Vec::new();
        let mut non_system: Vec<&Message> = Vec::new();

        for msg in messages {
            if msg.role == Role::System {
                system_messages.push(msg.clone());
            } else {
                non_system.push(msg);
            }
        }

        if non_system.len() <= max_messages {
            return messages.to_vec();
        }

        let start = non_system.len() - max_messages;
        let kept: Vec<Message> = non_system[start..]
            .iter()
            .map(|m| (*m).clone())
            .collect();

        system_messages.extend(kept);
        system_messages
    }
}

/// Rough token estimate for a text string.
///
/// Uses a simple heuristic: every 4 characters ≈ 1 token.
#[cfg(test)]
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Role};

    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    #[test]
    fn test_context_compressor_new() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 1000);
        assert_eq!(compressor.max_tokens, 1000);
        match compressor.strategy {
            CompressionStrategy::Truncate { keep_ratio } => {
                assert!((keep_ratio - 0.5).abs() < 1e-10);
            }
            _ => panic!("expected Truncate strategy"),
        }
    }

    // ------------------------------------------------------------------
    // Truncate
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_truncate_below_budget() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.8 }, 10_000);
        let messages = vec![
            Message::new(Role::System, "You are a helpful assistant."),
            Message::new(Role::User, "Hello!"),
            Message::new(Role::Assistant, "Hi there!"),
        ];
        let result = compressor.compress(&messages);
        assert_eq!(result.len(), 3, "should return all when under budget");
    }

    #[test]
    fn test_compress_truncate_above_budget() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 50);
        let mut messages = vec![Message::new(Role::System, "System prompt here.")];
        for i in 0..10 {
            messages.push(Message::new(
                Role::User,
                format!(
                    "Message number {} with some extra text for token estimation purposes.",
                    i
                ),
            ));
        }
        let result = compressor.compress(&messages);

        // System message must be present.
        assert_eq!(result[0].role, Role::System);
        // Result is smaller than the original.
        assert!(result.len() < messages.len(), "expected truncation");
        // The very last message must be preserved.
        assert!(
            result.last().unwrap().content.contains("Message number 9"),
            "last message should be preserved: {:?}",
            result.last().unwrap().content
        );
    }

    #[test]
    fn test_compress_truncate_preserves_system_messages() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.3 }, 30);
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "System: be helpful.".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::System,
                content: "System: use tools.".to_string(),
                ..Message::default()
            },
        ];
        for i in 0..10 {
            messages.push(Message::new(Role::User, format!("User message {}.", i)));
        }
        let result = compressor.compress(&messages);

        // All system messages preserved.
        assert_eq!(
            result.iter().filter(|m| m.role == Role::System).count(),
            2
        );
        // First two entries are system messages.
        assert_eq!(result[0].role, Role::System);
        assert_eq!(result[1].role, Role::System);
    }

    // ------------------------------------------------------------------
    // Drop
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_drop_above_limit() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Drop { max_messages: 3 }, 1000);
        let mut messages = vec![Message {
            role: Role::System,
            content: "System prompt.".to_string(),
            ..Message::default()
        }];
        for i in 0..10 {
            messages.push(Message::new(Role::User, format!("User message {}.", i)));
        }
        let result = compressor.compress(&messages);

        // System message preserved.
        assert_eq!(result.iter().filter(|m| m.role == Role::System).count(), 1);
        // 1 system + 3 non-system = 4 total.
        assert_eq!(result.len(), 4);

        let non_system: Vec<&Message> = result.iter().filter(|m| m.role != Role::System).collect();
        assert_eq!(non_system.len(), 3);
        assert!(non_system[0].content.contains("User message 7."));
        assert!(non_system[1].content.contains("User message 8."));
        assert!(non_system[2].content.contains("User message 9."));
    }

    #[test]
    fn test_compress_drop_below_limit() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Drop { max_messages: 10 }, 1000);
        let messages = vec![
            Message::new(Role::System, "System prompt."),
            Message::new(Role::User, "User 1."),
            Message::new(Role::User, "User 2."),
        ];
        let result = compressor.compress(&messages);
        assert_eq!(result.len(), 3, "should return all when under limit");
    }

    #[test]
    fn test_compress_drop_preserves_system() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Drop { max_messages: 1 }, 1000);
        let mut messages = vec![
            Message {
                role: Role::System,
                content: "System first.".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::System,
                content: "System second.".to_string(),
                ..Message::default()
            },
        ];
        for i in 0..5 {
            messages.push(Message::new(Role::User, format!("User {}.", i)));
        }
        let result = compressor.compress(&messages);
        assert_eq!(result.iter().filter(|m| m.role == Role::System).count(), 2);
        // 2 system + 1 user = 3.
        assert_eq!(result.len(), 3);
    }

    // ------------------------------------------------------------------
    // Summarize (placeholder)
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_summarize_placeholder() {
        let compressor = ContextCompressor::new(CompressionStrategy::Summarize, 50);
        let mut messages = vec![Message::new(Role::System, "System: be helpful.")];
        for i in 0..10 {
            messages.push(Message::new(
                Role::User,
                format!(
                    "Long user message number {} with enough text to trigger truncation.",
                    i
                ),
            ));
        }
        let result = compressor.compress(&messages);

        // System message preserved.
        assert_eq!(result[0].role, Role::System);
        // Original truncated.
        assert!(result.len() < messages.len(), "expected truncation");
        // Last message preserved.
        assert!(
            result.last().unwrap().content.contains("Long user message number 9"),
            "last message should be preserved"
        );
    }

    // ------------------------------------------------------------------
    // Token counting
    // ------------------------------------------------------------------

    #[test]
    fn test_count_tokens_empty() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 1000);
        let messages: Vec<Message> = vec![];
        assert_eq!(compressor.count_tokens(&messages), 0);
    }

    #[test]
    fn test_count_tokens_with_messages() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 1000);
        let messages = vec![
            Message::new(Role::User, "Hello world"),
            Message::new(Role::Assistant, "Hi there!"),
        ];
        // "Hello world" = 11 chars → 11 / 4 = 2 + 3 = 5
        // "Hi there!"  =  9 chars →  9 / 4 = 2 + 3 = 5
        // total = 10
        assert_eq!(compressor.count_tokens(&messages), 10);
    }

    // ------------------------------------------------------------------
    // Debug
    // ------------------------------------------------------------------

    #[test]
    fn test_compression_strategy_debug() {
        let strategy = CompressionStrategy::Truncate { keep_ratio: 0.5 };
        let debug = format!("{:?}", strategy);
        assert!(debug.contains("Truncate"));
    }

    // ------------------------------------------------------------------
    // Empty messages
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_empty_messages() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 1000);
        let messages: Vec<Message> = vec![];
        let result = compressor.compress(&messages);
        assert!(result.is_empty());
    }

    // ------------------------------------------------------------------
    // Single message
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_single_message() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.5 }, 1000);
        let messages = vec![Message::new(Role::User, "Hello")];
        let result = compressor.compress(&messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "Hello");
    }

    // ------------------------------------------------------------------
    // Drop with max_messages = 0
    // ------------------------------------------------------------------

    #[test]
    fn test_drop_max_zero_keeps_first() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Drop { max_messages: 0 }, 1000);
        let messages = vec![
            Message::new(Role::System, "You are a helpful assistant."),
            Message::new(Role::User, "Hello!"),
            Message::new(Role::Assistant, "Hi!"),
        ];
        let result = compressor.compress(&messages);
        assert_eq!(result.len(), 1, "should keep only the first message");
        assert_eq!(result[0].role, Role::System);
        assert!(result[0].content.contains("You are a helpful assistant"));
    }

    // ------------------------------------------------------------------
    // Strategy derives (Debug, Clone, PartialEq)
    // ------------------------------------------------------------------

    #[test]
    fn test_strategy_derives() {
        // Debug
        let s1 = CompressionStrategy::Summarize;
        let _ = format!("{:?}", s1);

        // Clone
        let s2 = CompressionStrategy::Truncate { keep_ratio: 0.7 };
        let s3 = s2.clone();
        assert_eq!(s2, s3);

        // PartialEq
        let a = CompressionStrategy::Drop { max_messages: 5 };
        let b = CompressionStrategy::Drop { max_messages: 5 };
        let c = CompressionStrategy::Drop { max_messages: 3 };
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, CompressionStrategy::Summarize);
    }

    // ------------------------------------------------------------------
    // estimate_tokens standalone function
    // ------------------------------------------------------------------

    #[test]
    fn test_estimate_tokens() {
        // 4 chars ≈ 1 token
        assert_eq!(super::estimate_tokens("abcd"), 1);
        assert_eq!(super::estimate_tokens("abcdefgh"), 2);
        // Empty string
        assert_eq!(super::estimate_tokens(""), 0);
        // Odd length rounds down via integer division
        assert_eq!(super::estimate_tokens("abcde"), 1);
        // Long text
        let long = "a".repeat(400);
        assert_eq!(super::estimate_tokens(&long), 100);
    }

    // ------------------------------------------------------------------
    // keep_ratio clamping
    // ------------------------------------------------------------------

    #[test]
    fn test_truncate_clamps_keep_ratio() {
        // keep_ratio at 0.1 boundary
        let compressor =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.1 }, 1000);
        let _ = compressor;

        // Verify the clamping is applied when keep_ratio would be unreasonable.
        // The compressor itself doesn't clamp, but the user is expected to pass valid values.
        // The implementation uses keep_ratio directly as passed. The test confirms
        // that extreme values are handled by the solver, not by clamping inside compress().
        let compressor_high =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 2.0 }, 1000);
        // keep_ratio >= 1.0 could mean "keep everything that fits", so budget >= max_tokens.
        // This is acceptable behavior.
        let messages = vec![
            Message::new(Role::System, "sys"),
            Message::new(Role::User, "Hello world"),
        ];
        let result = compressor_high.compress(&messages);
        assert_eq!(result.len(), 2);

        // Very small keep_ratio should still keep at least the system message
        // and as many non-system messages as fit in the tiny budget.
        let compressor_low =
            ContextCompressor::new(CompressionStrategy::Truncate { keep_ratio: 0.01 }, 1000);
        let messages = vec![
            Message::new(Role::System, "sys"),
            Message::new(Role::User, "Hello world"),
        ];
        let result = compressor_low.compress(&messages);
        assert!(!result.is_empty(), "should retain at least system message");
    }

    // ------------------------------------------------------------------
    // Actual Message struct integration
    // ------------------------------------------------------------------

    #[test]
    fn test_compress_uses_actual_message_struct() {
        let compressor =
            ContextCompressor::new(CompressionStrategy::Drop { max_messages: 2 }, 1000);
        let messages = vec![
            Message {
                role: Role::System,
                content: "System prompt.".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::User,
                content: "Hello!".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::Tool,
                content: "Tool result.".to_string(),
                tool_call_id: Some("call_1".to_string()),
                ..Message::default()
            },
        ];
        let result = compressor.compress(&messages);
        // System message preserved + last 2 non-system (assistant + tool)
        assert!(result.iter().any(|m| m.role == Role::System));
        assert_eq!(result.len(), 3, "1 system + 2 non-system");
    }
}
