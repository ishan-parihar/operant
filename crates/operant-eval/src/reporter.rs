//! Text report for eval runs: per-task pass/fail + summary.
//!
//! [`summary_pass`] returns whether the whole run passed, so a CLI/CI wrapper
//! can exit non-zero on failure.

use crate::verifier::{CheckResult, TaskVerdict};

/// Render a human-readable report for a set of verdicts.
///
/// Returns the report text; the caller decides what to do with it (print,
/// log, write to a file).
pub fn render_report(verdicts: &[TaskVerdict]) -> String {
    let mut out = String::new();
    out.push_str("operant-eval results\n");
    out.push_str("====================\n");
    for verdict in verdicts {
        let mark = if verdict.passed { "PASS" } else { "FAIL" };
        out.push_str(&format!("[{mark}] {}\n", verdict.task_id));
        for check in &verdict.checks {
            let check_mark = if check.passed { "  ok" } else { "  ✗ " };
            out.push_str(&format!("{check_mark} {} — {}\n", check.name, check.detail));
        }
    }
    let passed = verdicts.iter().filter(|v| v.passed).count();
    out.push_str("====================\n");
    out.push_str(&format!("{}/{} tasks passed\n", passed, verdicts.len()));
    out
}

/// True when every verdict passed (empty run counts as pass).
pub fn summary_pass(verdicts: &[TaskVerdict]) -> bool {
    verdicts.iter().all(|v| v.passed)
}

/// True when at least one check in the verdict failed — used to attribute
/// failures to their exact assertion.
pub fn failing_checks(verdict: &TaskVerdict) -> Vec<&CheckResult> {
    verdict.checks.iter().filter(|c| !c.passed).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(id: &str, passed: bool) -> TaskVerdict {
        TaskVerdict {
            task_id: id.to_string(),
            checks: vec![CheckResult {
                name: "actions_in_order",
                passed,
                detail: "detail".to_string(),
            }],
            passed,
        }
    }

    #[test]
    fn render_report_marks_pass_and_fail() {
        let report = render_report(&[verdict("a", true), verdict("b", false)]);
        assert!(report.contains("[PASS] a"));
        assert!(report.contains("[FAIL] b"));
        assert!(report.contains("1/2 tasks passed"));
    }

    #[test]
    fn summary_pass_true_only_when_all_pass() {
        assert!(summary_pass(&[]));
        assert!(summary_pass(&[verdict("a", true), verdict("b", true)]));
        assert!(!summary_pass(&[verdict("a", true), verdict("b", false)]));
    }

    #[test]
    fn failing_checks_returns_only_failures() {
        let v = TaskVerdict {
            task_id: "t".to_string(),
            checks: vec![
                CheckResult {
                    name: "actions_in_order",
                    passed: false,
                    detail: "a".to_string(),
                },
                CheckResult {
                    name: "keywords_present",
                    passed: true,
                    detail: "k".to_string(),
                },
            ],
            passed: false,
        };
        let failures = failing_checks(&v);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].name, "actions_in_order");
    }
}
