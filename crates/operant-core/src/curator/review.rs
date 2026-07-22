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

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, warn};

use crate::agent::{ChatRequest, ModelClient};
use crate::client::{ChatResponse, Message};

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
#[derive(Debug, Clone, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Deserialize)]
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

// ---------------------------------------------------------------------------
// Concrete implementation: wraps any ModelClient
// ---------------------------------------------------------------------------

/// Concrete [`LlmReviewClient`] backed by a [`ModelClient`].
///
/// Sends skill summaries to the LLM as a chat completion and parses the
/// JSON response into verdicts/consolidation verdicts.
pub struct ModelReviewClient {
    client: Arc<dyn ModelClient>,
    model: String,
}

impl ModelReviewClient {
    /// Create a new review client.
    ///
    /// `model` should be the model identifier to use for review calls
    /// (e.g. `"gpt-4o"`, `"claude-sonnet-4-20250514"`).
    pub fn new(client: Arc<dyn ModelClient>, model: String) -> Self {
        Self { client, model }
    }

    /// Send a prompt to the LLM and return the text response.
    async fn chat_completion(&self, prompt: &str) -> Result<String> {
        let messages = vec![
            Message::system("You are a precise JSON-generating assistant. Return ONLY valid JSON, no markdown fences, no explanation."),
            Message::user(prompt),
        ];
        let request = ChatRequest::new(&self.model, messages)
            .with_stream(false);

        let response: ChatResponse = self.client.chat(request).await?;

        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| anyhow::anyhow!("LLM review returned empty response"))
    }
}

#[async_trait::async_trait]
impl LlmReviewClient for ModelReviewClient {
    async fn review_skills(&self, skills: &[SkillSummary]) -> Result<Vec<SkillVerdict>> {
        if skills.is_empty() {
            return Ok(vec![]);
        }

        let skills_json = serde_json::to_string(
            &skills.iter().map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "use_count": s.use_count,
                    "last_used": s.last_used,
                })
            }).collect::<Vec<_>>(),
        )?;

        let prompt = format!(
            "Review the following skills and decide for each whether to \"keep\", \"archive\", or \"deprecate\" it.\n\nRules:\n- Archive skills that haven't been used in 30+ days and are too narrow.\n- Deprecate skills that are redundant with a better skill.\n- Keep skills that are actively used or provide unique value.\n\nSkills:\n{}\n\nReturn a JSON array of objects with keys: skill_name, action, reason.\nReturn ONLY the JSON array, no explanation.",
            skills_json
        );

        let raw = self.chat_completion(&prompt).await?;
        let raw = extract_json_array(&raw);

        let verdicts: Vec<SkillVerdict> = serde_json::from_str(&raw).unwrap_or_else(|e| {
            warn!(error = %e, raw = %raw, "Failed to parse skill review response");
            vec![]
        });

        debug!(count = verdicts.len(), "Skill review verdicts received");
        Ok(verdicts)
    }

    async fn consolidate_skills(
        &self,
        skills: &[SkillSummary],
    ) -> Result<Vec<ConsolidationVerdict>> {
        if skills.is_empty() {
            return Ok(vec![]);
        }

        let skills_json = serde_json::to_string(
            &skills.iter().map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "use_count": s.use_count,
                })
            }).collect::<Vec<_>>(),
        )?;

        let prompt = format!(
            "{}\n\nSkills:\n{}",
            CONSOLIDATION_PROMPT,
            skills_json
        );

        let raw = self.chat_completion(&prompt).await?;
        let raw = extract_json_array(&raw);

        let verdicts: Vec<ConsolidationVerdict> = serde_json::from_str(&raw).unwrap_or_else(|e| {
            warn!(error = %e, raw = %raw, "Failed to parse consolidation response");
            vec![]
        });

        debug!(count = verdicts.len(), "Consolidation verdicts received");
        Ok(verdicts)
    }
}

/// Strip markdown fences and extract the JSON array from the response.
fn extract_json_array(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip ```json ... ``` fences
    if let Some(rest) = trimmed.strip_prefix("```") {
        let inner = rest.strip_prefix("json").unwrap_or(rest);
        let inner = inner.trim();
        // Strip closing fence
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim().to_string();
        }
        return inner.to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A mock ModelClient that returns canned responses.
    struct MockModelClient {
        response: String,
    }

    #[async_trait::async_trait]
    impl ModelClient for MockModelClient {
        async fn chat(&self, _request: ChatRequest) -> crate::error::Result<ChatResponse> {
            Ok(ChatResponse {
                id: "mock".to_string(),
                object: "chat.completion".to_string(),
                created: 0,
                model: "mock".to_string(),
                choices: vec![crate::client::Choice {
                    index: 0,
                    message: crate::client::MessageDelta {
                        role: None,
                        content: Some(self.response.clone()),
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: crate::client::Usage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                },
            })
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> crate::error::Result<futures::stream::BoxStream<'static, crate::error::Result<crate::agent::StreamChunk>>> {
            unimplemented!("not needed for tests")
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_review_skills_empty() {
        let client = ModelReviewClient::new(
            Arc::new(MockModelClient { response: "[]".to_string() }),
            "mock".to_string(),
        );
        let verdicts = client.review_skills(&[]).await.unwrap();
        assert!(verdicts.is_empty());
    }

    #[tokio::test]
    async fn test_review_skills_parses_verdicts() {
        let mock_response = r#"[{"skill_name": "seo-audit", "action": "keep", "reason": "Active use"}]"#;
        let client = ModelReviewClient::new(
            Arc::new(MockModelClient { response: mock_response.to_string() }),
            "mock".to_string(),
        );
        let skills = vec![SkillSummary {
            name: "seo-audit".to_string(),
            description: "SEO audit skill".to_string(),
            use_count: 5,
            last_used: 1000,
        }];
        let verdicts = client.review_skills(&skills).await.unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].skill_name, "seo-audit");
        assert_eq!(verdicts[0].action, "keep");
    }

    #[tokio::test]
    async fn test_consolidate_skills_empty() {
        let client = ModelReviewClient::new(
            Arc::new(MockModelClient { response: "[]".to_string() }),
            "mock".to_string(),
        );
        let verdicts = client.consolidate_skills(&[]).await.unwrap();
        assert!(verdicts.is_empty());
    }

    #[tokio::test]
    async fn test_consolidate_skills_parses_verdicts() {
        let mock_response = r#"[{"umbrella": "web-quality", "absorbed": ["seo-audit", "web-vitals"], "rationale": "Overlap"}]"#;
        let client = ModelReviewClient::new(
            Arc::new(MockModelClient { response: mock_response.to_string() }),
            "mock".to_string(),
        );
        let skills = vec![SkillSummary {
            name: "seo-audit".to_string(),
            description: "SEO audit".to_string(),
            use_count: 10,
            last_used: 500,
        }];
        let verdicts = client.consolidate_skills(&skills).await.unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].umbrella, "web-quality");
        assert_eq!(verdicts[0].absorbed.len(), 2);
    }

    #[test]
    fn test_extract_json_array_plain() {
        assert_eq!(extract_json_array("[{\"a\":1}]"), "[{\"a\":1}]");
    }

    #[test]
    fn test_extract_json_array_fenced() {
        assert_eq!(extract_json_array("```json\n[{\"a\":1}]\n```"), "[{\"a\":1}]");
    }

    #[test]
    fn test_extract_json_array_whitespace() {
        assert_eq!(extract_json_array("  [{\"a\":1}]  "), "[{\"a\":1}]");
    }

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
