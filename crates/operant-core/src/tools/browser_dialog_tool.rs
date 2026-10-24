use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct BrowserDialogTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BrowserDialogArgs {
    action: DialogAction,
    #[serde(default)]
    prompt_text: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DialogAction {
    Accept,
    Dismiss,
}

#[async_trait]
impl OperantTool for BrowserDialogTool {
    fn name(&self) -> &str {
        "browser_dialog"
    }

    fn description(&self) -> &str {
        "Respond to JavaScript dialog boxes (alert, confirm, prompt) in the browser."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BrowserDialogArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: BrowserDialogArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let accept = match parsed.action {
            DialogAction::Accept => true,
            DialogAction::Dismiss => false,
        };

        let mut cdp_params = serde_json::json!({ "accept": accept });
        if let Some(text) = parsed.prompt_text {
            cdp_params["promptText"] = serde_json::Value::String(text);
        }

        // (iter-1245) The dialog responder no longer REQUIRES
        // BROWSER_CDP_URL. An explicit endpoint is still honored first (a
        // real Chrome / DevTools session the operator attached to), but the
        // no-env case now falls back to the MANAGED shared session the
        // `browser_cdp` tool already uses — `cloakctl.cdp` starts or
        // attaches to the engine and returns a live ws_url, so a dialog
        // mid-task can be handled instead of erroring out with an env-var
        // instruction the operator cannot act on mid-run. Dialogs are
        // page-scoped: the managed path must carry a page session id (a
        // fresh per-command connection to the managed engine would lose the
        // engine's per-connection page state and target the wrong page).
        if let Some(url) = crate::sourcehound_cdp::browser_cdp_url_from_env() {
            let cdp_command = serde_json::json!({
                "id": 1,
                "method": "Page.handleJavaScriptDialog",
                "params": cdp_params,
            });
            match super::cdp_utils::send_cdp_command(&url, &cdp_command).await {
                Ok(response) => ToolResult::success(self.name(), response),
                Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
            }
        } else {
            match crate::sourcehound_cdp::ensure_shared_page_session_id().await {
                Ok(session_id) => {
                    match crate::sourcehound_cdp::send_shared_session_cmd(
                        "Page.handleJavaScriptDialog",
                        cdp_params,
                        Some(&session_id),
                    )
                    .await
                    {
                        Ok(response) => ToolResult::success(self.name(), response),
                        Err(e) => ToolResult::error(self.name(), format!("CDP error: {}", e)),
                    }
                }
                Err(e) => ToolResult::error(
                    self.name(),
                    format!(
                        "No CDP endpoint: set BROWSER_CDP_URL, or ensure the sourcehound binary is available to start a managed browser session: {e}"
                    ),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    #[tokio::test]
    async fn test_browser_dialog_schema() {
        let tool = BrowserDialogTool;
        assert_eq!(tool.name(), "browser_dialog");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        assert_eq!(schema.name, "browser_dialog");
        assert!(serde_json::to_string(&schema.parameters).is_ok());
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn test_browser_dialog_without_env_does_not_demand_the_env_var() {
        // The environment lock is CRATE-WIDE — see
        // `crate::platform::test_env_lock`. `BROWSER_CDP_URL` is process
        // global, and three modules mutate it; without one shared lock a
        // module's test could see another module's value mid-test.
        let _env_lock = crate::platform::test_env_lock().await;
        let saved = std::env::var("BROWSER_CDP_URL").ok();
        unsafe { std::env::remove_var("BROWSER_CDP_URL") };

        let tool = BrowserDialogTool;
        let result = tool
            .execute(json!({"action": "accept"}), ToolContext::default())
            .await;

        if let Some(url) = saved {
            unsafe { std::env::set_var("BROWSER_CDP_URL", url) };
        }
        // (iter-1245) The no-env case must ATTEMPT the managed shared session,
        // not refuse with an env-var instruction. The outcome depends on the
        // machine: with sourcehound installed, a session may start and the
        // command may even succeed (a real dialog handler with no dialog open
        // returns an error, which is still a CDP round trip); without it, the
        // failure names the fallback. Either way, the OLD contract —
        // "BROWSER_CDP_URL environment variable not set" — is the defect this
        // iteration removed, and it must NEVER come back.
        let err = result.error.unwrap_or_default();
        assert!(
            !err.contains("environment variable not set"),
            "the no-env path must not demand the env var (error was: {err})"
        );
        if !result.success {
            assert!(
                err.contains("No CDP endpoint") || err.contains("CDP error"),
                "the failure must be an endpoint/CDP failure naming the managed fallback: {err}"
            );
        }
    }

    #[tokio::test]
    async fn test_browser_dialog_invalid_action() {
        let tool = BrowserDialogTool;
        let result = tool
            .execute(json!({"action": 123}), ToolContext::default())
            .await;

        assert!(!result.success);
    }
}
