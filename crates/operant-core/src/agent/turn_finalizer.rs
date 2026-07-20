//! Post-loop turn finalization for `OperantAgent::run()`.
//!
//! Extracted from the agent's main loop as part of the self-evolution pipeline.
//! After each turn completes (successfully or via budget exhaustion), the
//! finalizer checks whether a skill review should be triggered and spawns
//! a background review daemon if so.
//!
//! Ported from `hermes-agent/agent/turn_finalizer.py`.




/// Result of the skill nudge check after a turn completes.
#[derive(Debug, Clone)]
pub struct TurnFinalizeResult {
    /// Whether a skill review should be triggered.
    pub should_review_skills: bool,
    /// Whether a memory review should be triggered.
    pub should_review_memory: bool,
    /// The number of tool iterations this turn used.
    pub iteration_count: usize,
    /// The tool names that were called this turn.
    pub tools_called: Vec<String>,
    /// Whether skill_manage was called this turn.
    pub skill_manage_called: bool,
}

/// Check whether a background review should be triggered after a turn.
///
/// This mirrors hermes-agent's turn_finalizer.py logic:
/// - Track `_iters_since_skill` (incremented each iteration, reset when
///   `skill_manage` is called).
/// - When `_iters_since_skill >= _skill_nudge_interval`, trigger a skill
///   review.
/// - Memory review is triggered by a separate cadence (every N turns).
pub fn should_trigger_background_review(
    iters_since_skill: usize,
    skill_nudge_interval: usize,
    skill_manage_called: bool,
    tools_called: &[String],
) -> TurnFinalizeResult {
    let should_review_skills = if skill_nudge_interval > 0 && skill_manage_called {
        // skill_manage was called — reset the counter and don't nudge
        false
    } else if skill_nudge_interval > 0 && iters_since_skill >= skill_nudge_interval {
        // Enough iterations have passed — trigger a skill review
        true
    } else {
        false
    };

    // Memory review is always considered if the turn had substantive tool use
    // (matching hermes-agent's `_should_review_memory` logic)
    let has_substantive_tools = tools_called.iter().any(|t| {
        t != "memory"
            && t != "todo"
            && t != "skill_manage"
            && t != "session_search"
            && t != "skills_list"
            && t != "skill_view"
    });
    let should_review_memory = has_substantive_tools && tools_called.len() >= 2;

    TurnFinalizeResult {
        should_review_skills,
        should_review_memory,
        iteration_count: tools_called.len(),
        tools_called: tools_called.to_vec(),
        skill_manage_called,
    }
}

/// Summary of actions taken by the background review.
#[derive(Debug, Clone, Default)]
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
        // Parse as JSON value for flexibility
        let data: serde_json::Value = match serde_json::from_str(msg_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Only look at tool-role messages
        if data.get("role").and_then(|v| v.as_str()) != Some("tool") {
            continue;
        }

        // Skip messages that were already in the prior snapshot
        if let Some(id) = data.get("tool_call_id").and_then(|v| v.as_str()) {
            if prior_tool_ids.contains(id) {
                continue;
            }
        }

        // Parse the tool result content
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

        // Detect skill vs memory actions
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
    fn test_skill_manage_resets_counter() {
        let result = should_trigger_background_review(
            15,  // iters_since_skill
            10,  // skill_nudge_interval
            true, // skill_manage_called
            &["skill_manage".to_string()],
        );
        assert!(!result.should_review_skills);
        assert!(result.skill_manage_called);
    }

    #[test]
    fn test_nudge_fires_above_threshold() {
        let result = should_trigger_background_review(
            12,  // iters_since_skill >= 10
            10,  // skill_nudge_interval
            false, // skill_manage_called
            &["web_search".to_string(), "read_file".to_string()],
        );
        assert!(result.should_review_skills);
        assert!(result.should_review_memory); // has substantive tools
    }

    #[test]
    fn test_nudge_does_not_fire_below_threshold() {
        let result = should_trigger_background_review(
            5,   // iters_since_skill < 10
            10,  // skill_nudge_interval
            false,
            &["web_search".to_string()],
        );
        assert!(!result.should_review_skills);
    }

    #[test]
    fn test_nudge_disabled_when_interval_zero() {
        let result = should_trigger_background_review(
            100,
            0,   // skill_nudge_interval = 0 means disabled
            false,
            &["web_search".to_string()],
        );
        assert!(!result.should_review_skills);
    }

    #[test]
    fn test_memory_review_requires_substantive_tools() {
        let result = should_trigger_background_review(
            5,
            10,
            false,
            &["memory".to_string(), "todo".to_string()],
        );
        assert!(!result.should_review_memory); // only helper tools
    }
}
