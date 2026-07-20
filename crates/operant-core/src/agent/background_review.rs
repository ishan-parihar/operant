//! Background memory/skill review — fork the agent to evaluate the turn.
//!
//! After every turn, `OperantAgent::run()` may trigger a background review
//! that evaluates whether any skill or memory should be saved or updated.
//! Writes go straight to the memory + skill stores. Main conversation and
//! prompt cache are never touched.
//!
//! Ported from `hermes-agent/agent/background_review.py`.



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

/// Result of a background review spawn attempt.
#[derive(Debug, Clone)]
pub struct BackgroundReviewResult {
    /// Whether the review was spawned successfully.
    pub spawned: bool,
    /// The prompt that was sent to the review agent.
    pub prompt: String,
    /// Whether skill review was included.
    pub review_skills: bool,
    /// Whether memory review was included.
    pub review_memory: bool,
}

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
#[derive(Debug, Clone)]
pub struct BackgroundReviewConfig {
    /// Maximum iterations for the review agent (default: 16).
    pub max_iterations: usize,
    /// Model to use for the review (None = inherit parent).
    pub model: Option<String>,
    /// Skill nudge interval (default: 10).
    pub skill_nudge_interval: usize,
    /// Memory review interval in turns (default: 5).
    pub memory_review_interval: usize,
}

impl Default for BackgroundReviewConfig {
    fn default() -> Self {
        Self {
            max_iterations: 16,
            model: None,
            skill_nudge_interval: 10,
            memory_review_interval: 5,
        }
    }
}

/// State tracking for the self-evolution pipeline.
///
/// Tracks iteration counts and nudge thresholds across turns.
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

    /// Increment the memory review turn counter.
    pub fn bump_memory_counter(&mut self) {
        self.turns_since_memory_review += 1;
    }

    /// Reset the memory review turn counter.
    pub fn reset_memory_counter(&mut self) {
        self.turns_since_memory_review = 0;
    }

    /// Check if a skill review should be triggered.
    pub fn should_review_skills(&self) -> bool {
        self.skill_nudge_interval > 0 && self.iters_since_skill >= self.skill_nudge_interval
    }

    /// Check if a memory review should be triggered.
    pub fn should_review_memory(&self) -> bool {
        self.memory_review_interval > 0
            && self.turns_since_memory_review >= self.memory_review_interval
    }

    /// Check if any review should be triggered and return the result.
    pub fn check_review_triggers(&mut self, skill_manage_called: bool) -> (bool, bool) {
        if skill_manage_called {
            self.reset_skill_counter();
        }

        let review_skills = self.should_review_skills();
        let review_memory = self.should_review_memory();

        if review_skills {
            self.reset_skill_counter();
        }
        if review_memory {
            self.reset_memory_counter();
        }

        (review_skills, review_memory)
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
            ..Default::default()
        };
        let mut state = SelfEvolutionState::new(&config);

        // Bump 4 times — not enough
        for _ in 0..4 {
            state.bump_skill_counter();
        }
        assert!(!state.should_review_skills());

        // Bump once more — triggers
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

        // skill_manage called — resets counter
        let (review_skills, _) = state.check_review_triggers(true);
        assert!(!review_skills); // reset before check
        assert_eq!(state.iters_since_skill, 0);
    }

    #[test]
    fn test_self_evolution_memory_review() {
        let config = BackgroundReviewConfig {
            memory_review_interval: 3,
            ..Default::default()
        };
        let mut state = SelfEvolutionState::new(&config);

        state.bump_memory_counter();
        state.bump_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(state.should_review_memory());
    }
}
