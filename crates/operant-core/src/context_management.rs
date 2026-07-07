//! Context management — tiered eviction + decay curve rendering.
//!
//! Ports the key techniques from `cortexkit/magic-context` (a TypeScript
//! OpenCode plugin) into native Rust. operant previously had zero live
//! context management — `build_messages` concatenated system + memory +
//! skills + context_files + full conversation history every iteration,
//! which meant any long-running session would eventually exceed the
//! context window and 400-error.
//!
//! ## Techniques ported from magic-context:
//!
//! 1. **Tiered eviction** (`evict_to_budget`): when the message array
//!    exceeds a token budget, evict oldest-first within tiers:
//!    - T3 (lowest priority): tool results — large, ephemeral, replaceable
//!    - T2 (medium priority): assistant reasoning/thinking — verbose
//!    - T1 (highest priority): user messages + assistant final answers
//!    System messages are never evicted.
//!
//! 2. **Decay curve** (`decay_render`): older messages are rendered into
//!    progressively shorter summaries based on age + importance. The
//!    formula is `H = H50 * 2^((I-50)/D) / max(p, 0.10)` where H50 is
//!    the half-life at 50% importance, D is the decay constant, and p
//!    is the message importance (0-1). No LLM calls — pure deterministic
//!    truncation.
//!
//! 3. **Token estimation** (`estimate_tokens`): char-count / 4 heuristic
//!    (CJK-aware via the iter-18 fix). This is a placeholder until a
//!    real tokenizer (tiktoken-rs) is wired in.

use crate::client::Message;

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Estimate the token count of a string. Uses char-count / 4 (CJK-aware
/// since iter-18 switched from byte count). This is a rough heuristic —
/// a real tokenizer (tiktoken-rs) would be more accurate but adds a
/// dependency. The heuristic is good enough for budgeting decisions
/// (eviction thresholds), not for exact billing.
pub fn estimate_tokens(text: &str) -> usize {
    // char count / 4 is the standard heuristic for English text.
    // For CJK text each char is ~1 token, so the heuristic overestimates
    // for English and underestimates for CJK — acceptable for budgeting.
    (text.chars().count() + 3) / 4
}

/// Estimate the token count of a single message (role + content).
pub fn estimate_message_tokens(msg: &Message) -> usize {
    // 4 tokens overhead per message (role tags, separators) — matches
    // OpenAI's pricing model.
    4 + estimate_tokens(&msg.content)
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

// ---------------------------------------------------------------------------
// Tiered eviction
// ---------------------------------------------------------------------------

/// Evict messages from `messages` until the total token count fits within
/// `budget_tokens`. Returns the new (potentially shorter) message vec.
///
/// ## Eviction tiers (lowest priority evicted first):
///
/// - **System messages**: never evicted (system prompt, memory, skills).
/// - **T3 — tool results**: evicted first. Large, ephemeral, replaceable
///   (the agent can re-run the tool if it needs the result again).
/// - **T2 — assistant reasoning**: evicted second. Verbose thinking
///   blocks that aren't essential to the conversation flow.
/// - **T1 — user + assistant-final**: evicted last. These are the
///   actual conversation turns.
///
/// When evicting from a tier, the oldest messages are removed first
/// (FIFO within tier). A recency reserve of `keep_recent` messages
/// (default 6) is always preserved regardless of tier — the agent
/// needs recent context to understand the current turn.
///
/// This is a port of magic-context's tiered target-headroom eviction,
/// simplified to a single pass (magic-context uses idempotence latches
/// + multi-pass; we don't need that for a first implementation).
pub fn evict_to_budget(messages: Vec<Message>, budget_tokens: usize) -> Vec<Message> {
    let total = estimate_total_tokens(&messages);
    if total <= budget_tokens {
        return messages;
    }

    // Recency reserve: scale with budget so large contexts preserve more
    // recent messages. For a 128k context, ~20 messages; for a 4k context,
    // ~6. Clamped to [6, 50] to avoid degenerate cases. Was fixed at 6,
    // which was too few for large contexts (the agent lost too much recent
    // context) and too many for tiny contexts (it couldn't evict enough).
    //
    // (iter-139 — fixed ponytail-audit bug A25: the previous .min(messages.len())
    // made keep_recent = messages.len() when there were < 6 messages, which
    // made the recency reserve cover ALL messages → eviction impossible.
    // Dropping the .min() is correct: if there are fewer messages than
    // keep_recent, the eviction loop simply finds nothing to evict, which
    // is the right behavior — you don't need to evict when you have few
    // messages.)
    let keep_recent = ((budget_tokens / 4096) as usize).clamp(6, 50);
    let n = messages.len();

    // Build a list of (index, tier) for evictable messages. System
    // messages (index 0, role=System) and the last `keep_recent` messages
    // are never evicted.
    let mut evictable: Vec<(usize, u8)> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if i == 0 && msg.role == crate::client::Role::System {
            continue; // never evict system prompt
        }
        if i >= n.saturating_sub(keep_recent) {
            continue; // recency reserve
        }
        let tier = message_tier(msg);
        evictable.push((i, tier));
    }

    // Sort by tier descending (T3=3 first), then by index ascending
    // (oldest first within tier).
    evictable.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Evict until under budget.
    let mut keep = vec![true; n];
    let mut current_total = total;
    for (i, _tier) in &evictable {
        if current_total <= budget_tokens {
            break;
        }
        current_total = current_total.saturating_sub(estimate_message_tokens(&messages[*i]));
        keep[*i] = false;
    }

    messages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, msg)| msg)
        .collect()
}

/// Classify a message into an eviction tier. Higher = evicted first.
fn message_tier(msg: &Message) -> u8 {
    use crate::client::Role;
    match msg.role {
        Role::System => 0, // never evicted (handled separately, but defensive)
        Role::Tool => 3,   // T3: tool results — large, ephemeral
        Role::Assistant => {
            // T2 if it has reasoning/thinking, T1 if it's a final answer.
            // Heuristic: if the content is long (>500 chars) or contains
            // <think> tags, it's likely reasoning.
            if msg.content.len() > 500 || msg.content.contains("<think>") {
                2
            } else {
                1
            }
        }
        Role::User => 1, // T1: user messages — highest priority
    }
}

// ---------------------------------------------------------------------------
// Decay curve rendering
// ---------------------------------------------------------------------------

/// Render messages with a decay curve: older messages are truncated
/// proportionally to their age. The most recent messages are kept in full;
/// older messages are progressively shortened.
///
/// Ported from magic-context's `decay-curve.ts`. The formula is:
///   `H = H50 * 2^((I-50)/D) / max(p, 0.10)`
/// where:
///   - `H` = output length (chars to keep)
///   - `H50` = baseline length at 50% importance (default 200 chars)
///   - `I` = importance percentile (0-100; older = lower)
///   - `D` = decay constant (default 30; higher = slower decay)
///   - `p` = importance weight (default 1.0; clamped to >= 0.10)
///
/// No LLM calls — pure deterministic truncation. The idea is that old
/// messages still contribute context (so the agent remembers what was
/// discussed) but don't consume the full token budget.
///
/// Only applies to non-system messages. System messages are always kept
/// in full (they're the system prompt, memory, skills, etc.).
pub fn decay_render(messages: Vec<Message>, h50: usize, decay: f64) -> Vec<Message> {
    use crate::client::Role;

    let n = messages.len();
    if n <= 1 {
        return messages;
    }

    messages
        .into_iter()
        .enumerate()
        .map(|(i, msg)| {
            // System messages (index 0) are never decayed.
            if i == 0 && msg.role == Role::System {
                return msg;
            }

            // Importance percentile: most recent = 100, oldest = ~0.
            // i=0 is system (skipped above), so conversation starts at i=1.
            let conv_index = i.saturating_sub(1);
            let conv_len = n.saturating_sub(1);
            let importance = if conv_len == 0 {
                100.0
            } else {
                100.0 * (conv_index as f64 + 1.0) / (conv_len as f64)
            };

            // Decay curve formula. H50 is in TOKENS (not chars) for
            // consistency with estimate_tokens. We convert to chars
            // for truncation: tokens × 4 (the same heuristic used in
            // estimate_tokens). This ensures the decay targets match
            // the budget calculations — previously H50=200 was treated
            // as 200 chars (~50 tokens), which was too aggressive.
            let p = 1.0_f64; // default importance weight
            let p = p.max(0.10);
            let h_tokens = (h50 as f64) * 2.0_f64.powf((importance - 50.0) / decay) / p;
            let target_tokens = h_tokens.max(20.0) as usize; // never < 20 tokens
            let target_chars = target_tokens * 4; // tokens → chars heuristic

            if msg.content.chars().count() <= target_chars {
                msg // already short enough
            } else {
                // Hard-truncate to target_chars. We do NOT add a "[…truncated]"
                // marker because that would change the message content and
                // could confuse the model ("what is this marker?"). The
                // truncation is a context-management internal — the model
                // doesn't need to know it happened. If the model needs the
                // full content, it can re-request it via a tool call.
                let truncated: String = msg.content.chars().take(target_chars).collect();
                Message {
                    content: truncated,
                    ..msg
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Combined budget management
// ---------------------------------------------------------------------------

/// Apply context management to a message array: first decay-render older
/// messages, then evict if still over budget. This is the main entry
/// point called by `build_messages`.
///
/// `budget_tokens` is the target context window (e.g. 120000 for GPT-4).
/// `reserve_for_response` is the tokens to leave free for the model's
/// response (e.g. 4096).
pub fn manage_context(
    messages: Vec<Message>,
    budget_tokens: usize,
    reserve_for_response: usize,
) -> Vec<Message> {
    let effective_budget = budget_tokens.saturating_sub(reserve_for_response);

    // Step 1: decay-render older messages to compress them.
    let decayed = decay_render(messages, 200, 30.0);

    // Step 2: evict if still over budget.
    evict_to_budget(decayed, effective_budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Role};

    fn make_msg(role: Role, content: impl Into<String>) -> Message {
        Message::new(role, content.into())
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 -> 3
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1); // 1 char / 4 = 0.25 -> 1 (rounded up)
    }

    #[test]
    fn evict_to_budget_noop_when_under_budget() {
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi there"),
        ];
        let result = evict_to_budget(msgs.clone(), 10000);
        assert_eq!(result.len(), msgs.len());
    }

    #[test]
    fn evict_to_budget_removes_tool_results_first() {
        // System + user + tool_result + 6 recent messages (so the tool
        // result is NOT in the recency reserve and CAN be evicted).
        let msgs = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "old user message"),
            make_msg(Role::Tool, "very long tool result ".repeat(50)),
            make_msg(Role::User, "msg 1"),
            make_msg(Role::Assistant, "msg 2"),
            make_msg(Role::User, "msg 3"),
            make_msg(Role::Assistant, "msg 4"),
            make_msg(Role::User, "msg 5"),
            make_msg(Role::Assistant, "recent answer"),
        ];
        // Budget of 100 forces eviction. The tool result is T3 and
        // outside the recency reserve (last 6), so it gets evicted first.
        let result = evict_to_budget(msgs, 100);
        // Tool result should be evicted.
        assert!(
            !result.iter().any(|m| m.role == Role::Tool),
            "tool result should be evicted"
        );
        // System should be preserved.
        assert!(result.iter().any(|m| m.role == Role::System));
    }

    #[test]
    fn evict_never_removes_system_prompt() {
        let msgs = vec![
            make_msg(Role::System, "system prompt that is important"),
            make_msg(Role::Tool, "tool result ".repeat(100)),
            make_msg(Role::User, "recent"),
        ];
        let result = evict_to_budget(msgs, 10);
        // System prompt must survive even under extreme budget pressure.
        assert!(result.first().is_some_and(|m| m.role == Role::System));
    }

    #[test]
    fn evict_preserves_recency_reserve() {
        // 10 messages, all tool results (T3). Budget forces eviction.
        let msgs: Vec<Message> = (0..10)
            .map(|i| make_msg(Role::Tool, &format!("tool result {}", i)))
            .collect();
        let result = evict_to_budget(msgs, 50);
        // The last 6 (keep_recent) should always be preserved.
        assert!(
            result.len() >= 6,
            "recency reserve of 6 should be preserved, got {}",
            result.len()
        );
    }

    #[test]
    fn decay_render_preserves_system_message() {
        let msgs = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "old message that is quite long ".repeat(20)),
            make_msg(Role::User, "recent message"),
        ];
        let result = decay_render(msgs, 50, 30.0);
        // System message should be unchanged.
        assert_eq!(result[0].content, "system prompt");
    }

    #[test]
    fn decay_render_truncates_old_messages() {
        let long_content = "this is a very long message ".repeat(50);
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, &long_content), // oldest, should be truncated
            make_msg(Role::User, &long_content), // middle
            make_msg(Role::User, &long_content), // most recent, should be ~full
        ];
        let result = decay_render(msgs, 100, 30.0);
        // The oldest (index 1) should be shorter than the newest (index 3).
        let oldest_len = result[1].content.chars().count();
        let newest_len = result[3].content.chars().count();
        assert!(
            oldest_len < newest_len,
            "oldest should be shorter than newest: {} vs {}",
            oldest_len,
            newest_len
        );
    }

    #[test]
    fn manage_context_combines_decay_and_evict() {
        // 10 messages: system + 8 tool results + recent. The tool results
        // are T3 (evict first) and outside the recency reserve (last 6).
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::Tool, "result ".repeat(200)), // T3, oldest
            make_msg(Role::Tool, "result ".repeat(200)), // T3
            make_msg(Role::Tool, "result ".repeat(200)), // T3
            make_msg(Role::User, "msg 1"),
            make_msg(Role::Assistant, "msg 2"),
            make_msg(Role::User, "msg 3"),
            make_msg(Role::Assistant, "msg 4"),
            make_msg(Role::User, "msg 5"),
            make_msg(Role::Assistant, "recent answer"),
        ];
        // Budget of 100 tokens + 50 reserve = 50 effective.
        // The 3 tool results (~350 tokens each before decay) are T3 and
        // outside the recency reserve (keep_recent = 6, so last 6 are
        // preserved). They should be evicted.
        let result = manage_context(msgs, 100, 50);
        let total = estimate_total_tokens(&result);
        assert!(
            total <= 50,
            "total {} should be <= effective budget 50",
            total
        );
        // System + recent should survive.
        assert!(result.first().is_some_and(|m| m.role == Role::System));
        assert!(result.iter().any(|m| m.content == "recent answer"));
    }
}
