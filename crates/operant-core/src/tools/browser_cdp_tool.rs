use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct BrowserCdpTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct BrowserCdpArgs {
    method: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    target_id: Option<String>,
    /// Optional CDP session id (from `Target.attachToTarget` against the
    /// managed session) to scope the command to a page. Required for
    /// page-level methods like `Page.navigate` / `Runtime.evaluate`.
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
}

#[async_trait]
impl OperantTool for BrowserCdpTool {
    fn name(&self) -> &str {
        "browser_cdp"
    }

    fn description(&self) -> &str {
        "Send raw Chrome DevTools Protocol commands directly to the browser via WebSocket. \
         When BROWSER_CDP_URL is unset, commands run against the managed Obscura session \
         (auto-provisioning a page when no session_id is given) — \
         e.g. Runtime.evaluate {expression: \"document.title\", returnByValue: true}. \
         params may be an object or a JSON-encoded string."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserCdpArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: BrowserCdpArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let params = normalize_params(parsed.params.clone());

        match std::env::var("BROWSER_CDP_URL") {
            Ok(url) if !url.trim().is_empty() => {
                // Explicit external endpoint (real Chrome / DevTools): targets
                // live process-wide there, so a fresh connection per command
                // is fine.
                let target_ws_url = if let Some(ref target_id) = parsed.target_id {
                    match resolve_target_ws_url(&url, target_id).await {
                        Ok(u) => u,
                        Err(e) => {
                            return ToolResult::error(
                                self.name(),
                                format!("Target resolution failed: {}", e),
                            );
                        }
                    }
                } else {
                    url
                };
                let mut command = json!({ "id": 1u64, "method": parsed.method, "params": params });
                if let Some(session_id) = parsed.session_id {
                    command["sessionId"] = Value::String(session_id);
                }
                match super::cdp_utils::send_cdp_command(&target_ws_url, &command).await {
                    Ok(response) => ToolResult::success(self.name(), response),
                    Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
                }
            }
            _ => {
                // Managed Obscura session: pages/sessions are per-connection,
                // so commands must run over the shared persistent socket. If
                // no session_id was given, auto-provision a page session so
                // page-scoped methods (Runtime.evaluate, Page.navigate, ...)
                // just work.
                let session_id = match parsed.session_id.clone() {
                    Some(sid) => Some(sid),
                    None => crate::obscura_cdp::ensure_shared_page_session_id()
                        .await
                        .ok(),
                };
                match crate::obscura_cdp::send_shared_session_cmd(
                    &parsed.method,
                    params,
                    session_id.as_deref(),
                )
                .await
                {
                    Ok(response) => ToolResult::success(self.name(), response),
                    Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
                }
            }
        }
    }
}

/// Normalize tool-call `params` into a JSON object.
///
/// Models frequently pass `params` as a JSON-encoded **string** (e.g.
/// `"params": "{\"expression\": \"document.title\"}"`). Sending that verbatim
/// makes the server read `params` as a string and fail with
/// `-32601 "expression required"` because the field lookup on a string yields
/// nothing. Parse the string into an object (falling back to `{"value": s}`
/// for non-JSON payloads).
fn normalize_params(params: Option<Value>) -> Value {
    match params {
        Some(Value::String(s)) => {
            serde_json::from_str::<Value>(&s).unwrap_or_else(|_| json!({ "value": s }))
        }
        Some(v) => v,
        None => json!({}),
    }
}

async fn resolve_target_ws_url(cdp_url: &str, target_id: &str) -> Result<String, String> {
    let list_url = cdp_url
        .replace("/devtools/browser/", "/json/")
        .replace("ws://", "http://")
        .replace("wss://", "https://");

    let client = reqwest::Client::new();
    let targets: Vec<Value> = client
        .get(&list_url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    for target in targets.iter() {
        let id = target.get("id").and_then(|v: &Value| v.as_str());
        if id == Some(target_id) {
            return target
                .get("webSocketDebuggerUrl")
                .and_then(|v: &Value| v.as_str())
                .map(|s: &str| s.to_string())
                .ok_or_else(|| format!("Target '{}' has no webSocketDebuggerUrl", target_id));
        }
    }

    Err(format!(
        "Target '{}' not found in browser targets",
        target_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    #[tokio::test]
    async fn test_browser_cdp_schema() {
        let tool = BrowserCdpTool;
        assert_eq!(tool.name(), "browser_cdp");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "browser_cdp");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[tokio::test]
    async fn test_browser_cdp_unreachable_endpoint_errors() {
        // An explicit but unreachable BROWSER_CDP_URL must surface a CDP error
        // (the tool now auto-provisions the managed session when the env var is
        // unset, so the deterministic failure path is a bad explicit endpoint).
        let saved = std::env::var("BROWSER_CDP_URL").ok();
        // SAFETY: test-only env mutation
        unsafe { std::env::set_var("BROWSER_CDP_URL", "ws://127.0.0.1:1") };

        let tool = BrowserCdpTool;
        let result = tool
            .execute(
                json!({"method": "Target.getTargets"}),
                ToolContext::default(),
            )
            .await;

        if let Some(url) = saved {
            unsafe { std::env::set_var("BROWSER_CDP_URL", url) };
        } else {
            unsafe { std::env::remove_var("BROWSER_CDP_URL") };
        }
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("CDP"));
    }

    #[tokio::test]
    async fn test_browser_cdp_invalid_args() {
        let tool = BrowserCdpTool;
        let result = tool
            .execute(json!("not_an_object"), ToolContext::default())
            .await;

        assert!(!result.success);
    }

    #[test]
    fn normalize_params_parses_json_strings_into_objects() {
        // The live-test failure: params arrived as a JSON string and obscura
        // rejected the command with "expression required".
        let normalized = normalize_params(Some(json!("{\"expression\": \"document.title\"}")));
        assert_eq!(normalized, json!({"expression": "document.title"}));

        // Objects pass through untouched.
        let obj = normalize_params(Some(json!({"url": "https://example.com"})));
        assert_eq!(obj, json!({"url": "https://example.com"}));

        // Non-JSON strings degrade to a safe value wrapper instead of a
        // malformed command.
        let fallback = normalize_params(Some(json!("not json")));
        assert_eq!(fallback, json!({ "value": "not json" }));

        // Missing params become an empty object.
        assert_eq!(normalize_params(None), json!({}));
    }
}
