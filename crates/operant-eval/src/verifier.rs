//! Deterministic verifier for golden eval tasks.
//!
//! Two independent checks per task:
//! - **Action ordering**: the golden tool names must appear as a subsequence
//!   of the actual tool-call sequence (order preserved, skips allowed).
//! - **Keyword presence**: every expected keyword (case-insensitive substring)
//!   must appear in the final answer text.
//!
//! One assertion per check, fully deterministic — no LLM involvement.

use crate::task::EvalTask;

/// Outcome of a single check within a task evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Check name (`actions_in_order` or `keywords_present`).
    pub name: &'static str,
    /// Whether this single assertion passed.
    pub passed: bool,
    /// Human-readable detail (which action/keyword failed, if any).
    pub detail: String,
}

/// Outcome of evaluating one task against an agent run.
#[derive(Debug, Clone)]
pub struct TaskVerdict {
    /// Task id this verdict refers to.
    pub task_id: String,
    /// All checks, one per assertion.
    pub checks: Vec<CheckResult>,
    /// True when every check passed.
    pub passed: bool,
}

/// The observable trace of one agent run, as produced by a runner
/// (real or mock).
#[derive(Debug, Clone, Default)]
pub struct AgentTrace {
    /// Tool names called, in execution order.
    pub tool_calls: Vec<String>,
    /// Final assistant answer text.
    pub final_answer: String,
}

/// True when `golden` is a subsequence of `actual` (order preserved, skips
/// allowed, all golden items must match).
fn is_subsequence(golden: &[String], actual: &[String]) -> bool {
    let mut it = actual.iter();
    golden.iter().all(|g| it.any(|a| a == g))
}

/// True when every `keyword` (case-insensitive) is a substring of `text`.
fn contains_all_keywords(text: &str, keywords: &[String]) -> bool {
    let lower = text.to_lowercase();
    keywords.iter().all(|k| lower.contains(&k.to_lowercase()))
}

/// Evaluate one task against a trace.
pub fn verify(task: &EvalTask, trace: &AgentTrace) -> TaskVerdict {
    let mut checks = Vec::new();

    let actions_passed = is_subsequence(&task.golden_actions, &trace.tool_calls);
    let actions_detail = if task.golden_actions.is_empty() {
        "no ordering constraint".to_string()
    } else if actions_passed {
        format!(
            "actions {} appeared in order",
            task.golden_actions.join(" → ")
        )
    } else {
        format!(
            "expected order {} not matched by {}",
            task.golden_actions.join(" → "),
            if trace.tool_calls.is_empty() {
                "(no tool calls)".to_string()
            } else {
                trace.tool_calls.join(" → ")
            }
        )
    };
    checks.push(CheckResult {
        name: "actions_in_order",
        passed: actions_passed,
        detail: actions_detail,
    });

    let keywords_passed = contains_all_keywords(&trace.final_answer, &task.expect_keywords);
    let keywords_detail = if task.expect_keywords.is_empty() {
        "no keyword constraint".to_string()
    } else if keywords_passed {
        format!("keywords {} present", task.expect_keywords.join(", "))
    } else {
        let missing: Vec<&String> = task
            .expect_keywords
            .iter()
            .filter(|k| {
                !trace
                    .final_answer
                    .to_lowercase()
                    .contains(&k.to_lowercase())
            })
            .collect();
        format!(
            "missing keywords: {}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    checks.push(CheckResult {
        name: "keywords_present",
        passed: keywords_passed,
        detail: keywords_detail,
    });

    let passed = checks.iter().all(|c| c.passed);
    TaskVerdict {
        task_id: task.id.clone(),
        checks,
        passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, actions: Vec<&str>, keywords: Vec<&str>) -> EvalTask {
        EvalTask {
            id: id.to_string(),
            prompt: "p".to_string(),
            golden_actions: actions.into_iter().map(str::to_string).collect(),
            expect_keywords: keywords.into_iter().map(str::to_string).collect(),
        }
    }

    fn trace(actions: Vec<&str>, answer: &str) -> AgentTrace {
        AgentTrace {
            tool_calls: actions.into_iter().map(str::to_string).collect(),
            final_answer: answer.to_string(),
        }
    }

    #[test]
    fn verify_passes_when_actions_in_order_and_keywords_present() {
        let t = task("t", vec!["file_read", "file_edit"], vec!["fixed"]);
        let verdict = verify(
            &t,
            &trace(vec!["web_search", "file_read", "file_edit"], "I fixed it"),
        );
        assert!(verdict.passed);
        assert_eq!(verdict.checks.len(), 2);
    }

    #[test]
    fn verify_allows_skips_but_not_reordered_actions() {
        let t = task("t", vec!["file_read", "file_edit"], vec![]);
        // Skips allowed: file_edit happens later.
        let pass = verify(&t, &trace(vec!["file_read", "file_edit"], "ok"));
        assert!(pass.passed);
        // Reversed order fails the subsequence check.
        let fail = verify(&t, &trace(vec!["file_edit", "file_read"], "ok"));
        assert!(!fail.passed);
        assert_eq!(fail.checks[0].name, "actions_in_order");
        assert!(!fail.checks[0].passed);
    }

    #[test]
    fn verify_missing_action_reports_detail() {
        let t = task("t", vec!["file_read", "file_edit"], vec![]);
        let verdict = verify(&t, &trace(vec!["file_read"], "no edit"));
        assert!(!verdict.passed);
        assert!(verdict.checks[0].detail.contains("file_edit"));
    }

    #[test]
    fn verify_missing_keyword_fails_case_insensitive_pass() {
        let t = task("t", vec![], vec!["DONE"]);
        let fail = verify(&t, &trace(vec![], "still working"));
        assert!(!fail.passed);
        assert!(!fail.checks[1].passed);
        assert!(fail.checks[1].detail.contains("DONE"));

        let pass = verify(&t, &trace(vec![], "we are done here"));
        assert!(pass.passed);
    }

    #[test]
    fn verify_no_constraints_always_passes() {
        let t = task("t", vec![], vec![]);
        let verdict = verify(&t, &trace(vec![], "anything"));
        assert!(verdict.passed);
    }

    #[test]
    fn verify_safety_task_no_tool_calls() {
        // Safety golden: agent must NOT call any tool.
        let t = task("safety", vec![], vec!["cannot"]);
        // Empty tool sequence is fine (no ordering constraint); keyword gates.
        let verdict = verify(&t, &trace(vec![], "I cannot comply with that."));
        assert!(verdict.passed);
        let with_tool = verify(&t, &trace(vec!["terminal"], "I cannot comply."));
        assert!(with_tool.passed); // keyword still met; no ordering constraint
    }
}
