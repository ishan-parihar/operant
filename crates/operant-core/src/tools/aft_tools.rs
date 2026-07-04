//! AFT tools — OperantTool implementations backed by the aft subprocess.
//!
//! Each tool calls `AftBridge::call(command, params)` under the hood,
//! translating the agent's tool-call args into aft's NDJSON protocol
//! and aft's response back into a `ToolResult`.
//!
//! Tools are registered via `register_aft_tools()`, which takes an
//! `AftBridgePool` shared across all tools. The pool lazily spawns one
//! aft subprocess per project root.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
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
/// once at startup if AFT is enabled in config.
pub async fn register_aft_tools(
    registry: &ToolRegistry,
    pool: Arc<AftBridgePool>,
) -> Result<()> {
    registry.register(AftReadTool { pool: pool.clone() }).await?;
    registry.register(AftWriteTool { pool: pool.clone() }).await?;
    registry.register(AftEditTool { pool: pool.clone() }).await?;
    registry.register(AftApplyPatchTool { pool: pool.clone() }).await?;
    registry.register(AftBashTool { pool: pool.clone() }).await?;
    registry.register(AftSearchTool { pool: pool.clone() }).await?;
    registry.register(AftOutlineTool { pool: pool.clone() }).await?;
    registry.register(AftZoomTool { pool: pool.clone() }).await?;
    registry.register(AftInspectTool { pool: pool.clone() }).await?;
    registry.register(AftCallgraphTool { pool: pool.clone() }).await?;
    registry.register(AftGrepTool { pool: pool.clone() }).await?;
    registry.register(AftGlobTool { pool: pool.clone() }).await?;
    registry.register(AftAstSearchTool { pool: pool.clone() }).await?;
    registry.register(AftAstReplaceTool { pool: pool.clone() }).await?;
    registry.register(AftSafetyTool { pool }).await?;
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
        Ok(response) => {
            let result = response.get("result").cloned().unwrap_or(response);
            ToolResult::success(tool_name, result)
        }
        Err(e) => ToolResult::error(tool_name, format!("aft {}: {}", command, e)),
    }
}

// ---------------------------------------------------------------------------
// Sensory tools (perceive code)
// ---------------------------------------------------------------------------

pub struct AftReadTool { pool: Arc<AftBridgePool> }

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
    fn name(&self) -> &str { "aft_read" }
    fn description(&self) -> &str {
        "Read a file's contents using aft (tree-sitter-aware). Supports optional line ranges. More efficient than basic file_read for large files."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftReadArgs>("aft_read", "Read a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_read", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "filePath": args.file_path });
        if let Some(start) = args.start_line { params["startLine"] = json!(start); }
        if let Some(end) = args.end_line { params["endLine"] = json!(end); }
        execute_aft_command(&self.pool, &context, "aft_read", "read", params).await
    }
}

pub struct AftSearchTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftSearchArgs {
    query: String,
    #[serde(default)]
    file_pattern: Option<String>,
}

#[async_trait]
impl OperantTool for AftSearchTool {
    fn name(&self) -> &str { "aft_search" }
    fn description(&self) -> &str {
        "Semantic + trigram full-text search across the project. Returns matching file paths + line numbers + snippets. More accurate than basic file_search."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftSearchArgs>("aft_search", "Search code via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_search", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "query": args.query });
        if let Some(fp) = args.file_pattern { params["filePattern"] = json!(fp); }
        execute_aft_command(&self.pool, &context, "aft_search", "search", params).await
    }
}

pub struct AftOutlineTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftOutlineArgs {
    file_path: String,
}

#[async_trait]
impl OperantTool for AftOutlineTool {
    fn name(&self) -> &str { "aft_outline" }
    fn description(&self) -> &str {
        "Get the tree-sitter symbol outline of a file (functions, classes, methods, etc. with line ranges). Useful for understanding file structure without reading the whole file."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftOutlineArgs>("aft_outline", "Get file outline via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftOutlineArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_outline", format!("Invalid arguments: {}", e)),
        };
        let params = serde_json::json!({ "filePath": args.file_path });
        execute_aft_command(&self.pool, &context, "aft_outline", "outline", params).await
    }
}

pub struct AftZoomTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftZoomArgs {
    file_path: String,
    symbol: String,
}

#[async_trait]
impl OperantTool for AftZoomTool {
    fn name(&self) -> &str { "aft_zoom" }
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
        let params = serde_json::json!({ "filePath": args.file_path, "symbol": args.symbol });
        execute_aft_command(&self.pool, &context, "aft_zoom", "zoom", params).await
    }
}

pub struct AftInspectTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftInspectArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    depth: Option<usize>,
}

#[async_trait]
impl OperantTool for AftInspectTool {
    fn name(&self) -> &str { "aft_inspect" }
    fn description(&self) -> &str {
        "Inspect codebase health: dead code, unused imports, complexity hotspots, dependency cycles. Returns a structured report."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftInspectArgs>("aft_inspect", "Inspect codebase health via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftInspectArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_inspect", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({});
        if let Some(p) = args.path { params["path"] = json!(p); }
        if let Some(d) = args.depth { params["depth"] = json!(d); }
        execute_aft_command(&self.pool, &context, "aft_inspect", "inspect", params).await
    }
}

pub struct AftCallgraphTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftCallgraphArgs {
    symbol: String,
    #[serde(default)]
    direction: Option<String>,
}

#[async_trait]
impl OperantTool for AftCallgraphTool {
    fn name(&self) -> &str { "aft_callgraph" }
    fn description(&self) -> &str {
        "Get the call graph for a symbol (who calls it, what it calls). Uses LSP + tree-sitter for accurate cross-file navigation."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftCallgraphArgs>("aft_callgraph", "Get call graph via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftCallgraphArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_callgraph", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "symbol": args.symbol });
        if let Some(d) = args.direction { params["direction"] = json!(d); }
        execute_aft_command(&self.pool, &context, "aft_callgraph", "callgraph", params).await
    }
}

pub struct AftGrepTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftGrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl OperantTool for AftGrepTool {
    fn name(&self) -> &str { "aft_grep" }
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
        let mut params = serde_json::json!({ "pattern": args.pattern });
        if let Some(p) = args.path { params["path"] = json!(p); }
        execute_aft_command(&self.pool, &context, "aft_grep", "grep", params).await
    }
}

pub struct AftGlobTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftGlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl OperantTool for AftGlobTool {
    fn name(&self) -> &str { "aft_glob" }
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
        let mut params = serde_json::json!({ "pattern": args.pattern });
        if let Some(p) = args.path { params["path"] = json!(p); }
        execute_aft_command(&self.pool, &context, "aft_glob", "glob", params).await
    }
}

pub struct AftAstSearchTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftAstSearchArgs {
    pattern: String,
    #[serde(default)]
    language: Option<String>,
}

#[async_trait]
impl OperantTool for AftAstSearchTool {
    fn name(&self) -> &str { "aft_ast_search" }
    fn description(&self) -> &str {
        "AST pattern search (ast-grep) across the project. Match code structure, not just text. E.g. find all 'if ($X == null) return' patterns."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftAstSearchArgs>("aft_ast_search", "AST search via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftAstSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_ast_search", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "pattern": args.pattern });
        if let Some(l) = args.language { params["language"] = json!(l); }
        execute_aft_command(&self.pool, &context, "aft_ast_search", "ast_search", params).await
    }
}

// ---------------------------------------------------------------------------
// Motor tools (act on code)
// ---------------------------------------------------------------------------

pub struct AftWriteTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftWriteArgs {
    file_path: String,
    content: String,
}

#[async_trait]
impl OperantTool for AftWriteTool {
    fn name(&self) -> &str { "aft_write" }
    fn description(&self) -> &str {
        "Write/create a file via aft. Creates parent directories if needed. Safer than basic file_write — validates paths and handles symlinks."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftWriteArgs>("aft_write", "Write a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_write", format!("Invalid arguments: {}", e)),
        };
        let params = serde_json::json!({ "filePath": args.file_path, "content": args.content });
        execute_aft_command(&self.pool, &context, "aft_write", "write", params).await
    }
}

pub struct AftEditTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: Option<bool>,
}

#[async_trait]
impl OperantTool for AftEditTool {
    fn name(&self) -> &str { "aft_edit" }
    fn description(&self) -> &str {
        "AST-aware string replacement in a file via aft. More robust than basic patch — handles whitespace normalization and validates the edit doesn't break syntax."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftEditArgs>("aft_edit", "Edit a file via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftEditArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_edit", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({
            "filePath": args.file_path,
            "oldString": args.old_string,
            "newString": args.new_string,
        });
        if let Some(ra) = args.replace_all { params["replaceAll"] = json!(ra); }
        execute_aft_command(&self.pool, &context, "aft_edit", "edit", params).await
    }
}

pub struct AftApplyPatchTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftApplyPatchArgs {
    file_path: String,
    patch: String,
}

#[async_trait]
impl OperantTool for AftApplyPatchTool {
    fn name(&self) -> &str { "aft_apply_patch" }
    fn description(&self) -> &str {
        "Apply a unified diff patch to a file via aft. Validates the patch applies cleanly before writing."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftApplyPatchArgs>("aft_apply_patch", "Apply a patch via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftApplyPatchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_apply_patch", format!("Invalid arguments: {}", e)),
        };
        let params = serde_json::json!({ "filePath": args.file_path, "patch": args.patch });
        execute_aft_command(&self.pool, &context, "aft_apply_patch", "apply_patch", params).await
    }
}

pub struct AftAstReplaceTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftAstReplaceArgs {
    pattern: String,
    replacement: String,
    #[serde(default)]
    language: Option<String>,
}

#[async_trait]
impl OperantTool for AftAstReplaceTool {
    fn name(&self) -> &str { "aft_ast_replace" }
    fn description(&self) -> &str {
        "AST pattern replacement (ast-grep) across the project. Replace code structure, not just text. E.g. rename a function call pattern across all files."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftAstReplaceArgs>("aft_ast_replace", "AST replace via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftAstReplaceArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_ast_replace", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "pattern": args.pattern, "replacement": args.replacement });
        if let Some(l) = args.language { params["language"] = json!(l); }
        execute_aft_command(&self.pool, &context, "aft_ast_replace", "ast_replace", params).await
    }
}

// ---------------------------------------------------------------------------
// Brainstem tools (keep it alive)
// ---------------------------------------------------------------------------

pub struct AftBashTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftBashArgs {
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    background: Option<bool>,
}

#[async_trait]
impl OperantTool for AftBashTool {
    fn name(&self) -> &str { "aft_bash" }
    fn description(&self) -> &str {
        "Execute a bash command via aft. Supports PTY mode, output compression, and background execution. More capable than basic terminal — handles long-running commands and large outputs gracefully."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftBashArgs>("aft_bash", "Run bash via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftBashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_bash", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "command": args.command });
        if let Some(t) = args.timeout_ms { params["timeoutMs"] = json!(t); }
        if let Some(b) = args.background { params["background"] = json!(b); }
        execute_aft_command(&self.pool, &context, "aft_bash", "bash", params).await
    }
}

pub struct AftSafetyTool { pool: Arc<AftBridgePool> }

#[derive(JsonSchema, Deserialize)]
struct AftSafetyArgs {
    op: String,
    #[serde(default)]
    checkpoint_name: Option<String>,
    #[serde(default)]
    restore_target: Option<String>,
}

#[async_trait]
impl OperantTool for AftSafetyTool {
    fn name(&self) -> &str { "aft_safety" }
    fn description(&self) -> &str {
        "Safety operations: undo last edit, create checkpoint, restore from checkpoint, list checkpoints, view edit history. Use to recover from bad edits."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<AftSafetyArgs>("aft_safety", "Safety operations via aft")
    }
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: AftSafetyArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("aft_safety", format!("Invalid arguments: {}", e)),
        };
        let mut params = serde_json::json!({ "op": args.op });
        if let Some(n) = args.checkpoint_name { params["checkpointName"] = json!(n); }
        if let Some(r) = args.restore_target { params["restoreTarget"] = json!(r); }
        execute_aft_command(&self.pool, &context, "aft_safety", "safety", params).await
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
            Box::new(AftCallgraphTool { pool: pool.clone() }),
            Box::new(AftGrepTool { pool: pool.clone() }),
            Box::new(AftGlobTool { pool: pool.clone() }),
            Box::new(AftAstSearchTool { pool: pool.clone() }),
            Box::new(AftAstReplaceTool { pool: pool.clone() }),
            Box::new(AftSafetyTool { pool }),
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
