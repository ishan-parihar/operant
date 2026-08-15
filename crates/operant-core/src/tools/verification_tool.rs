//! Verify Task Tool — agent-facing verification harness (hermes
//! `hermes verify` parity). Detects a project recipe, runs its phases
//! (bootstrap/build/test + optional HTTP readiness start), and records
//! every phase in the `verification_events` evidence ledger.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use crate::database::Database;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};
use crate::verification::{
    CheckSpec, StartSpec, VerificationRecipe, detect_recipe, find_project_root, run_check,
    run_verify,
};

/// Tool that verifies a project against its detected recipe.
pub struct VerifyTaskTool {
    database: Arc<Database>,
}

impl VerifyTaskTool {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct VerifyArgs {
    /// Action: "detect", "run", "ad_hoc", or "evidence"
    action: String,

    /// Working directory for detection/execution (defaults to the process cwd)
    cwd: Option<String>,

    /// For "run": optional start phase {command, port, path} — boots the app
    /// and probes the readiness URL
    start: Option<StartSpec>,

    /// For "run": subset of phases to run, e.g. ["build", "test"]
    phases: Option<Vec<String>>,

    /// For "ad_hoc": list of {cmd, expectedExit, expectContains} checks
    checks: Option<Vec<CheckSpec>>,

    /// For "evidence": number of recent evidence entries to show (default 10)
    limit: Option<u32>,
}

#[async_trait]
impl OperantTool for VerifyTaskTool {
    fn name(&self) -> &str {
        "verify_task"
    }

    fn description(&self) -> &str {
        "Verify work against a project's detected recipe (hermes verify parity). \
         'detect' shows the recipe for a directory; 'run' executes bootstrap/build/test \
         phases and optionally boots the app with an HTTP readiness probe; 'ad_hoc' runs \
         arbitrary check commands with expected-exit/output assertions; 'evidence' lists \
         recent verification records from the evidence ledger."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<VerifyArgs>(
            "verify_task",
            "Verify a project: run its detected build/test recipe, boot and probe the app, or run ad-hoc checks.",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::error("verify_task", "Missing required field: action"),
        };

        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let root = find_project_root(&cwd);

        match action {
            "detect" => self.handle_detect(&root).await,
            "run" => self.handle_run(&root, &args).await,
            "ad_hoc" => self.handle_ad_hoc(&root, &args).await,
            "evidence" => self.handle_evidence(&args).await,
            other => ToolResult::error(
                "verify_task",
                format!(
                    "Unknown action: '{}'. Use detect, run, ad_hoc, or evidence.",
                    other
                ),
            ),
        }
    }
}

impl VerifyTaskTool {
    async fn handle_detect(&self, root: &std::path::Path) -> ToolResult {
        match detect_recipe(root) {
            Some(recipe) => ToolResult::success(
                "verify_task",
                json!({
                    "success": true,
                    "cwd": root.display().to_string(),
                    "recipe": recipe,
                }),
            ),
            None => ToolResult::success(
                "verify_task",
                json!({
                    "success": true,
                    "detected": false,
                    "cwd": root.display().to_string(),
                    "hint": "No known manifest (Cargo.toml, package.json, pyproject.toml, go.mod, pom.xml, Makefile). Use action=ad_hoc for custom checks.",
                }),
            ),
        }
    }

    async fn handle_run(&self, root: &std::path::Path, args: &Value) -> ToolResult {
        let recipe = match detect_recipe(root) {
            Some(r) => r,
            None => {
                return ToolResult::error(
                    "verify_task",
                    "No verification recipe detected in this directory. Use action=ad_hoc with explicit checks.",
                );
            }
        };

        let start = args
            .get("start")
            .cloned()
            .and_then(|v| serde_json::from_value::<StartSpec>(v).ok());
        let include_start = start.is_some();

        let result = run_verify(&recipe, root, include_start, start.as_ref()).await;
        self.record_phases(&recipe, &result.phases);

        ToolResult::success(
            "verify_task",
            json!({
                "success": true,
                "overall_ok": result.ok,
                "recipe": result.recipe,
                "phases": result.phases,
            }),
        )
    }

    async fn handle_ad_hoc(&self, root: &std::path::Path, args: &Value) -> ToolResult {
        let checks: Vec<CheckSpec> = match args.get("checks") {
            Some(c) => match serde_json::from_value(c.clone()) {
                Ok(list) => list,
                Err(e) => {
                    return ToolResult::error(
                        "verify_task",
                        format!("Invalid 'checks' array: {}", e),
                    );
                }
            },
            None => {
                return ToolResult::error(
                    "verify_task",
                    "Missing required field: checks (array of {cmd, expectedExit?, expectContains?})",
                );
            }
        };
        if checks.is_empty() {
            return ToolResult::error("verify_task", "'checks' must not be empty");
        }

        let mut phases = Vec::new();
        for check in &checks {
            phases.push(run_check(check, root).await);
        }
        let overall = phases.iter().all(|p| p.ok);
        self.record_phases(
            &VerificationRecipe {
                name: "ad_hoc".to_string(),
                language: "shell".to_string(),
                bootstrap: vec![],
                build: vec![],
                test: vec![],
            },
            &phases,
        );

        ToolResult::success(
            "verify_task",
            json!({
                "success": true,
                "overall_ok": overall,
                "checks": phases,
            }),
        )
    }

    async fn handle_evidence(&self, args: &Value) -> ToolResult {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
        match self.database.list_verification_events(limit) {
            Ok(events) => ToolResult::success(
                "verify_task",
                json!({
                    "success": true,
                    "count": events.len(),
                    "events": events,
                }),
            ),
            Err(e) => ToolResult::error("verify_task", format!("Evidence query failed: {}", e)),
        }
    }

    fn record_phases(
        &self,
        recipe: &VerificationRecipe,
        phases: &[crate::verification::PhaseResult],
    ) {
        for phase in phases {
            let _ = self.database.record_verification_event(
                &recipe.name,
                &phase.name,
                phase.ok,
                phase.exit_code,
                &phase.output_tail,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> Arc<Database> {
        let dir = TempDir::new().unwrap();
        Arc::new(Database::init(dir.path().join("verify_tool.db")).unwrap())
    }

    #[test]
    fn test_tool_name_and_description() {
        let tool = VerifyTaskTool::new(test_db());
        assert_eq!(tool.name(), "verify_task");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_execute_missing_action() {
        let tool = VerifyTaskTool::new(test_db());
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("action"));
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let tool = VerifyTaskTool::new(test_db());
        let result = tool
            .execute(json!({"action": "explode"}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_detect_no_project() {
        let tool = VerifyTaskTool::new(test_db());
        let dir = TempDir::new().unwrap();
        let result = tool
            .execute(
                json!({"action": "detect", "cwd": dir.path().to_string_lossy()}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["detected"], false);
    }

    #[tokio::test]
    async fn test_detect_rust_project() {
        let tool = VerifyTaskTool::new(test_db());
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let result = tool
            .execute(
                json!({"action": "detect", "cwd": dir.path().to_string_lossy()}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["recipe"]["language"], "rust");
    }

    #[tokio::test]
    async fn test_run_missing_recipe_errors() {
        let tool = VerifyTaskTool::new(test_db());
        let dir = TempDir::new().unwrap();
        let result = tool
            .execute(
                json!({"action": "run", "cwd": dir.path().to_string_lossy()}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_ad_hoc_checks_and_evidence() {
        let tool = VerifyTaskTool::new(test_db());
        let dir = TempDir::new().unwrap();
        let result = tool
            .execute(
                json!({
                    "action": "ad_hoc",
                    "cwd": dir.path().to_string_lossy(),
                    "checks": [
                        {"cmd": "echo good", "expectContains": "good"},
                        {"cmd": "exit 1"}
                    ]
                }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["overall_ok"], false);
        assert_eq!(parsed["checks"][0]["ok"], true);
        assert_eq!(parsed["checks"][1]["ok"], false);

        // Evidence recorded.
        let evidence = tool
            .execute(
                json!({"action": "evidence", "limit": 5}),
                ToolContext::default(),
            )
            .await;
        assert!(evidence.success);
        let e: Value = serde_json::from_str(&evidence.content).unwrap();
        assert!(e["count"].as_u64().unwrap() >= 2);
        assert_eq!(e["events"][0]["recipe"], "ad_hoc");
    }

    #[tokio::test]
    async fn test_evidence_empty() {
        let tool = VerifyTaskTool::new(test_db());
        let evidence = tool
            .execute(json!({"action": "evidence"}), ToolContext::default())
            .await;
        assert!(evidence.success);
        let e: Value = serde_json::from_str(&evidence.content).unwrap();
        assert_eq!(e["count"], 0);
    }
}
