//! Environment exposure probe tool — G10 (hermes env_probe intent).
//!
//! Lets the agent proactively audit which environment variables look secret
//! and are exposed to its own process, so secrets are never blind-spotted
//! into a prompt, trajectory, or tool result. Observation-only: reports
//! variable NAMES, never values.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::env_probe::{format_exposure_report, probe_dotenv_file, probe_env_exposure};
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct EnvProbeArgs {
    /// Optional path to a `.env` file to audit without loading it.
    #[serde(default)]
    pub dotenv_path: Option<String>,
}

/// Tool that reports secret-looking environment variable exposures.
pub struct EnvProbeTool;

#[async_trait]
impl OperantTool for EnvProbeTool {
    fn name(&self) -> &str {
        "env_probe"
    }

    fn description(&self) -> &str {
        "Audit the environment for secret-looking variable exposures (API keys, tokens, \
         passwords). Reports variable NAMES and sources only — never values. Use this \
         before reading, writing, or echoing environment-derived data, and to verify a \
         secret is not in context."
    }

    fn schema(&self) -> crate::tools::ToolSchema {
        crate::tools::ToolSchema {
            name: "env_probe".to_string(),
            description: "Audit the environment for secret-looking variable exposures (API keys, tokens, passwords). Reports variable NAMES and sources only — never values. Use before reading or echoing env-derived data.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "dotenv_path": {
                        "type": "string",
                        "description": "Optional path to a .env file to audit without loading it into the process env"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: EnvProbeArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error("env_probe", format!("Invalid arguments: {e}"));
            }
        };

        let mut findings = probe_env_exposure();
        if let Some(path) = parsed.dotenv_path {
            findings.extend(probe_dotenv_file(std::path::Path::new(&path)));
        }
        // Dedupe by name+source.
        findings.sort_by(|a, b| {
            (a.source.as_str(), a.name.as_str()).cmp(&(b.source.as_str(), b.name.as_str()))
        });
        findings.dedup_by(|a, b| a.name == b.name && a.source == b.source);

        let report = format_exposure_report(&findings);
        ToolResult::success("env_probe", report)
    }
}
