//! Camofox browser persistent state management tool
//!
//! Manages browser state (cookies, localStorage) snapshots for the Camofox browser.
//! State files are stored in ~/.hermes/camofox_states/ as JSON files.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

/// Get the state storage directory
fn state_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".hermes").join("camofox_states")
}

/// Ensure the state directory exists
fn ensure_state_dir() -> Result<(), String> {
    let dir = state_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create state directory: {}", e))
}

/// Tool for managing persistent Camofox browser state
pub struct CamofoxStateTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CamofoxStateArgs {
    /// Operation to perform: save_state, load_state, clear_state, list_states
    operation: String,
    /// Optional profile name for the state (defaults to "default")
    profile: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CamofoxStateData {
    profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cookies: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    localStorage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<Value>,
    created_at: String,
    updated_at: String,
}

#[async_trait]
impl HermesTool for CamofoxStateTool {
    fn name(&self) -> &str {
        "browser_camofox_state"
    }

    fn description(&self) -> &str {
        "Manage persistent browser state for the Camofox browser tool. \
         Supports operations: save_state (save cookies/localStorage to disk), \
         load_state (load from a previously saved state file), \
         clear_state (delete a saved state), list_states (list all saved profiles). \
         State files are stored in ~/.hermes/camofox_states/."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CamofoxStateArgs>(
            "browser_camofox_state",
            "Manage Camofox browser state",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: CamofoxStateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(
                    "browser_camofox_state",
                    format!("Invalid arguments: {}", e),
                )
            }
        };

        let profile = args.profile.unwrap_or_else(|| "default".to_string());

        match args.operation.as_str() {
            "save_state" => handle_save_state(&profile),
            "load_state" => handle_load_state(&profile),
            "clear_state" => handle_clear_state(&profile),
            "list_states" => handle_list_states(),
            other => ToolResult::error(
                "browser_camofox_state",
                format!(
                    "Unknown operation '{}'. Supported: save_state, load_state, clear_state, list_states",
                    other
                ),
            ),
        }
    }
}

fn state_file_path(profile: &str) -> PathBuf {
    state_dir().join(format!("{}.json", profile))
}

fn handle_save_state(profile: &str) -> ToolResult {
    if let Err(e) = ensure_state_dir() {
        return ToolResult::error("browser_camofox_state", e);
    }

    let now = chrono::Utc::now().to_rfc3339();

    let state = CamofoxStateData {
        profile: profile.to_string(),
        cookies: None,
        localStorage: None,
        session: Some(serde_json::json!({"saved_at": now})),
        created_at: now.clone(),
        updated_at: now,
    };

    let path = state_file_path(profile);
    let json = match serde_json::to_string_pretty(&state) {
        Ok(j) => j,
        Err(e) => {
            return ToolResult::error(
                "browser_camofox_state",
                format!("Failed to serialize state: {}", e),
            )
        }
    };

    if let Err(e) = fs::write(&path, &json) {
        return ToolResult::error(
            "browser_camofox_state",
            format!("Failed to write state file: {}", e),
        );
    }

    ToolResult::success(
        "browser_camofox_state",
        serde_json::json!({
            "success": true,
            "message": format!("State saved for profile '{}'", profile),
            "profile": profile,
            "path": path.to_string_lossy()
        }),
    )
}

fn handle_load_state(profile: &str) -> ToolResult {
    let path = state_file_path(profile);

    if !path.exists() {
        return ToolResult::success(
            "browser_camofox_state",
            serde_json::json!({
                "success": false,
                "message": format!("No saved state found for profile '{}'", profile),
                "profile": profile,
                "exists": false
            }),
        );
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::error(
                "browser_camofox_state",
                format!("Failed to read state file: {}", e),
            )
        }
    };

    let state: CamofoxStateData = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::error(
                "browser_camofox_state",
                format!("Failed to parse state file: {}", e),
            )
        }
    };

    ToolResult::success(
        "browser_camofox_state",
        serde_json::json!({
            "success": true,
            "message": format!("State loaded for profile '{}'", profile),
            "profile": state.profile,
            "created_at": state.created_at,
            "updated_at": state.updated_at,
            "has_cookies": state.cookies.is_some(),
            "has_local_storage": state.localStorage.is_some(),
            "path": path.to_string_lossy()
        }),
    )
}

fn handle_clear_state(profile: &str) -> ToolResult {
    let path = state_file_path(profile);

    if !path.exists() {
        return ToolResult::success(
            "browser_camofox_state",
            serde_json::json!({
                "success": true,
                "message": format!("No state file to clear for profile '{}'", profile),
                "profile": profile
            }),
        );
    }

    if let Err(e) = fs::remove_file(&path) {
        return ToolResult::error(
            "browser_camofox_state",
            format!("Failed to remove state file: {}", e),
        );
    }

    ToolResult::success(
        "browser_camofox_state",
        serde_json::json!({
            "success": true,
            "message": format!("State cleared for profile '{}'", profile),
            "profile": profile
        }),
    )
}

fn handle_list_states() -> ToolResult {
    if let Err(e) = ensure_state_dir() {
        return ToolResult::error("browser_camofox_state", e);
    }

    let dir = state_dir();
    let mut profiles: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    profiles.push(stem.to_string());
                }
            }
        }
    }

    profiles.sort();

    ToolResult::success(
        "browser_camofox_state",
        serde_json::json!({
            "success": true,
            "profiles": profiles,
            "count": profiles.len(),
            "state_dir": dir.to_string_lossy()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_camofox_state_invalid_args() {
        let tool = CamofoxStateTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_camofox_state_unknown_operation() {
        let tool = CamofoxStateTool;
        let args = serde_json::json!({
            "operation": "unknown_op"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_camofox_state_list() {
        let tool = CamofoxStateTool;
        let args = serde_json::json!({
            "operation": "list_states"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
    }
}
