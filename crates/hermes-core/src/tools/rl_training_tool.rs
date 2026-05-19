use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{OnceLock, RwLock};
use tokio::process::Command;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

pub struct RlTrainingTool;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct EnvironmentInfo {
    name: String,
    config: Option<Value>,
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RunInfo {
    id: String,
    environment: String,
    config: Value,
    status: String,
    created_at: Option<String>,
    last_output: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RlState {
    environments: Vec<EnvironmentInfo>,
    selected_environment: Option<String>,
    current_config: Option<Value>,
    active_runs: Vec<RunInfo>,
}

static RL_STATE: OnceLock<RwLock<RlState>> = OnceLock::new();

fn get_rl_state() -> &'static RwLock<RlState> {
    RL_STATE.get_or_init(|| RwLock::new(RlState::default()))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RlArgs {
    action: RlAction,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    config_updates: Option<Value>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default)]
    config: Option<String>,
    #[serde(default)]
    hyperparams: Option<Value>,
    #[serde(default)]
    resume_run_id: Option<String>,
    #[serde(default)]
    timesteps: Option<u32>,
    #[serde(default)]
    max_checkpoints: Option<u32>,
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    run_config: Option<Value>,
    #[serde(default)]
    wandb_project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum RlAction {
    ListEnvironments,
    SelectEnvironment,
    GetCurrentConfig,
    EditConfig,
    StartTraining,
    CheckStatus,
    StopTraining,
    GetResults,
    ListRuns,
    TestInference,
}

#[async_trait]
impl HermesTool for RlTrainingTool {
    fn name(&self) -> &str {
        "rl"
    }

    fn description(&self) -> &str {
        "Reinforcement learning training tools. Manage environments, train agents, track runs with WandB."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<RlArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let parsed: RlArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };

        match parsed.action {
            RlAction::ListEnvironments => self.handle_list_environments(&parsed).await,
            RlAction::SelectEnvironment => self.handle_select_environment(&parsed).await,
            RlAction::GetCurrentConfig => self.handle_get_current_config().await,
            RlAction::EditConfig => self.handle_edit_config(&parsed).await,
            RlAction::StartTraining => self.handle_start_training(&parsed).await,
            RlAction::CheckStatus => self.handle_check_status(&parsed).await,
            RlAction::StopTraining => self.handle_stop_training(&parsed).await,
            RlAction::GetResults => self.handle_get_results(&parsed).await,
            RlAction::ListRuns => self.handle_list_runs().await,
            RlAction::TestInference => self.handle_test_inference(&parsed).await,
        }
    }
}

impl RlTrainingTool {
    fn scan_framework_envs() -> Vec<EnvironmentInfo> {
        let hermes_home = std::env::var("HERMES_HOME").unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|p| p.join(".hermes").to_string_lossy().to_string())
                .unwrap_or_else(|| "~/.hermes".to_string())
        });
        let envs_dir = std::path::Path::new(&hermes_home).join("environments");
        let mut envs = Vec::new();

        if envs_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&envs_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .extension()
                        .map_or(false, |e| e == "yaml" || e == "yml")
                    {
                        if let Some(stem) = path.file_stem() {
                            let name = stem.to_string_lossy().to_string();
                            let config = std::fs::read_to_string(&path).ok();
                            let config_val = config.and_then(|c| serde_yaml::from_str(&c).ok());
                            envs.push(EnvironmentInfo {
                                name,
                                config: config_val,
                                status: Some("available".to_string()),
                            });
                        }
                    }
                }
            }
        }

        if envs.is_empty() {
            envs.push(EnvironmentInfo {
                name: "CartPole-v1".to_string(),
                config: Some(json!({
                    "framework": "gym",
                    "max_episode_steps": 500,
                    "description": "Classic cart-pole balancing (Gymnasium)"
                })),
                status: Some("available".to_string()),
            });
            envs.push(EnvironmentInfo {
                name: "MetaDrive".to_string(),
                config: Some(json!({
                    "framework": "gym",
                    "description": "Autonomous driving environment (MetaDrive)"
                })),
                status: Some("available".to_string()),
            });
        }
        envs
    }

    async fn handle_list_environments(&self, _args: &RlArgs) -> ToolResult {
        let envs = Self::scan_framework_envs();
        {
            let state = get_rl_state().read().unwrap();
            if !state.environments.is_empty() && envs.is_empty() {
                return ToolResult::success(
                    self.name(),
                    json!({
                        "environments": state.environments,
                        "selected": state.selected_environment,
                    }),
                );
            }
        }
        {
            let mut state = get_rl_state().write().unwrap();
            state.environments = envs.clone();
        }
        let selected = {
            let state = get_rl_state().read().unwrap();
            state.selected_environment.clone()
        };
        ToolResult::success(
            self.name(),
            json!({
                "environments": envs,
                "selected": selected,
            }),
        )
    }

    async fn handle_select_environment(&self, args: &RlArgs) -> ToolResult {
        let name = match args.name {
            Some(ref n) => n.clone(),
            None => return ToolResult::error(self.name(), "name required"),
        };

        let mut state = get_rl_state().write().unwrap();
        let exists = state.environments.iter().any(|e| e.name == name);
        if !exists {
            let envs = Self::scan_framework_envs();
            state.environments = envs;
            let exists_again = state.environments.iter().any(|e| e.name == name);
            if !exists_again {
                return ToolResult::error(self.name(), format!("Environment '{}' not found", name));
            }
        }
        state.selected_environment = Some(name.clone());
        if let Some(env) = state.environments.iter().find(|e| e.name == name) {
            state.current_config = env.config.clone();
        }
        ToolResult::success(
            self.name(),
            json!({
                "selected_environment": name,
                "message": format!("Environment '{}' selected", name),
            }),
        )
    }

    async fn handle_get_current_config(&self) -> ToolResult {
        let state = get_rl_state().read().unwrap();
        let config = state.current_config.clone().unwrap_or(json!({}));
        ToolResult::success(
            self.name(),
            json!({
                "config": config,
                "environment": state.selected_environment,
            }),
        )
    }

    async fn handle_edit_config(&self, args: &RlArgs) -> ToolResult {
        let updates = match args.config_updates {
            Some(ref v) => v.clone(),
            None => return ToolResult::error(self.name(), "config_updates required"),
        };
        let mut state = get_rl_state().write().unwrap();
        let mut config = state.current_config.clone().unwrap_or(json!({}));
        if let Some(obj) = config.as_object_mut() {
            if let Some(upd_obj) = updates.as_object() {
                for (k, v) in upd_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        state.current_config = Some(config.clone());

        let path = format!(
            "{}/environments/{}.yaml",
            std::env::var("HERMES_HOME").unwrap_or_else(|_| {
                dirs::home_dir()
                    .map(|p| p.join(".hermes").to_string_lossy().to_string())
                    .unwrap_or_else(|| "~/.hermes".to_string())
            }),
            state.selected_environment.as_deref().unwrap_or("default")
        );
        if let Ok(yaml) = serde_yaml::to_string(&config) {
            let _ = std::fs::write(&path, &yaml);
        }

        ToolResult::success(
            self.name(),
            json!({
                "config": config,
                "saved_to": path,
            }),
        )
    }

    async fn handle_start_training(&self, args: &RlArgs) -> ToolResult {
        let state = get_rl_state().read().unwrap();
        let env_name = args
            .environment
            .clone()
            .or_else(|| state.selected_environment.clone())
            .unwrap_or_else(|| "CartPole-v1".to_string());
        drop(state);

        let run_id = format!("rl_{}", chrono::Utc::now().timestamp());
        let config = args
            .config
            .clone()
            .unwrap_or_else(|| "DefaultConfig".to_string());

        let mut cmd = Command::new("hermes");
        cmd.args([
            "environment",
            "launch_training",
            "--environment",
            &env_name,
            "--run-id",
            &run_id,
            "--config",
            &config,
        ]);
        if let Some(hp) = &args.hyperparams {
            if let Ok(hp_str) = serde_json::to_string(hp) {
                cmd.args(["--hyperparams", &hp_str]);
            }
        }
        if let Some(ts) = args.timesteps {
            cmd.args(["--timesteps", &ts.to_string()]);
        }
        if let Some(fw) = &args.framework {
            cmd.args(["--framework", fw]);
        }

        let child = cmd.spawn();
        match child {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);
                let run_info = RunInfo {
                    id: run_id.clone(),
                    environment: env_name.clone(),
                    config: json!({"config": config, "env": env_name}),
                    status: "running".to_string(),
                    created_at: Some(chrono::Utc::now().to_rfc3339()),
                    last_output: None,
                };
                {
                    let mut state = get_rl_state().write().unwrap();
                    state.active_runs.push(run_info);
                }
                ToolResult::success(
                    self.name(),
                    json!({
                        "run_id": run_id,
                        "environment": env_name,
                        "pid": pid,
                        "message": "Training started in background",
                    }),
                )
            }
            Err(e) => ToolResult::error(self.name(), format!("Failed to start training: {}", e)),
        }
    }

    async fn handle_check_status(&self, args: &RlArgs) -> ToolResult {
        let run_id = match args.run_id {
            Some(ref id) => id.clone(),
            None => {
                let state = get_rl_state().read().unwrap();
                match state.active_runs.last() {
                    Some(run) => run.id.clone(),
                    None => return ToolResult::error(self.name(), "No active runs"),
                }
            }
        };

        if let Ok(api_key) = std::env::var("WANDB_API_KEY") {
            let url = format!(
                "https://api.wandb.ai/artifactsV2/runs?run={}&project=rl-training",
                run_id
            );
            let client = reqwest::Client::new();
            match client
                .get(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<Value>().await {
                        return ToolResult::success(
                            self.name(),
                            json!({
                                "run_id": run_id,
                                "status": "queried",
                                "wandb_data": data,
                            }),
                        );
                    }
                }
                Err(_) => {}
            }
        }

        let state = get_rl_state().read().unwrap();
        let run = state.active_runs.iter().find(|r| r.id == run_id);
        match run {
            Some(r) => ToolResult::success(
                self.name(),
                json!({
                    "run_id": run_id,
                    "status": r.status,
                    "environment": r.environment,
                }),
            ),
            None => ToolResult::error(self.name(), format!("Run '{}' not found", run_id)),
        }
    }

    async fn handle_stop_training(&self, args: &RlArgs) -> ToolResult {
        let run_id = match args.run_id {
            Some(ref id) => id.clone(),
            None => {
                return ToolResult::error(self.name(), "run_id required to stop training");
            }
        };

        let mut state = get_rl_state().write().unwrap();
        if let Some(run) = state.active_runs.iter_mut().find(|r| r.id == run_id) {
            run.status = "stopped".to_string();

            let mut cmd = Command::new("hermes");
            cmd.args(["environment", "stop_training", "--run-id", &run_id]);
            let _ = cmd.spawn();

            ToolResult::success(
                self.name(),
                json!({
                    "run_id": run_id,
                    "status": "stopped",
                    "message": format!("Training run '{}' stopped", run_id),
                }),
            )
        } else {
            ToolResult::error(self.name(), format!("Run '{}' not found", run_id))
        }
    }

    async fn handle_get_results(&self, args: &RlArgs) -> ToolResult {
        let run_id = match args.run_id {
            Some(ref id) => id.clone(),
            None => {
                let state = get_rl_state().read().unwrap();
                match state.active_runs.last() {
                    Some(run) => run.id.clone(),
                    None => return ToolResult::error(self.name(), "No runs available"),
                }
            }
        };

        let run_info = {
            let state = get_rl_state().read().unwrap();
            let run = state.active_runs.iter().find(|r| r.id == run_id);
            run.cloned()
        };

        let wandb_results = if let Ok(api_key) = std::env::var("WANDB_API_KEY") {
            let url = format!(
                "https://api.wandb.ai/metrics?run={}&project=rl-training&limit=100",
                run_id
            );
            let client = reqwest::Client::new();
            match client
                .get(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await
            {
                Ok(resp) => resp.json::<Value>().await.ok(),
                Err(_) => None,
            }
        } else {
            None
        };

        ToolResult::success(
            self.name(),
            json!({
                "run_id": run_id,
                "run_info": run_info,
                "wandb_results": wandb_results,
            }),
        )
    }

    async fn handle_list_runs(&self) -> ToolResult {
        let runs = {
            let state = get_rl_state().read().unwrap();
            state.active_runs.clone()
        };
        ToolResult::success(
            self.name(),
            json!({
                "runs": runs,
                "count": runs.len(),
            }),
        )
    }

    async fn handle_test_inference(&self, args: &RlArgs) -> ToolResult {
        let run_id = match args.run_id {
            Some(ref id) => id.clone(),
            None => {
                let state = get_rl_state().read().unwrap();
                match state.active_runs.last() {
                    Some(run) => run.id.clone(),
                    None => return ToolResult::error(self.name(), "No trained models available"),
                }
            }
        };

        let run_config = args.run_config.clone().unwrap_or(json!({}));

        let mut cmd = Command::new("hermes");
        cmd.args([
            "environment",
            "test_inference",
            "--run-id",
            &run_id,
            "--config",
            &serde_json::to_string(&run_config).unwrap_or_default(),
        ]);

        match cmd.output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                ToolResult::success(
                    self.name(),
                    json!({
                        "run_id": run_id,
                        "stdout": stdout,
                        "stderr": stderr,
                        "success": output.status.success(),
                    }),
                )
            }
            Err(e) => ToolResult::error(self.name(), format!("Test inference failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    #[tokio::test]
    async fn test_rl_schema() {
        let tool = RlTrainingTool;
        assert_eq!(tool.name(), "rl");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "rl");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_rl_list_environments() {
        let tool = RlTrainingTool;
        let result = tool
            .execute(
                json!({"action": "listEnvironments"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap_or(json!({}));
        assert!(v.get("environments").is_some());
    }

    #[tokio::test]
    async fn test_rl_select_missing_name() {
        let tool = RlTrainingTool;
        let result = tool
            .execute(
                json!({"action": "selectEnvironment"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_rl_get_config() {
        let tool = RlTrainingTool;
        let result = tool
            .execute(
                json!({"action": "getCurrentConfig"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_rl_list_runs_empty() {
        let tool = RlTrainingTool;
        let result = tool
            .execute(json!({"action": "listRuns"}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_rl_invalid_action() {
        let tool = RlTrainingTool;
        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
