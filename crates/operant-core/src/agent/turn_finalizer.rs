//! Post-loop turn finalization for `OperantAgent::run()`.
//!
//! Extracted from the agent's main loop as part of the self-evolution pipeline.
//! After each turn completes (successfully or via budget exhaustion), the
//! finalizer checks whether a skill review should be triggered and spawns
//! a background review daemon if so.
//!
//! Ported from `hermes-agent/agent/turn_finalizer.py`.
//!
//! The core turn-finalization logic is handled inline in `OperantAgent::run()`
//! via `SelfEvolutionState`. This module provides additional types for
//! background review result summarization (used when the forked-agent
//! background review daemon is wired up).

/// Summary of actions taken by the background review.
///
/// Used by the background review daemon (once wired up) to surface a compact
/// summary of skill/memory changes to the user.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct BackgroundReviewSummary {
    /// Human-readable action descriptions.
    pub actions: Vec<String>,
    /// Whether any skills were created/updated.
    pub skills_changed: bool,
    /// Whether any memory entries were added/updated.
    pub memory_changed: bool,
}

/// Build a compact action summary from background review messages.
///
/// Scans the review agent's messages for successful tool actions and
/// surfaces a compact summary to the user. Matches hermes-agent's
/// `summarize_background_review_actions`.
#[allow(dead_code)]
pub fn summarize_review_actions(
    review_messages: &[String],
    prior_messages: &[String],
) -> BackgroundReviewSummary {
    let mut summary = BackgroundReviewSummary::default();

    // Collect existing tool call IDs from prior messages to avoid re-surfacing
    let prior_tool_ids: std::collections::HashSet<String> = prior_messages
        .iter()
        .filter_map(|m| {
            let data: serde_json::Value = serde_json::from_str(m).ok()?;
            if data.get("role").and_then(|v| v.as_str()) == Some("tool") {
                data.get("tool_call_id").and_then(|v| v.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect();

    for msg_str in review_messages {
        let data: serde_json::Value = match serde_json::from_str(msg_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if data.get("role").and_then(|v| v.as_str()) != Some("tool") {
            continue;
        }

        if let Some(id) = data.get("tool_call_id").and_then(|v| v.as_str()) {
            if prior_tool_ids.contains(id) {
                continue;
            }
        }

        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let result: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
        if !result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }

        let message = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if message.is_empty() {
            continue;
        }

        let is_skill = message.to_lowercase().contains("skill");
        if is_skill {
            summary.skills_changed = true;
        } else {
            summary.memory_changed = true;
        }

        summary.actions.push(message);
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarize_review_actions_empty() {
        let summary = summarize_review_actions(&[], &[]);
        assert!(summary.actions.is_empty());
        assert!(!summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_skill() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Updated skill web-search\"}"}"#.to_string()];
        let summary = summarize_review_actions(&review, &[]);
        assert_eq!(summary.actions.len(), 1);
        assert!(summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_skips_prior() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Saved memory\"}"}"#.to_string()];
        let prior = vec![r#"{"role":"tool","tool_call_id":"tc1"}"#.to_string()];
        let summary = summarize_review_actions(&review, &prior);
        assert!(summary.actions.is_empty());
    }
}
