//! Golden-task definitions, YAML-loadable from `operant-eval/tasks/*.yaml`.

use serde::{Deserialize, Serialize};

/// A single golden eval task: a prompt plus the tool-call ordering and/or
/// keywords the agent must satisfy.
///
/// ```yaml
/// id: tool-ordering
/// prompt: "Read README.md then fix the typo in line 3."
/// golden_actions: ["file_read", "file_edit"]
/// expect_keywords: ["fixed"]
/// forbid_actions: ["terminal"]  # optional negative constraint
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalTask {
    /// Unique task id (used in reports).
    pub id: String,
    /// The user prompt fed to the agent.
    pub prompt: String,
    /// Tool names the agent must call, **in order** (subsequence match).
    /// Empty = no ordering constraint.
    #[serde(default)]
    pub golden_actions: Vec<String>,
    /// Substrings (case-insensitive) that must appear in the final answer.
    /// Empty = no keyword constraint.
    #[serde(default)]
    pub expect_keywords: Vec<String>,
    /// Tool names the agent must **not** call. Any call to a forbidden tool
    /// fails the task. Empty = no restriction.
    #[serde(default)]
    pub forbid_actions: Vec<String>,
}

impl EvalTask {
    /// Load all tasks from `*.yaml` files in `dir` (non-recursive).
    ///
    /// Returns `(tasks, errors)` so a missing/parse-broken file is reported
    /// instead of silently skipped: the caller decides whether to fail.
    pub fn load_from_dir(dir: &std::path::Path) -> (Vec<EvalTask>, Vec<String>) {
        let mut tasks = Vec::new();
        let mut errors = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(format!("cannot read {}: {e}", dir.display()));
                return (tasks, errors);
            }
        };
        let mut paths: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("cannot read {}: {e}", path.display()));
                    continue;
                }
            };
            match serde_yaml::from_str::<EvalTask>(&text) {
                Ok(task) => tasks.push(task),
                Err(e) => errors.push(format!("parse error in {}: {e}", path.display())),
            }
        }
        (tasks, errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_dir_loads_all_yaml_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.yaml"),
            "id: task-a\nprompt: hello\ngolden_actions: [file_read]\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.yaml"),
            "id: task-b\nprompt: world\nexpect_keywords: [done]\n",
        )
        .unwrap();
        // Non-yaml + broken yaml
        std::fs::write(dir.path().join("c.txt"), "not yaml").unwrap();
        std::fs::write(dir.path().join("d.yaml"), "id: [unclosed").unwrap();

        let (tasks, errors) = EvalTask::load_from_dir(dir.path());
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-a");
        assert_eq!(tasks[0].golden_actions, vec!["file_read"]);
        assert!(tasks[1].expect_keywords.contains(&"done".to_string()));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("d.yaml"));
    }

    #[test]
    fn load_from_dir_missing_dir_reports_error() {
        let (tasks, errors) = EvalTask::load_from_dir(std::path::Path::new("/no/such/dir"));
        assert!(tasks.is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn golden_tasks_dir_loads_cleanly() {
        // Guard the shipped tasks/*.yaml against schema drift.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tasks");
        let (tasks, errors) = EvalTask::load_from_dir(&dir);
        assert!(errors.is_empty(), "golden tasks failed to load: {errors:?}");
        assert!(!tasks.is_empty());
    }
}
