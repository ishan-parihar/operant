use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use schemars::JsonSchema;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::tools::{HermesTool, ToolContext, ToolResult};
use crate::schema::ToolSchema;

pub struct ComputerUseTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CuaArgs {
    action: CuaAction,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    #[serde(default)]
    element: Option<i32>,
    #[serde(default)]
    button: Option<String>,
    #[serde(default)]
    click_count: Option<i32>,
    #[serde(default)]
    modifiers: Option<Vec<String>>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    amount: Option<i32>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    keys: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    pid: Option<i32>,
    #[serde(default)]
    window_id: Option<i32>,
    #[serde(default)]
    from_element: Option<i32>,
    #[serde(default)]
    to_element: Option<i32>,
    #[serde(default)]
    from_x: Option<i32>,
    #[serde(default)]
    from_y: Option<i32>,
    #[serde(default)]
    to_x: Option<i32>,
    #[serde(default)]
    to_y: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum CuaAction {
    Capture,
    Click,
    DoubleClick,
    RightClick,
    MiddleClick,
    Drag,
    Scroll,
    Type,
    Key,
    SetValue,
    Wait,
    ListApps,
    FocusApp,
}

#[async_trait]
impl HermesTool for ComputerUseTool {
    fn name(&self) -> &str {
        "computer_use"
    }

    fn description(&self) -> &str {
        "Background macOS desktop control via cua-driver. Supports 13 actions: capture (screenshot/AX tree), click, double_click, right_click, middle_click, drag, scroll, type, key, set_value, wait, list_apps, focus_app. macOS-only, requires cua-driver binary."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CuaArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: CuaArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid args: {}", e)),
        };

        match parsed.action {
            CuaAction::Capture => self.handle_capture(&parsed).await,
            CuaAction::Click => self.handle_click(&parsed, 1, "left").await,
            CuaAction::DoubleClick => self.handle_click(&parsed, 2, "left").await,
            CuaAction::RightClick => self.handle_click(&parsed, 1, "right").await,
            CuaAction::MiddleClick => self.handle_click(&parsed, 1, "middle").await,
            CuaAction::Drag => self.handle_drag(&parsed).await,
            CuaAction::Scroll => self.handle_scroll(&parsed).await,
            CuaAction::Type => self.handle_type(&parsed).await,
            CuaAction::Key => self.handle_key(&parsed).await,
            CuaAction::SetValue => self.handle_set_value(&parsed).await,
            CuaAction::Wait => self.handle_wait(&parsed).await,
            CuaAction::ListApps => self.handle_list_apps().await,
            CuaAction::FocusApp => self.handle_focus_app(&parsed).await,
        }
    }
}

impl ComputerUseTool {
    fn check_cua_driver() -> bool {
        std::process::Command::new("cua-driver")
            .arg("--version")
            .output()
            .is_ok()
    }

    fn install_hint() -> String {
        "cua-driver not found. Install:\n  /bin/bash -c \"$(curl -fsSL \
         https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh)\""
            .to_string()
    }

    async fn call_mcp_tool(action: &str, tool_args: Value) -> std::result::Result<Value, String> {
        let mut child = Command::new("cua-driver")
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn cua-driver: {}", e))?;

        let mut writer = child.stdin.take().ok_or("No stdin")?;
        let stdout = child.stdout.take().ok_or("No stdout")?;
        let mut reader = BufReader::new(stdout);

        let init = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "hermes-rs", "version": "0.1.0"}
            }
        });
        writer
            .write_all(format!("{}\n", serde_json::to_string(&init).unwrap()).as_bytes())
            .await
            .map_err(|e| format!("Write error: {}", e))?;

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("Read error: {}", e))?;

        let notif = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        writer
            .write_all(format!("{}\n", serde_json::to_string(&notif).unwrap()).as_bytes())
            .await
            .map_err(|e| format!("Write error: {}", e))?;

        let tool_call = json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": action, "arguments": tool_args}
        });
        writer
            .write_all(format!("{}\n", serde_json::to_string(&tool_call).unwrap()).as_bytes())
            .await
            .map_err(|e| format!("Write error: {}", e))?;

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("Read error: {}", e))?;

        let _ = child.kill().await;

        let response: Value =
            serde_json::from_str(&line).map_err(|e| format!("JSON parse error: {}", e))?;

        if let Some(err) = response.get("error") {
            return Err(format!("MCP error: {}", err));
        }

        Ok(response)
    }

    async fn handle_capture(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let mode = args.mode.clone().unwrap_or_else(|| "som".to_string());

        let lw_result = Self::call_mcp_tool("list_windows", json!({"on_screen_only": true})).await;
        let lw_response = match lw_result {
            Ok(v) => v,
            Err(e) => return ToolResult::error(self.name(), e),
        };

        let windows = lw_response
            .pointer("/result/content/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("[]");

        let captured = if mode == "vision" {
            Self::call_mcp_tool(
                "screenshot",
                json!({"format": "jpeg", "quality": 85}),
            )
            .await
        } else {
            Self::call_mcp_tool("get_window_state", json!({})).await
        };

        match captured {
            Ok(response) => ToolResult::success(
                self.name(),
                json!({
                    "windows": windows,
                    "result": response,
                    "mode": mode,
                }),
            ),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_click(&self, args: &CuaArgs, click_count: i32, button: &str) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let pid = match args.pid {
            Some(p) => p,
            None => return ToolResult::error(self.name(), "pid required for click actions"),
        };

        let tool_name = match (button, click_count) {
            ("right", _) => "right_click",
            (_, 2) => "double_click",
            _ => "click",
        };

        let mut tool_args = json!({"pid": pid});
        if let Some(elem) = args.element {
            tool_args["element_index"] = json!(elem);
            if let Some(wid) = args.window_id {
                tool_args["window_id"] = json!(wid);
            }
        } else if let (Some(x), Some(y)) = (args.x, args.y) {
            tool_args["x"] = json!(x);
            tool_args["y"] = json!(y);
        } else {
            return ToolResult::error(self.name(), "click requires element= or x/y");
        }
        if let Some(ref mods) = args.modifiers {
            tool_args["modifier"] = json!(mods);
        }

        match Self::call_mcp_tool(tool_name, tool_args).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_drag(&self, _args: &CuaArgs) -> ToolResult {
        ToolResult::error(self.name(), "drag is not supported by the cua-driver backend")
    }

    async fn handle_scroll(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let pid = match args.pid {
            Some(p) => p,
            None => return ToolResult::error(self.name(), "pid required for scroll"),
        };

        let direction = args.direction.clone().unwrap_or_else(|| "down".to_string());
        let amount = args.amount.unwrap_or(3).max(1).min(50);

        let mut tool_args = json!({"pid": pid, "direction": direction, "amount": amount});
        if let Some(elem) = args.element {
            tool_args["element_index"] = json!(elem);
            if let Some(wid) = args.window_id {
                tool_args["window_id"] = json!(wid);
            }
        } else if let (Some(x), Some(y)) = (args.x, args.y) {
            tool_args["x"] = json!(x);
            tool_args["y"] = json!(y);
        }

        match Self::call_mcp_tool("scroll", tool_args).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_type(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let pid = match args.pid {
            Some(p) => p,
            None => return ToolResult::error(self.name(), "pid required for type"),
        };
        let text = match args.text {
            Some(ref t) => t.clone(),
            None => return ToolResult::error(self.name(), "text required for type"),
        };

        match Self::call_mcp_tool("type_text_chars", json!({"pid": pid, "text": text})).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_key(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let pid = match args.pid {
            Some(p) => p,
            None => return ToolResult::error(self.name(), "pid required for key"),
        };
        let keys = match args.keys {
            Some(ref k) => k.clone(),
            None => return ToolResult::error(self.name(), "keys required for key"),
        };

        let parts: Vec<&str> = keys.split(|c| c == '+' || c == '-').collect();
        let mod_names = ["cmd", "command", "shift", "option", "alt", "ctrl", "control", "fn"];
        let modifiers: Vec<&str> = parts.iter().filter(|p| mod_names.contains(p)).copied().collect();
        let key = parts.iter().find(|p| !mod_names.contains(p)).copied();

        if modifiers.is_empty() {
            match Self::call_mcp_tool("press_key", json!({"pid": pid, "key": key})).await {
                Ok(response) => ToolResult::success(self.name(), response),
                Err(e) => ToolResult::error(self.name(), e),
            }
        } else if let Some(k) = key {
            let norm_mods: Vec<&str> = modifiers
                .iter()
                .map(|m| match *m {
                    "command" => "cmd",
                    "alt" => "option",
                    "control" => "ctrl",
                    _ => m,
                })
                .collect();
            let mut hotkey_parts: Vec<Value> = norm_mods.iter().map(|m| json!(m)).collect();
            hotkey_parts.push(json!(k));
            match Self::call_mcp_tool("hotkey", json!({"pid": pid, "keys": hotkey_parts})).await {
                Ok(response) => ToolResult::success(self.name(), response),
                Err(e) => ToolResult::error(self.name(), e),
            }
        } else {
            ToolResult::error(self.name(), "Could not parse key from input")
        }
    }

    async fn handle_set_value(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let pid = match args.pid {
            Some(p) => p,
            None => return ToolResult::error(self.name(), "pid required for set_value"),
        };
        let window_id = match args.window_id {
            Some(w) => w,
            None => return ToolResult::error(self.name(), "window_id required for set_value"),
        };
        let element = match args.element {
            Some(e) => e,
            None => return ToolResult::error(self.name(), "element required for set_value"),
        };
        let value = match args.value {
            Some(ref v) => v.clone(),
            None => return ToolResult::error(self.name(), "value required for set_value"),
        };

        let tool_args = json!({
            "pid": pid, "window_id": window_id, "element_index": element, "value": value
        });

        match Self::call_mcp_tool("set_value", tool_args).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_wait(&self, _args: &CuaArgs) -> ToolResult {
        ToolResult::success(self.name(), json!({"message": "wait completed"}))
    }

    async fn handle_list_apps(&self) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        match Self::call_mcp_tool("list_apps", json!({})).await {
            Ok(response) => ToolResult::success(self.name(), response),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }

    async fn handle_focus_app(&self, args: &CuaArgs) -> ToolResult {
        if !Self::check_cua_driver() {
            return ToolResult::error(self.name(), Self::install_hint());
        }

        let app = match args.app {
            Some(ref a) => a.clone(),
            None => return ToolResult::error(self.name(), "app required for focus_app"),
        };

        let lw_result = Self::call_mcp_tool("list_windows", json!({"on_screen_only": true})).await;
        match lw_result {
            Ok(response) => ToolResult::success(self.name(), json!({"result": response, "app": app})),
            Err(e) => ToolResult::error(self.name(), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    #[tokio::test]
    async fn test_computer_use_schema() {
        let tool = ComputerUseTool;
        assert_eq!(tool.name(), "computer_use");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "computer_use");
        let props = &schema.parameters["properties"];
        assert!(props.get("action").is_some());
    }

    #[tokio::test]
    async fn test_computer_use_missing_pid() {
        let tool = ComputerUseTool;
        let result = tool
            .execute(
                json!({"action": "click", "x": 100, "y": 200}),
                ToolContext::default(),
            )
            .await;

        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_computer_use_invalid_action() {
        let tool = ComputerUseTool;
        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;

        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_computer_use_wait_succeeds() {
        let tool = ComputerUseTool;
        let result = tool
            .execute(json!({"action": "wait"}), ToolContext::default())
            .await;

        assert!(result.success);
    }
}
