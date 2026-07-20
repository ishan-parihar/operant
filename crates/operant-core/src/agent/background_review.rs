//! Background memory/skill review — fork the agent to evaluate the turn.
//!
//! After every turn, `OperantAgent::run()` may trigger a background review
//! that evaluates whether any skill or memory should be saved or updated.
//! Writes go straight to the memory + skill stores. Main conversation and
//! prompt cache are never touched.
//!
//! Ported from `hermes-agent/agent/background_review.py`.

use tracing::debug;

/// Review prompt for skill updates. This is the message sent to the
/// review agent fork when a skill review is triggered.
pub const SKILL_REVIEW_PROMPT: &str = "\
Review the conversation above and update the skill library. Be \
ACTIVE — most sessions produce at least one skill update, even if \
small. A pass that does nothing is a missed learning opportunity, \
not a neutral outcome.

Target shape of the library: CLASS-LEVEL skills, each with a rich \
SKILL.md and a `references/` directory for session-specific detail. \
Not a long flat list of narrow one-session-one-skill entries.

Signals to look for (any one of these warrants action):
  • User corrected your style, tone, format, or verbosity.
  • User corrected your workflow, approach, or sequence of steps.
  • Non-trivial technique, fix, workaround, or debugging path emerged.
  • A skill that got loaded or consulted turned out wrong or outdated.

Preference order — prefer the earliest action that fits:
  1. UPDATE A CURRENTLY-LOADED SKILL.
  2. UPDATE AN EXISTING UMBRELLA SKILL.
  3. ADD A SUPPORT FILE under an existing umbrella.
  4. CREATE A NEW CLASS-LEVEL UMBRELLA SKILL.

If nothing needs updating, say 'Nothing to save.' and stop.";

/// Review prompt for memory updates.
pub const MEMORY_REVIEW_PROMPT: &str = "\
Review the conversation above and consider saving to memory if appropriate.

Focus on:
1. Has the user revealed things about themselves — their persona, desires, \
preferences, or personal details worth remembering?
2. Has the user expressed expectations about how you should behave?

If something stands out, save it using the memory tool. \
If nothing is worth saving, just say 'Nothing to save.' and stop.";

/// Combined review prompt for both memory and skill updates.
pub const COMBINED_REVIEW_PROMPT: &str = "\
Review the conversation above and update two things:

**Memory**: who the user is. Did the user reveal persona, preferences, \
or expectations? Save durable preferences with the memory tool.

**Skills**: how to do this class of task. Be ACTIVE — most sessions produce \
at least one skill update. Signals that warrant a skill update:
  • User corrected your style, tone, format, or approach.
  • Non-trivial technique, fix, or debugging path emerged.
  • A loaded skill turned out wrong or outdated.

If genuinely nothing stands out on either, say 'Nothing to save.' and stop.";

/// Build the review prompt based on which triggers fired.
///
/// Returns the appropriate prompt string for the background review agent.
pub fn build_review_prompt(review_memory: bool, review_skills: bool) -> String {
    if review_memory && review_skills {
        COMBINED_REVIEW_PROMPT.to_string()
    } else if review_memory {
        MEMORY_REVIEW_PROMPT.to_string()
    } else {
        SKILL_REVIEW_PROMPT.to_string()
    }
}

/// Configuration for the background review daemon.
///
/// Used to construct `SelfEvolutionState` and configure the review agent.
#[derive(Debug, Clone)]
pub struct BackgroundReviewConfig {
    /// Skill nudge interval (default: 10).
    pub skill_nudge_interval: usize,
    /// Memory review interval in turns (default: 5).
    pub memory_review_interval: usize,
}

impl Default for BackgroundReviewConfig {
    fn default() -> Self {
        Self {
            skill_nudge_interval: 10,
            memory_review_interval: 5,
        }
    }
}

/// State tracking for the self-evolution pipeline.
///
/// Tracks iteration counts and nudge thresholds across turns.
/// Used by `OperantAgent::run()` to determine when skill/memory
/// reviews should be triggered.
pub struct SelfEvolutionState {
    /// Number of iterations since the last skill_manage call.
    pub iters_since_skill: usize,
    /// How many iterations between skill nudges (0 = disabled).
    pub skill_nudge_interval: usize,
    /// Number of turns since the last memory review.
    pub turns_since_memory_review: usize,
    /// How many turns between memory reviews (0 = disabled).
    pub memory_review_interval: usize,
}

impl SelfEvolutionState {
    /// Create a new state with the given configuration.
    pub fn new(config: &BackgroundReviewConfig) -> Self {
        Self {
            iters_since_skill: 0,
            skill_nudge_interval: config.skill_nudge_interval,
            turns_since_memory_review: 0,
            memory_review_interval: config.memory_review_interval,
        }
    }

    /// Increment the skill iteration counter (called each agent iteration).
    pub fn bump_skill_counter(&mut self) {
        self.iters_since_skill += 1;
    }

    /// Reset the skill iteration counter (called when skill_manage is used).
    pub fn reset_skill_counter(&mut self) {
        self.iters_since_skill = 0;
    }

    /// Check if a skill review should be triggered.
    pub fn should_review_skills(&self) -> bool {
        self.skill_nudge_interval > 0 && self.iters_since_skill >= self.skill_nudge_interval
    }

    /// Increment the memory turn counter (called each completed turn).
    pub fn bump_memory_counter(&mut self) {
        self.turns_since_memory_review += 1;
    }

    /// Reset the memory turn counter (called after a memory review fires).
    pub fn reset_memory_counter(&mut self) {
        self.turns_since_memory_review = 0;
    }

    /// Check if a memory review should be triggered.
    pub fn should_review_memory(&self) -> bool {
        self.memory_review_interval > 0 && self.turns_since_memory_review >= self.memory_review_interval
    }

    // ── Hydration / Persistence (Phase 4) ───────────────────────────
    // When a session is resumed via persistent_session_id, the in-memory
    // counters start at 0. Hydrate them from session_metadata so the
    // review cadence continues where it left off.

    /// Hydrate counters from a metadata map (loaded from session_metadata).
    ///
    /// Keys: `evo_turns_since_memory`, `evo_iters_since_skill`.
    /// Missing keys are treated as 0 (first run of a session).
    pub fn hydrate_from_metadata(&mut self, metadata: &std::collections::HashMap<String, String>) {
        if let Some(val) = metadata.get("evo_turns_since_memory") {
            if let Ok(n) = val.parse::<usize>() {
                self.turns_since_memory_review = n;
                debug!(
                    turns = n,
                    "Hydrated memory review counter from persisted session"
                );
            }
        }
        if let Some(val) = metadata.get("evo_iters_since_skill") {
            if let Ok(n) = val.parse::<usize>() {
                self.iters_since_skill = n;
                debug!(
                    iters = n,
                    "Hydrated skill nudge counter from persisted session"
                );
            }
        }
    }

    /// Serialize current counters to a key-value map suitable for
    /// `Database::set_session_metadata`. Call after each turn to persist
    /// the counters so they survive session restarts.
    pub fn persist_counters(&self) -> Vec<(&'static str, String)> {
        vec![
            ("evo_turns_since_memory", self.turns_since_memory_review.to_string()),
            ("evo_iters_since_skill", self.iters_since_skill.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_combined() {
        let prompt = build_review_prompt(true, true);
        assert!(prompt.contains("Memory"));
        assert!(prompt.contains("Skills"));
    }

    #[test]
    fn test_build_prompt_skills_only() {
        let prompt = build_review_prompt(false, true);
        assert!(prompt.contains("skill library"));
        assert!(!prompt.contains("**Memory**"));
    }

    #[test]
    fn test_build_prompt_memory_only() {
        let prompt = build_review_prompt(true, false);
        assert!(prompt.contains("saving to memory"));
    }

    #[test]
    fn test_self_evolution_skill_nudge() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..4 {
            state.bump_skill_counter();
        }
        assert!(!state.should_review_skills());

        state.bump_skill_counter();
        assert!(state.should_review_skills());
    }

    #[test]
    fn test_self_evolution_skill_manage_resets() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            ..Default::default()
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..10 {
            state.bump_skill_counter();
        }
        assert!(state.should_review_skills());

        state.reset_skill_counter();
        assert!(!state.should_review_skills());
        assert_eq!(state.iters_since_skill, 0);
    }

    #[test]
    fn test_self_evolution_disabled_when_zero() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 0,
            memory_review_interval: 0,
        };
        let state = SelfEvolutionState::new(&config);
        assert!(!state.should_review_skills());
        assert!(!state.should_review_memory());
    }

    // ── Memory review counter tests ────────────────────────────────────

    #[test]
    fn test_memory_review_counter_bump_and_trigger() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        state.bump_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(state.should_review_memory());
    }

    #[test]
    fn test_memory_review_counter_reset() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..3 {
            state.bump_memory_counter();
        }
        assert!(state.should_review_memory());

        state.reset_memory_counter();
        assert!(!state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 0);
    }

    #[test]
    fn test_memory_review_disabled_when_zero() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 0,
        };
        let state = SelfEvolutionState::new(&config);
        assert!(!state.should_review_memory());
    }

    #[test]
    fn test_memory_review_full_cycle() {
        // Simulates: bump 2 → trigger at 3 → reset → bump 1.
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        state.bump_memory_counter();
        state.bump_memory_counter();
        assert!(!state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 2);

        state.bump_memory_counter();
        assert!(state.should_review_memory());

        state.reset_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(!state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 1);
    }

    // ── Integration tests for the self-evolution pipeline ─────────────
    // These simulate the full loop pattern used in OperantAgent::run().

    #[test]
    fn test_full_cycle_bump_then_skill_manage_resets() {
        // Simulates: bump 3 times → skill_manage called → bump 2 more.
        // After skill_manage, the counter resets, so 2 bumps shouldn't
        // trigger a review at threshold=5.
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Bump 3 times
        state.bump_skill_counter();
        state.bump_skill_counter();
        state.bump_skill_counter();
        assert!(!state.should_review_skills());
        assert_eq!(state.iters_since_skill, 3);

        // skill_manage called — resets counter
        state.reset_skill_counter();
        assert_eq!(state.iters_since_skill, 0);
        assert!(!state.should_review_skills());

        // Bump 2 more — still below threshold
        state.bump_skill_counter();
        state.bump_skill_counter();
        assert!(!state.should_review_skills());
        assert_eq!(state.iters_since_skill, 2);
    }

    #[test]
    fn test_full_cycle_nudge_triggers_then_resets() {
        // Simulates: bump to threshold → trigger → reset → bump again.
        // After reset, the next cycle starts fresh.
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 3,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Bump to threshold
        state.bump_skill_counter();
        state.bump_skill_counter();
        assert!(!state.should_review_skills());

        state.bump_skill_counter();
        assert!(state.should_review_skills());

        // Reset after trigger
        state.reset_skill_counter();
        assert!(!state.should_review_skills());
        assert_eq!(state.iters_since_skill, 0);

        // Second cycle: bump 2 more — not enough to trigger
        state.bump_skill_counter();
        state.bump_skill_counter();
        assert!(!state.should_review_skills());

        // One more — triggers again
        state.bump_skill_counter();
        assert!(state.should_review_skills());
    }

    #[test]
    fn test_skill_manage_at_exactly_threshold() {
        // If skill_manage is called at exactly the threshold, the counter
        // resets and no review is triggered.
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..5 {
            state.bump_skill_counter();
        }
        assert!(state.should_review_skills());

        // skill_manage resets BEFORE the check in the agent loop
        state.reset_skill_counter();
        assert!(!state.should_review_skills());
    }

    #[test]
    fn test_interval_persists_after_reset() {
        // Verify that the skill nudge interval is independent of config changes.
        let config1 = BackgroundReviewConfig {
            skill_nudge_interval: 3,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config1);

        // Bump 2 — not enough
        state.bump_skill_counter();
        state.bump_skill_counter();
        assert!(!state.should_review_skills());

        // Bump 1 more — triggers
        state.bump_skill_counter();
        assert!(state.should_review_skills());

        // Reset and verify the interval is still 3
        state.reset_skill_counter();
        assert_eq!(state.iters_since_skill, 0);
        assert!(!state.should_review_skills());
    }

    // ── Hydration / Persistence tests (Phase 4) ─────────────────────

    #[test]
    fn test_hydrate_from_metadata_restores_counters() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Simulate a session that had 4 memory turns and 2 skill iters
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("evo_turns_since_memory".to_string(), "4".to_string());
        metadata.insert("evo_iters_since_skill".to_string(), "2".to_string());

        state.hydrate_from_metadata(&metadata);

        assert_eq!(state.turns_since_memory_review, 4);
        assert_eq!(state.iters_since_skill, 2);
        // Memory review should trigger (4 >= 3)
        assert!(state.should_review_memory());
        // Skill review should NOT trigger (2 < 5)
        assert!(!state.should_review_skills());
    }

    #[test]
    fn test_hydrate_from_metadata_handles_missing_keys() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Empty metadata — counters stay at 0 (fresh session)
        let metadata = std::collections::HashMap::new();
        state.hydrate_from_metadata(&metadata);

        assert_eq!(state.turns_since_memory_review, 0);
        assert_eq!(state.iters_since_skill, 0);
        assert!(!state.should_review_memory());
        assert!(!state.should_review_skills());
    }

    #[test]
    fn test_hydrate_from_metadata_handles_invalid_values() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("evo_turns_since_memory".to_string(), "not_a_number".to_string());
        metadata.insert("evo_iters_since_skill".to_string(), "".to_string());

        // Invalid values are silently ignored; counters stay at 0
        state.hydrate_from_metadata(&metadata);
        assert_eq!(state.turns_since_memory_review, 0);
        assert_eq!(state.iters_since_skill, 0);
    }

    #[test]
    fn test_persist_counters_roundtrip() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Bump counters
        state.bump_memory_counter();
        state.bump_memory_counter();
        state.bump_skill_counter();

        // Persist
        let pairs = state.persist_counters();
        assert_eq!(pairs.len(), 2);

        // Convert to HashMap (simulates what Database would store/retrieve)
        let metadata: std::collections::HashMap<String, String> = pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();

        // Hydrate into a fresh state
        let mut state2 = SelfEvolutionState::new(&config);
        state2.hydrate_from_metadata(&metadata);

        assert_eq!(state2.turns_since_memory_review, 2);
        assert_eq!(state2.iters_since_skill, 1);
    }

    #[test]
    fn test_hydrate_then_bump_continues_correctly() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 3,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Hydrate at 2 turns (one away from memory review)
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("evo_turns_since_memory".to_string(), "2".to_string());
        state.hydrate_from_metadata(&metadata);

        // One more bump triggers memory review
        state.bump_memory_counter();
        assert!(state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 3);

        // Reset and continue
        state.reset_memory_counter();
        assert!(!state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 0);
    }
}
