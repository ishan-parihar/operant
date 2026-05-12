//! 3-layer tool result persistence.
//!
//! Defends against context-window overflow by managing tool results at
//! three levels:
//!
//! 1. **Preview** — ring buffer of the last N results per tool.
//! 2. **Per-result** — full results keyed by tool name.
//! 3. **Turn budget** — per-tool call count and token tracking within a turn.

use std::collections::{HashMap, VecDeque};

use crate::tools::ToolResult;

/// Configuration for the preview ring buffer.
#[derive(Debug, Clone)]
pub struct PreviewConfig {
    /// Maximum number of preview entries per tool.
    pub max_previews: usize,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self { max_previews: 5 }
    }
}

/// A per-tool entry in the preview ring buffer.
#[derive(Debug, Clone)]
struct PreviewEntry {
    tool_name: String,
    result: ToolResult,
}

/// 3-layer tool result storage.
///
/// Manages tool results across three layers:
///
/// - **Layer 1 (Preview)**: A ring buffer storing the last `max_previews`
///   results for each tool, for quick display purposes.
/// - **Layer 2 (Per-result)**: A `HashMap<String, Vec<ToolResult>>` that
///   stores every result keyed by tool name — useful for full access.
/// - **Layer 3 (Turn budget)**: A `HashMap<String, (usize, u64)>` that
///   tracks the number of calls and total tokens consumed per tool within
///   the current turn.
#[derive(Debug, Clone)]
pub struct ToolResultStorage {
    /// Layer 1: Ring buffer of recent results per tool.
    preview: VecDeque<PreviewEntry>,
    /// Layer 2: All results keyed by tool name.
    per_result: HashMap<String, Vec<ToolResult>>,
    /// Layer 3: Per-tool (call_count, total_tokens) for the current turn.
    turn_budget: HashMap<String, (usize, u64)>,
    /// Preview configuration.
    config: PreviewConfig,
}

impl ToolResultStorage {
    /// Create a new `ToolResultStorage` with the given preview configuration.
    pub fn new(config: PreviewConfig) -> Self {
        Self {
            preview: VecDeque::with_capacity(config.max_previews + 1),
            per_result: HashMap::new(),
            turn_budget: HashMap::new(),
            config,
        }
    }

    /// Store a tool result in all applicable layers.
    ///
    /// The result is:
    /// - Added to the preview ring buffer (trimmed to `max_previews`).
    /// - Appended to the per-tool result list.
    ///
    /// Budget tracking is updated separately via [`track_budget`].
    pub fn store_result(&mut self, tool_name: &str, result: ToolResult) {
        // Layer 1: preview ring buffer.
        self.preview.push_back(PreviewEntry {
            tool_name: tool_name.to_string(),
            result: result.clone(),
        });
        while self.preview.len() > self.config.max_previews {
            self.preview.pop_front();
        }

        // Layer 2: per-result storage.
        self.per_result
            .entry(tool_name.to_string())
            .or_default()
            .push(result);
    }

    /// Return the last N results for a tool from the preview buffer.
    pub fn get_preview(&self, tool_name: &str) -> Option<Vec<&ToolResult>> {
        let results: Vec<&ToolResult> = self
            .preview
            .iter()
            .filter(|e| e.tool_name == tool_name)
            .map(|e| &e.result)
            .collect();
        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    /// Return all stored results for a tool.
    pub fn get_all_for_tool(&self, tool_name: &str) -> Vec<&ToolResult> {
        self.per_result
            .get(tool_name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Increment the call count and token total for a tool in the current
    /// turn.
    pub fn track_budget(&mut self, tool_name: &str, token_count: u64) {
        let entry = self
            .turn_budget
            .entry(tool_name.to_string())
            .or_insert((0, 0));
        entry.0 += 1;
        entry.1 += token_count;
    }

    /// Return the current turn budget usage for a tool, if any.
    ///
    /// Returns `Some((calls, tokens))` or `None` if the tool has not been
    /// tracked this turn.
    pub fn get_budget_usage(&self, tool_name: &str) -> Option<(u32, u64)> {
        self.turn_budget
            .get(tool_name)
            .map(|(calls, tokens)| (*calls as u32, *tokens))
    }

    /// Reset the turn-level state — clears the turn budget and preview
    /// buffer.  Per-result (layer 2) data is preserved for the session.
    pub fn reset_turn(&mut self) {
        self.turn_budget.clear();
        self.preview.clear();
    }

    /// Total tokens consumed by all tools in the current turn.
    pub fn total_tokens_this_turn(&self) -> u64 {
        self.turn_budget.values().map(|(_, tokens)| tokens).sum()
    }

    /// Total tool calls made in the current turn.
    pub fn total_calls_this_turn(&self) -> u32 {
        self.turn_budget
            .values()
            .map(|(calls, _)| *calls as u32)
            .sum()
    }

    /// Check whether the current turn's budget exceeds the given limits.
    ///
    /// Returns `true` if the total tokens OR total calls this turn exceed
    /// the corresponding limit in `config`.
    pub fn is_budget_exceeded(&self, config: &crate::budget_config::BudgetConfig) -> bool {
        if self.total_tokens_this_turn() > config.max_tokens_per_turn {
            return true;
        }
        if self.total_calls_this_turn() > config.max_calls_per_turn {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolResult;
    use crate::budget_config::BudgetConfig;

    fn make_result(tool_call_id: &str, content: &str) -> ToolResult {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            success: true,
            content: content.to_string(),
            error: None,
        }
    }

    #[test]
    fn test_new_storage_empty() {
        let storage = ToolResultStorage::new(PreviewConfig::default());
        assert_eq!(storage.total_calls_this_turn(), 0);
        assert_eq!(storage.total_tokens_this_turn(), 0);
    }

    #[test]
    fn test_store_and_get_preview() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        let r = make_result("call_1", "result_1");
        storage.store_result("test_tool", r);

        let preview = storage.get_preview("test_tool");
        assert!(preview.is_some());
        assert_eq!(preview.unwrap().len(), 1);
    }

    #[test]
    fn test_get_preview_nonexistent_tool() {
        let storage = ToolResultStorage::new(PreviewConfig::default());
        assert!(storage.get_preview("nonexistent").is_none());
    }

    #[test]
    fn test_get_all_for_tool() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.store_result("tool_a", make_result("call_1", "a1"));
        storage.store_result("tool_a", make_result("call_2", "a2"));
        storage.store_result("tool_b", make_result("call_3", "b1"));

        let all_a = storage.get_all_for_tool("tool_a");
        assert_eq!(all_a.len(), 2);

        let all_b = storage.get_all_for_tool("tool_b");
        assert_eq!(all_b.len(), 1);

        let all_c = storage.get_all_for_tool("tool_c");
        assert!(all_c.is_empty());
    }

    #[test]
    fn test_preview_ring_buffer_capacity() {
        let config = PreviewConfig { max_previews: 3 };
        let mut storage = ToolResultStorage::new(config);
        for i in 0..10 {
            storage.store_result("tool", make_result(&format!("call_{}", i), &format!("r{}", i)));
        }
        let preview = storage.get_preview("tool").unwrap();
        // Only the last 3 should survive.
        assert_eq!(preview.len(), 3);
        assert_eq!(preview[0].content, "r7");
        assert_eq!(preview[2].content, "r9");
    }

    #[test]
    fn test_track_budget() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.track_budget("tool_a", 100);
        storage.track_budget("tool_a", 200);
        storage.track_budget("tool_b", 50);

        let a_usage = storage.get_budget_usage("tool_a").unwrap();
        assert_eq!(a_usage, (2, 300));

        let b_usage = storage.get_budget_usage("tool_b").unwrap();
        assert_eq!(b_usage, (1, 50));
    }

    #[test]
    fn test_get_budget_usage_nonexistent() {
        let storage = ToolResultStorage::new(PreviewConfig::default());
        assert!(storage.get_budget_usage("nonexistent").is_none());
    }

    #[test]
    fn test_reset_turn() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.store_result("tool", make_result("call_1", "data"));
        storage.track_budget("tool", 500);

        assert!(storage.get_preview("tool").is_some());
        assert!(storage.get_budget_usage("tool").is_some());

        storage.reset_turn();

        assert!(storage.get_preview("tool").is_none());
        assert!(storage.get_budget_usage("tool").is_none());
        // Per-result data survives reset.
        assert_eq!(storage.get_all_for_tool("tool").len(), 1);
    }

    #[test]
    fn test_total_tokens_and_calls() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.track_budget("a", 100);
        storage.track_budget("a", 200);
        storage.track_budget("b", 300);

        assert_eq!(storage.total_tokens_this_turn(), 600);
        assert_eq!(storage.total_calls_this_turn(), 3);
    }

    #[test]
    fn test_is_budget_exceeded_tokens() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.track_budget("tool", 150_000);

        let config = BudgetConfig::default()
            .with_max_tokens_per_turn(100_000);
        assert!(storage.is_budget_exceeded(&config));
    }

    #[test]
    fn test_is_budget_exceeded_calls() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.track_budget("tool", 10);
        storage.track_budget("tool", 10);

        let config = BudgetConfig::default()
            .with_max_calls_per_turn(1);
        assert!(storage.is_budget_exceeded(&config));
    }

    #[test]
    fn test_is_budget_not_exceeded() {
        let mut storage = ToolResultStorage::new(PreviewConfig::default());
        storage.track_budget("tool", 100);

        let config = BudgetConfig::default()
            .with_max_tokens_per_turn(200_000)
            .with_max_calls_per_turn(50);
        assert!(!storage.is_budget_exceeded(&config));
    }

    #[test]
    fn test_store_result_maintains_tool_order() {
        let mut storage = ToolResultStorage::new(PreviewConfig { max_previews: 10 });
        storage.store_result("tool", make_result("c1", "first"));
        storage.store_result("tool", make_result("c2", "second"));

        let all = storage.get_all_for_tool("tool");
        assert_eq!(all[0].tool_call_id, "c1");
        assert_eq!(all[1].tool_call_id, "c2");

        let preview = storage.get_preview("tool").unwrap();
        assert_eq!(preview[0].tool_call_id, "c1");
        assert_eq!(preview[1].tool_call_id, "c2");
    }
}
