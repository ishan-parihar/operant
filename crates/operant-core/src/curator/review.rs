//! LLM-driven review types for the curator.
//!
//! Provides the trait and data structures used by the curator engine
//! to delegate skill review decisions to an LLM client.
//!
//! ## Consolidation
//!
//! When consolidation is enabled, the curator also identifies overlapping
//! skills and merges them into class-level "umbrella" skills. This matches
//! hermes-agent's curator consolidation pass.

use anyhow::Result;

// ---------------------------------------------------------------------------
// Skill review types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Consolidation types
// ---------------------------------------------------------------------------

/// A consolidation verdict — identifies which skills should be merged
/// into an umbrella skill.
#[derive(Debug, Clone)]
pub struct ConsolidationVerdict {
    /// The umbrella skill name (may be new or existing).
    pub umbrella: String,
    /// Skills to absorb into the umbrella (excludes the umbrella itself).
    pub absorbed: Vec<String>,
    /// Human-readable rationale for the consolidation.
    pub rationale: String,
}

/// Result of a consolidation run.
#[derive(Debug, Clone, Default)]
pub struct ConsolidationResult {
    /// Skills that were consolidated (absorbed into an umbrella).
    pub consolidated: Vec<ConsolidationEntry>,
    /// Skills that were archived for staleness (not consolidated).
    pub pruned: Vec<String>,
    /// Errors encountered during consolidation.
    pub errors: Vec<String>,
    /// Cron job skill references rewritten after consolidation.
    pub cron_rewrites: Option<crate::cronjobs::CronRewriteReport>,
}

/// A single consolidation entry — records which skill was absorbed
/// into which umbrella.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsolidationEntry {
    /// The skill that was absorbed.
    pub name: String,
    /// The umbrella it was absorbed into.
    pub into: String,
    /// Human-readable reason.
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Consolidation prompt
// ---------------------------------------------------------------------------

/// Prompt for the LLM consolidation pass.
///
/// Matches hermes-agent's curator consolidation prompt structure.
pub const CONSOLIDATION_PROMPT: &str = r#"You are a skill curator. Your job is to identify overlapping skills
and consolidate them into class-level umbrella skills.

Rules:
1. Only consolidate skills that genuinely overlap in topic or scope.
   Do NOT consolidate unrelated skills just because they share a word.
2. The umbrella name must be at the CLASS level — not a specific PR
   number, error string, or session artifact.
3. When absorbing a skill into an umbrella:
   a. Create or update the umbrella's SKILL.md to include the absorbed
      skill's unique insights as a new section.
   b. Archive the original skill directory.
4. Do NOT consolidate pinned skills (marked pinned=true in usage data).
5. Do NOT consolidate built-in/bundled skills.
6. If no consolidation is warranted, return an empty list.

Output format: JSON array of consolidation objects:
[
  {
    "umbrella": "class-level-name",
    "absorbed": ["skill-a", "skill-b"],
    "rationale": "Why these skills overlap and should merge"
  }
]

If nothing should be consolidated, return: []"#;

// ---------------------------------------------------------------------------
// LLM review client trait
// ---------------------------------------------------------------------------

/// Trait for LLM-based curator review.
///
/// Implementations should submit the skill summaries to an LLM and return
/// verdicts for each skill.
#[async_trait::async_trait]
pub trait LlmReviewClient: Send + Sync {
    /// Review a batch of skills and return verdicts.
    async fn review_skills(&self, skills: &[SkillSummary]) -> Result<Vec<SkillVerdict>>;

    /// Identify overlapping skills and return consolidation verdicts.
    ///
    /// The `skills` list includes all agent-created skills with their
    /// summaries. The client should analyze content overlap and return
    /// verdicts for skills that should be merged.
    async fn consolidate_skills(
        &self,
        skills: &[SkillSummary],
    ) -> Result<Vec<ConsolidationVerdict>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consolidation_result_default() {
        let result = ConsolidationResult::default();
        assert!(result.consolidated.is_empty());
        assert!(result.pruned.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_consolidation_entry_fields() {
        let entry = ConsolidationEntry {
            name: "web-search-google".to_string(),
            into: "web-search".to_string(),
            reason: "Overlapping web search skills".to_string(),
        };
        assert_eq!(entry.name, "web-search-google");
        assert_eq!(entry.into, "web-search");
    }

    #[test]
    fn test_consolidation_verdict_fields() {
        let verdict = ConsolidationVerdict {
            umbrella: "code-review".to_string(),
            absorbed: vec!["pr-review".to_string(), "diff-analysis".to_string()],
            rationale: "All review-related skills".to_string(),
        };
        assert_eq!(verdict.umbrella, "code-review");
        assert_eq!(verdict.absorbed.len(), 2);
    }

    #[test]
    fn test_consolidation_prompt_not_empty() {
        assert!(!CONSOLIDATION_PROMPT.is_empty());
        assert!(CONSOLIDATION_PROMPT.contains("umbrella"));
    }

    #[test]
    fn test_consolidation_prompt_rules() {
        assert!(CONSOLIDATION_PROMPT.contains("pinned"));
        assert!(CONSOLIDATION_PROMPT.contains("built-in"));
        assert!(CONSOLIDATION_PROMPT.contains("CLASS level"));
    }

    #[test]
    fn test_consolidation_result_with_entries() {
        let mut result = ConsolidationResult::default();
        result.consolidated.push(ConsolidationEntry {
            name: "web-search-google".to_string(),
            into: "web-search".to_string(),
            reason: "Overlapping".to_string(),
        });
        result.pruned.push("stale-skill".to_string());
        result.errors.push("test error".to_string());
        assert_eq!(result.consolidated.len(), 1);
        assert_eq!(result.pruned.len(), 1);
        assert_eq!(result.errors.len(), 1);
    }
}
