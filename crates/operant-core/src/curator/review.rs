//! LLM-driven review types for the curator.
//!
//! Provides the trait and data structures used by the curator engine
//! to delegate skill review decisions to an LLM client.

use anyhow::Result;

/// Summary of a skill for LLM review.
#[derive(Debug, Clone)]
pub struct SkillSummary {
    /// Skill name.
    pub name: String,
    /// Skill description from SKILL.md frontmatter.
    pub description: String,
    /// Number of times the skill has been used.
    pub use_count: u64,
    /// Unix timestamp of last usage.
    pub last_used: i64,
}

/// Verdict from the LLM for a single skill.
#[derive(Debug, Clone)]
pub struct SkillVerdict {
    /// The skill this verdict applies to.
    pub skill_name: String,
    /// Recommended action: "keep", "archive", or "deprecate".
    pub action: String,
    /// Human-readable reason for the decision.
    pub reason: String,
}

/// Trait for LLM-based curator review.
///
/// Implementations should submit the skill summaries to an LLM and return
/// verdicts for each skill.
#[async_trait::async_trait]
pub trait LlmReviewClient: Send + Sync {
    /// Review a batch of skills and return verdicts.
    async fn review_skills(&self, skills: &[SkillSummary]) -> Result<Vec<SkillVerdict>>;
}
