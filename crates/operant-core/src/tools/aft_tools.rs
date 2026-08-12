//! AFT tools — OperantTool implementations backed by the aft subprocess.
//!
//! Each tool calls `AftBridge::call(command, params)` under the hood,
//! translating the agent's tool-call args into aft's NDJSON protocol
//! (v0.49.x: params FLAT at top level, except `bash` which is nested and
//! async — see [`AftBridge::bash`]) and aft's flat response back into a
//! `ToolResult`.
//!
//! Tools are registered via `register_aft_tools()`, which takes an
//! `AftBridgePool` shared across all tools. The pool lazily spawns one
//! aft subprocess per project root.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::aft_bridge::AftBridgePool;
use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all AFT tools with a registry, sharing the given bridge pool.
///
/// The pool lazily spawns one aft subprocess per project root. Call this
/// once at startup if AFT is enabled in config. Registration is best-effort:
/// every tool is registered regardless of whether the binary resolves —
/// failures surface at call time, and the CLI keeps native tools as a
/// fallback (natural degradation, no tool-less agent).
pub async fn register_aft_tools(registry: &ToolRegistry, pool: Arc<AftBridgePool>) -> Result<()> {
    registry
        .register(AftReadTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftWriteTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftEditTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftApplyPatchTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftBashTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftSearchTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftOutlineTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftZoomTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftInspectTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftCallersTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftGrepTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftGlobTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftAstSearchTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftAstReplaceTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftCheckpointTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftListCheckpointsTool { pool: pool.clone() })
        .await?;
    registry
        .register(AftUndoTool { pool: pool.clone() })
        .await?;
    registry.register(AftStatusTool { pool }).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: get project root from context or fall back to cwd
// ---------------------------------------------------------------------------

fn project_root_from_context(context: &ToolContext) -> std::path::PathBuf {
    context
        .get("project_root")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
}

/// Strip protocol noise (id/success/code) from an aft response so the
/// model sees only meaningful payload (content/output/text/message/…).
fn response_payload(response: &Value) -> Value {
    let mut out = response.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("id");
        obj.remove("success");
        obj.remove("code");
    }
    out
}

/// Execute an aft command via the bridge pool and convert the response
/// to a ToolResult. Shared by all aft tools.
async fn execute_aft_command(
    pool: &AftBridgePool,
    context: &ToolContext,
    tool_name: &str,
    command: &str,
    params: Value,
) -> ToolResult {
    let project_root = project_root_from_context(context);
    let bridge = match pool.get(&project_root).await {
        Ok(b) => b,
        Err(e) => return ToolResult::error(tool_name, format!("aft bridge spawn failed: {}", e)),
    };
    match bridge.call(command, params).await {
        Ok(response) => ToolResult::success(tool_name, response_payload(&response)),
        Err(e) => ToolResult::error(tool_name, format!("aft {}: {}", command, e)),
    }
}

// ---------------------------------------------------------------------------
// Sensory tools (perceive code)
// ---------------------------------------------------------------------------

pub struct AftReadTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftReadArgs {
    file_path: String,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

#[async_trait]
impl OperantTool for AftReadTool {
    fn name(&self) -> &str {
        "aft_read"
    }
    fn description(&self) -> &str {
        "Read a file's contents using aft (tree-sitter-aware). Supports optional line ranges. Returns content prefixed with line numbers plus truncation metadata. More efficient than basic file_read for large files."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftReadArgs>("aft_read", "Read a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_read", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "file": args.file_path });
        if let Some(start) = args.start_line {
            params["start_line"] = json!(start);
        }
        if let Some(end) = args.end_line {
            params["end_line"] = json!(end);
        }
        execute_aft_command(&self.pool, &context, "aft_read", "read", params).await
    }
}

pub struct AftSearchTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftSearchArgs {
    query: String,
}

#[async_trait]
impl OperantTool for AftSearchTool {
    fn name(&self) -> &str {
        "aft_search"
    }
    fn description(&self) -> &str {
        "Semantic + trigram full-text search across the project. Returns matching file paths + line numbers + snippets (lexical fallback when the semantic index is not enabled). More accurate than basic file_search."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftSearchArgs>("aft_search", "Search code via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_search", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_search",
            "semantic_search",
            json!({ "query": args.query }),
        )
        .await
    }
}

pub struct AftOutlineTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftOutlineArgs {
    file_path: String,
}

#[async_trait]
impl OperantTool for AftOutlineTool {
    fn name(&self) -> &str {
        "aft_outline"
    }
    fn description(&self) -> &str {
        "Get the tree-sitter symbol outline of a file (functions, classes, methods, etc.). Useful for understanding file structure without reading the whole file."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftOutlineArgs>("aft_outline", "Get file outline via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftOutlineArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_outline", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_outline",
            "outline",
            json!({ "file": args.file_path }),
        )
        .await
    }
}

pub struct AftZoomTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftZoomArgs {
    file_path: String,
    symbol: String,
}

#[async_trait]
impl OperantTool for AftZoomTool {
    fn name(&self) -> &str {
        "aft_zoom"
    }
    fn description(&self) -> &str {
        "Zoom into a specific symbol's definition body in a file. Returns the full source of the function/method/class without surrounding context."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftZoomArgs>("aft_zoom", "Zoom into a symbol via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftZoomArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_zoom", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_zoom",
            "zoom",
            json!({ "file": args.file_path, "symbol": args.symbol }),
        )
        .await
    }
}

pub struct AftInspectTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftInspectArgs {
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl OperantTool for AftInspectTool {
    fn name(&self) -> &str {
        "aft_inspect"
    }
    fn description(&self) -> &str {
        "Inspect codebase health: dead code, unused imports, complexity hotspots, dependency cycles. Returns a structured report. Runs the configure handshake automatically."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftInspectArgs>("aft_inspect", "Inspect codebase health via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftInspectArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_inspect", format!("Invalid arguments: {}", e)),
        };
        let mut params = json!({});
        if let Some(p) = args.path {
            params["path"] = json!(p);
        }
        execute_aft_command(&self.pool, &context, "aft_inspect", "inspect", params).await
    }
}

pub struct AftCallersTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftCallersArgs {
    file_path: String,
    symbol: String,
}

#[async_trait]
impl OperantTool for AftCallersTool {
    fn name(&self) -> &str {
        "aft_callers"
    }
    fn description(&self) -> &str {
        "Find the callers of a symbol within a file (who calls it). Uses tree-sitter for accurate cross-file navigation. Runs the configure handshake automatically."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftCallersArgs>("aft_callers", "Get callers via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftCallersArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_callers", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_callers",
            "callers",
            json!({ "file": args.file_path, "symbol": args.symbol }),
        )
        .await
    }
}

pub struct AftGrepTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftGrepArgs {
    pattern: String,
}

#[async_trait]
impl OperantTool for AftGrepTool {
    fn name(&self) -> &str {
        "aft_grep"
    }
    fn description(&self) -> &str {
        "Regex grep across the project via aft. Returns matching file:line:content triples."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftGrepArgs>("aft_grep", "Grep via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftGrepArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_grep", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_grep",
            "grep",
            json!({ "pattern": args.pattern }),
        )
        .await
    }
}

pub struct AftGlobTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftGlobArgs {
    pattern: String,
}

#[async_trait]
impl OperantTool for AftGlobTool {
    fn name(&self) -> &str {
        "aft_glob"
    }
    fn description(&self) -> &str {
        "Glob file pattern matching via aft. Returns matching file paths."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftGlobArgs>("aft_glob", "Glob via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftGlobArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_glob", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_glob",
            "glob",
            json!({ "pattern": args.pattern }),
        )
        .await
    }
}

pub struct AftAstSearchTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftAstSearchArgs {
    pattern: String,
    #[serde(default)]
    lang: Option<String>,
}

#[async_trait]
impl OperantTool for AftAstSearchTool {
    fn name(&self) -> &str {
        "aft_ast_search"
    }
    fn description(&self) -> &str {
        "AST pattern search across the project. Match code structure, not just text (e.g. find all 'if ($X == null)' patterns)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftAstSearchArgs>("aft_ast_search", "AST search via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftAstSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("aft_ast_search", format!("Invalid arguments: {}", e));
            }
        };
        let mut params = json!({ "pattern": args.pattern });
        if let Some(l) = args.lang {
            params["lang"] = json!(l);
        }
        execute_aft_command(&self.pool, &context, "aft_ast_search", "ast_search", params).await
    }
}

// ---------------------------------------------------------------------------
// Motor tools (act on code)
// ---------------------------------------------------------------------------

pub struct AftWriteTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftWriteArgs {
    file_path: String,
    content: String,
}

#[async_trait]
impl OperantTool for AftWriteTool {
    fn name(&self) -> &str {
        "aft_write"
    }
    fn description(&self) -> &str {
        "Write/create a file via aft. Creates parent directories if needed, validates paths, and snapshots a backup for undo. Safer than basic file_write."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftWriteArgs>("aft_write", "Write a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_write", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_write",
            "write",
            json!({ "file": args.file_path, "content": args.content }),
        )
        .await
    }
}

pub struct AftEditTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
}

#[async_trait]
impl OperantTool for AftEditTool {
    fn name(&self) -> &str {
        "aft_edit"
    }
    fn description(&self) -> &str {
        "Literal match-and-replace in a file via aft (edit_match). Replaces every occurrence of old_string with new_string and snapshots a backup for undo. More robust than basic patch — validates the edit and reports backup metadata."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftEditArgs>("aft_edit", "Edit a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftEditArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_edit", format!("Invalid arguments: {}", e)),
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_edit",
            "edit_match",
            json!({
                "file": args.file_path,
                "match": args.old_string,
                "replacement": args.new_string,
            }),
        )
        .await
    }
}

pub struct AftApplyPatchTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftApplyPatchArgs {
    patch: String,
}

#[async_trait]
impl OperantTool for AftApplyPatchTool {
    fn name(&self) -> &str {
        "aft_apply_patch"
    }
    fn description(&self) -> &str {
        "Apply a patch to the project via aft. The patch uses the '*** Begin Patch / *** Update File: <path>' format (aft's own patch dialect — not unified diff). Reports hunks applied, partial/failed status, and a diff."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftApplyPatchArgs>("aft_apply_patch", "Apply a patch via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftApplyPatchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("aft_apply_patch", format!("Invalid arguments: {}", e));
            }
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_apply_patch",
            "apply_patch",
            json!({ "patch_text": args.patch }),
        )
        .await
    }
}

pub struct AftAstReplaceTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftAstReplaceArgs {
    pattern: String,
    rewrite: String,
    #[serde(default)]
    lang: Option<String>,
}

#[async_trait]
impl OperantTool for AftAstReplaceTool {
    fn name(&self) -> &str {
        "aft_ast_replace"
    }
    fn description(&self) -> &str {
        "AST pattern replacement across the project. Replace code structure, not just text (e.g. rename a call pattern across all files). `rewrite` is the replacement template."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftAstReplaceArgs>("aft_ast_replace", "AST replace via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftAstReplaceArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("aft_ast_replace", format!("Invalid arguments: {}", e));
            }
        };
        let mut params = json!({ "pattern": args.pattern, "rewrite": args.rewrite });
        if let Some(l) = args.lang {
            params["lang"] = json!(l);
        }
        execute_aft_command(
            &self.pool,
            &context,
            "aft_ast_replace",
            "ast_replace",
            params,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Brainstem tools (keep it alive)
// ---------------------------------------------------------------------------

pub struct AftBashTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftBashArgs {
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait]
impl OperantTool for AftBashTool {
    fn name(&self) -> &str {
        "aft_bash"
    }
    fn description(&self) -> &str {
        "Execute a bash command via aft. Uses aft's async task model (bash → bash_completed) with output compression and token accounting. More capable than basic terminal — handles long-running commands and large outputs gracefully. Response includes exit_code, output_preview, and compression stats."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftBashArgs>("aft_bash", "Run bash via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftBashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_bash", format!("Invalid arguments: {}", e)),
        };
        let project_root = project_root_from_context(&context);
        let bridge = match self.pool.get(&project_root).await {
            Ok(b) => b,
            Err(e) => {
                return ToolResult::error("aft_bash", format!("aft bridge spawn failed: {}", e));
            }
        };
        match bridge.bash(&args.command, args.timeout_ms).await {
            Ok(response) => {
                let mut payload = response_payload(&response);
                // Surface the completion frame's output preview as `output`
                // so the model sees the shell result regardless of shape.
                if payload.get("output").is_none()
                    && let Some(preview) = payload.get("output_preview").and_then(|v| v.as_str())
                {
                    payload["output"] = json!(preview);
                }
                ToolResult::success("aft_bash", payload)
            }
            Err(e) => ToolResult::error("aft_bash", format!("aft bash: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Safety tools (recover from bad edits)
// ---------------------------------------------------------------------------

pub struct AftCheckpointTool {
    pool: Arc<AftBridgePool>,
}

#[derive(JsonSchema, Deserialize)]
struct AftCheckpointArgs {
    name: String,
}

#[async_trait]
impl OperantTool for AftCheckpointTool {
    fn name(&self) -> &str {
        "aft_checkpoint"
    }
    fn description(&self) -> &str {
        "Create a named checkpoint snapshot of the project state. Use before risky multi-file edits so they can be undone later."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftCheckpointArgs>("aft_checkpoint", "Create checkpoint via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftCheckpointArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("aft_checkpoint", format!("Invalid arguments: {}", e));
            }
        };
        execute_aft_command(
            &self.pool,
            &context,
            "aft_checkpoint",
            "checkpoint",
            json!({ "name": args.name }),
        )
        .await
    }
}

/// Argument-less tools need a real object schema (`type: "object"` with
/// empty properties) — `serde_json::Value` produces `type: null`, which
/// strict OpenAI-compatible providers (e.g. opencode.ai) reject with
/// "Invalid schema ... got 'type: null'" (found in live loop testing).
#[derive(JsonSchema, Deserialize)]
struct EmptyArgs {}

pub struct AftListCheckpointsTool {
    pool: Arc<AftBridgePool>,
}

#[async_trait]
impl OperantTool for AftListCheckpointsTool {
    fn name(&self) -> &str {
        "aft_list_checkpoints"
    }
    fn description(&self) -> &str {
        "List all aft checkpoints (name, file count, created timestamp)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EmptyArgs>("aft_list_checkpoints", "List checkpoints via aft")
    }
    async fn execute(&self, _args: Value, context: ToolContext) -> ToolResult {
        execute_aft_command(
            &self.pool,
            &context,
            "aft_list_checkpoints",
            "list_checkpoints",
            json!({}),
        )
        .await
    }
}

pub struct AftUndoTool {
    pool: Arc<AftBridgePool>,
}

#[async_trait]
impl OperantTool for AftUndoTool {
    fn name(&self) -> &str {
        "aft_undo"
    }
    fn description(&self) -> &str {
        "Undo the last aft write/edit operation, restoring the pre-edit backup. Returns what was restored. Use to recover from bad edits."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EmptyArgs>("aft_undo", "Undo last edit via aft")
    }
    async fn execute(&self, _args: Value, context: ToolContext) -> ToolResult {
        execute_aft_command(&self.pool, &context, "aft_undo", "undo", json!({})).await
    }
}

pub struct AftStatusTool {
    pool: Arc<AftBridgePool>,
}

#[async_trait]
impl OperantTool for AftStatusTool {
    fn name(&self) -> &str {
        "aft_status"
    }
    fn description(&self) -> &str {
        "Check the aft bridge health/status. Returns aft's status payload. Use to verify the IDE-grade tool backend is alive."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EmptyArgs>("aft_status", "aft bridge status")
    }
    async fn execute(&self, _args: Value, context: ToolContext) -> ToolResult {
        execute_aft_command(&self.pool, &context, "aft_status", "status", json!({})).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool() -> Arc<AftBridgePool> {
        Arc::new(AftBridgePool::new())
    }

    #[test]
    fn aft_read_tool_schema_is_valid() {
        let tool = AftReadTool { pool: make_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "aft_read");
    }

    #[test]
    fn aft_edit_tool_schema_is_valid() {
        let tool = AftEditTool { pool: make_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "aft_edit");
    }

    #[test]
    fn aft_bash_tool_schema_is_valid() {
        let tool = AftBashTool { pool: make_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "aft_bash");
    }

    #[test]
    fn aft_search_tool_schema_is_valid() {
        let tool = AftSearchTool { pool: make_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "aft_search");
    }

    #[test]
    fn aft_outline_tool_schema_is_valid() {
        let tool = AftOutlineTool { pool: make_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "aft_outline");
    }

    #[test]
    fn all_aft_tool_names_are_prefixed() {
        // All aft tools must have names starting with "aft_" so they're
        // distinguishable from the basic built-in tools.
        let pool = make_pool();
        let tools: Vec<Box<dyn OperantTool>> = vec![
            Box::new(AftReadTool { pool: pool.clone() }),
            Box::new(AftWriteTool { pool: pool.clone() }),
            Box::new(AftEditTool { pool: pool.clone() }),
            Box::new(AftApplyPatchTool { pool: pool.clone() }),
            Box::new(AftBashTool { pool: pool.clone() }),
            Box::new(AftSearchTool { pool: pool.clone() }),
            Box::new(AftOutlineTool { pool: pool.clone() }),
            Box::new(AftZoomTool { pool: pool.clone() }),
            Box::new(AftInspectTool { pool: pool.clone() }),
            Box::new(AftCallersTool { pool: pool.clone() }),
            Box::new(AftGrepTool { pool: pool.clone() }),
            Box::new(AftGlobTool { pool: pool.clone() }),
            Box::new(AftAstSearchTool { pool: pool.clone() }),
            Box::new(AftAstReplaceTool { pool: pool.clone() }),
            Box::new(AftCheckpointTool { pool: pool.clone() }),
            Box::new(AftListCheckpointsTool { pool: pool.clone() }),
            Box::new(AftUndoTool { pool: pool.clone() }),
            Box::new(AftStatusTool { pool }),
        ];
        for tool in &tools {
            assert!(
                tool.name().starts_with("aft_"),
                "tool name '{}' should start with 'aft_'",
                tool.name()
            );
        }
    }
}
