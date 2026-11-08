//! `kernel_exec` — run Python against the persistent per-session kernel.
//!
//! Variables and imports survive across turns (persistent kernel control-plane
//! behavior). With the tool bridge enabled, cells may also call allowlisted
//! operant tools via `await operant_tool(name, args)`.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use super::runtime::KernelRuntime;

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KernelExecArgs {
    code: String,
    /// Optional explicit namespace; defaults to the current session id.
    namespace: Option<String>,
}

pub struct KernelExecTool {
    rt: Arc<KernelRuntime>,
}

impl KernelExecTool {
    pub fn new(rt: Arc<KernelRuntime>) -> Self {
        Self { rt }
    }
}

#[async_trait]
impl OperantTool for KernelExecTool {
    fn name(&self) -> &str {
        "kernel_exec"
    }

    fn description(&self) -> &str {
        "Run Python in a persistent kernel whose variables and imports survive \
         across turns. Supports top-level await; with the tool bridge enabled, \
         `await operant_tool(\"tool_name\", {...})` calls one allowlisted operant \
         tool from inside your program (loops/batching/error-handling in ONE \
         call). NOT a security sandbox: code runs with user permissions."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<KernelExecArgs>("kernel_exec", "Persistent kernel exec")
    }

    fn toolset(&self) -> &str {
        "kernel"
    }

    fn is_available(&self) -> bool {
        runtime_config().tools.kernel.enabled
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: KernelExecArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("kernel_exec", format!("Invalid arguments: {e}"));
            }
        };
        if args.code.trim().is_empty() {
            return ToolResult::error("kernel_exec", "'code' is required");
        }
        let settings = self.rt.settings();
        // The explicit namespace WINS — the field's doc says it "defaults to
        // the current session id", which is the opposite of the session id
        // overriding an explicit namespace (RT-328). The result echoes the
        // namespace actually used, so the old precedence also misreported it.
        let session_key = args
            .namespace
            .or_else(|| context.get("session_id").map(str::to_string))
            .unwrap_or_else(|| "default".to_string());

        let mut params = json!({
            "session_key": session_key,
            "code": args.code,
            // The one-second head-start lets the sidecar's own watchdog fire
            // before the outer request deadline, so its message is the error
            // the user sees. A 0-second setting saturates the subtraction to
            // 0, and 0 to the sidecar means UNBOUNDED — the same floor
            // `code_execution` carries (RT-335 / RT-337).
            "cell_timeout_secs": settings.request_timeout_secs.saturating_sub(1).max(1),
        });
        // Bridge budget resets per cell; drainer enforces max_calls_per_exec.
        if settings.tool_bridge.enabled && self.rt.executor().is_some() {
            self.rt.reset_cell_budget();
            params["enable_bridge"] = Value::Bool(true);
        }

        match self.rt.request("exec", params).await {
            Ok(mut result) => {
                result["persistent"] = json!(true);
                result["session"] = json!(session_key);
                result["bridge_calls"] = json!(self.rt.cell_calls());
                ToolResult::success("kernel_exec", result.to_string())
            }
            Err(e) => ToolResult::error(
                "kernel_exec",
                format!(
                    "{e}. Fix: ensure python>=3.11 + uv, then run \
                     scripts/check-kernel-sidecar.sh; submodule needs \
                     'git submodule update --init --recursive'"
                ),
            ),
        }
    }
}
