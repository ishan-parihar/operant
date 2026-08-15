//! Progressive tool disclosure ("tool search") for Operant.
//!
//! When active, MCP server tools (names prefixed `mcp_`, registered by
//! `McpNamespacedTool`) are replaced in the model-visible tools array by
//! three bridge tools — `tool_search`, `tool_describe`, `tool_call` — and
//! surfaced on demand. Native builtin tools never defer.
//!
//! Design constraints (hermes `tools/tool_search.py` parity):
//!
//! * **Native tools never defer.** Only MCP tools (the `mcp_*` namespace)
//!   are deferrable. Always-load means always-load.
//! * **Tiered disclosure:**
//!   - Tier 0 — no MCP tools present (or `enabled: "off"`): pure
//!     passthrough, everything eager, bridge tools hidden.
//!   - Tier 1 — MCP tools present and the catalog listing fits the
//!     listing budget (`min(threshold_pct% of context, listing_max_tokens)`):
//!     bridge + skills-style listing (name + short description per tool),
//!     degrading to a names-only listing when the full form is over budget.
//!   - Tier 2 — even names-only is over budget: bare bridge + a
//!     one-line-per-server summary (server name + tool count) so the model
//!     still knows WHICH domains are reachable; individual tools are
//!     discoverable only via `tool_search`.
//! * **The catalog is stateless across turns and tools-array assemblies.**
//!   It is rebuilt from the live registry every time (`tool_search` /
//!   `tool_describe` / `tool_call` all read the registry at call time).
//! * **Bridge tools route through the registry executor exactly like a
//!   direct call**, so timeouts, guardrails, and tool-result truncation
//!   fire identically. `tool_call` executes the underlying tool via
//!   `ToolRegistry::execute` and returns its result verbatim.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::ToolSearchSettings;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// Bridge tool names. Reserved — any registered tool with these names is
/// treated as part of the bridge, never deferred and never hidden while
/// the bridge is active.
pub const TOOL_SEARCH_NAME: &str = "tool_search";
pub const TOOL_DESCRIBE_NAME: &str = "tool_describe";
pub const TOOL_CALL_NAME: &str = "tool_call";
pub const BRIDGE_TOOL_NAMES: &[&str] = &[TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME];

/// Cheap token estimate from char count without a real tokenizer (~4 chars
/// per token for English+JSON). Underestimating errs toward NOT activating
/// the listing (safe default); overestimating would truncate listings that
/// actually fit.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Whether a tool name is deferrable. Only MCP server tools (`mcp_*`)
/// defer; every native tool stays eager.
pub fn is_deferrable(name: &str) -> bool {
    name.starts_with("mcp_")
}

/// Result of assembling the model-visible tools array.
pub struct AssembledTools {
    /// Schemas to send to the LLM (core tools + bridge tools when active).
    pub visible: Vec<ToolSchema>,
    /// Deferred (MCP) schemas hidden behind the bridge this assembly.
    pub deferred: Vec<ToolSchema>,
}

/// Partition schemas and apply tiered disclosure.
pub fn assemble_tools(
    all: Vec<ToolSchema>,
    settings: &ToolSearchSettings,
    context_window: usize,
) -> AssembledTools {
    let mut core = Vec::new();
    let mut deferred = Vec::new();
    for s in all {
        if is_deferrable(&s.name) {
            deferred.push(s);
        } else {
            core.push(s);
        }
    }
    // The bridge tools are registered like any other tool; they are always
    // excluded from the "core" pass so they never duplicate.
    core.retain(|s| !BRIDGE_TOOL_NAMES.contains(&s.name.as_str()));

    let active = match settings.enabled.as_str() {
        "on" => true,
        "off" => false,
        // "auto": activate only when deferrable tools exist.
        _ => !deferred.is_empty(),
    };

    if !active || deferred.is_empty() {
        // Tier 0 — pure passthrough. EVERYTHING stays eager (including
        // any MCP tools); bridge tools stay hidden.
        let mut visible = core;
        visible.extend(deferred);
        return AssembledTools {
            visible,
            deferred: Vec::new(),
        };
    }

    let listing = build_listing(&deferred, settings, context_window);
    let mut visible = core;
    visible.push(build_search_schema(&listing));
    visible.push(build_describe_schema());
    visible.push(build_call_schema());

    AssembledTools { visible, deferred }
}

/// Resolve the listing mode against the budget:
/// "full" → skills-style listing; "names" → names-only; "bare" → Tier 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListingMode {
    Full,
    Names,
    Bare,
}

fn listing_enabled(settings: &ToolSearchSettings) -> bool {
    settings.listing != "off"
}

/// Effective listing token budget = min(listing_max_tokens, threshold% of
/// context window). A zero context window means "unknown" → rely on the
/// absolute cap only.
fn listing_budget(settings: &ToolSearchSettings, context_window: usize) -> usize {
    let pct_budget = if context_window > 0 {
        (settings.threshold_pct / 100.0 * context_window as f64) as usize
    } else {
        usize::MAX
    };
    settings.listing_max_tokens.min(pct_budget)
}

fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as usize
}

/// Decide which listing form fits the budget.
fn resolve_listing_mode(
    deferred: &[ToolSchema],
    settings: &ToolSearchSettings,
    context_window: usize,
) -> ListingMode {
    if !listing_enabled(settings) {
        return ListingMode::Bare;
    }
    let budget = listing_budget(settings, context_window);
    let full = full_listing_text(deferred);
    if estimate_tokens(&full) <= budget {
        return ListingMode::Full;
    }
    let names = names_listing_text(deferred);
    if estimate_tokens(&names) <= budget {
        return ListingMode::Names;
    }
    ListingMode::Bare
}

fn full_listing_text(deferred: &[ToolSchema]) -> String {
    deferred
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

fn names_listing_text(deferred: &[ToolSchema]) -> String {
    deferred
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Group deferred tools by their `mcp_<server>_` prefix for the Tier 2
/// per-server summary.
fn server_summary(deferred: &[ToolSchema]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for s in deferred {
        // `mcp_<server>_<tool>` — strip the leading `mcp_` and take the
        // first underscore-delimited segment as the server name (tool
        // names may themselves contain underscores).
        let rest = s.name.strip_prefix("mcp_").unwrap_or(&s.name);
        let server = match rest.split_once('_') {
            Some((server, _)) => server,
            None => rest,
        };
        match counts.iter_mut().find(|(name, _)| name == server) {
            Some((_, n)) => *n += 1,
            None => counts.push((server.to_string(), 1)),
        }
    }
    counts
        .iter()
        .map(|(server, n)| format!("- {}: {} tool(s)", server, n))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_listing(
    deferred: &[ToolSchema],
    settings: &ToolSearchSettings,
    context_window: usize,
) -> String {
    match resolve_listing_mode(deferred, settings, context_window) {
        ListingMode::Full => full_listing_text(deferred),
        ListingMode::Names => format!(
            "Deferred tools ({}): {}",
            deferred.len(),
            names_listing_text(deferred)
        ),
        ListingMode::Bare => format!(
            "Deferred MCP servers ({} tool(s) total):\n{}",
            deferred.len(),
            server_summary(deferred)
        ),
    }
}

fn build_search_schema(listing: &str) -> ToolSchema {
    let listing_block = if listing.is_empty() {
        String::new()
    } else {
        format!("\n\nCurrently deferred tools (call tool_describe for full schemas):\n{listing}")
    };
    ToolSchema::new(
        TOOL_SEARCH_NAME,
        format!(
            "Search the catalog of deferred (MCP) tools that are NOT directly callable this \
             turn. Returns matching tool names and short descriptions. Load a full JSON schema \
             with tool_describe, then invoke with tool_call.{listing_block}"
        ),
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to match against deferred tool names and descriptions. Empty matches all."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return."
                }
            },
            "required": ["query"]
        }),
    )
}

fn build_describe_schema() -> ToolSchema {
    ToolSchema::new(
        TOOL_DESCRIBE_NAME,
        "Return the full JSON schema for one deferred (MCP) tool returned by tool_search, so \
         you can construct valid arguments before calling tool_call.",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact deferred tool name (as returned by tool_search)."
                }
            },
            "required": ["name"]
        }),
    )
}

fn build_call_schema() -> ToolSchema {
    ToolSchema::new(
        TOOL_CALL_NAME,
        "Invoke a deferred (MCP) tool by its exact name with JSON arguments. Use tool_search to \
         discover the name and tool_describe to load its schema first.",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact deferred tool name to execute."
                },
                "arguments": {
                    "type": "object",
                    "description": "Tool arguments as a JSON object."
                }
            },
            "required": ["name", "arguments"]
        }),
    )
}

fn synthetic_call_id(tool: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{tool}_{nanos}")
}

/// `tool_search` — search the deferred catalog (rebuilt statelessly from
/// the live registry at call time).
pub struct ToolSearchTool {
    registry: ToolRegistry,
}

impl ToolSearchTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }

    /// The deferred schemas currently registered (MCP tools only).
    async fn deferred_schemas(&self) -> Vec<ToolSchema> {
        self.registry
            .get_schemas()
            .await
            .into_iter()
            .filter(|s| is_deferrable(&s.name))
            .collect()
    }
}

#[async_trait]
impl OperantTool for ToolSearchTool {
    fn name(&self) -> &str {
        TOOL_SEARCH_NAME
    }

    fn description(&self) -> &str {
        "Search the catalog of deferred (MCP) tools not directly callable this turn."
    }

    fn schema(&self) -> ToolSchema {
        build_search_schema("")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(10)
            .min(25);

        let deferred = self.deferred_schemas().await;
        let mut matches: Vec<(String, String)> = deferred
            .iter()
            .filter(|s| {
                query.is_empty()
                    || s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| (s.name.clone(), s.description.clone()))
            .collect();
        matches.sort_by(|a, b| a.0.cmp(&b.0));
        matches.truncate(limit);

        ToolResult::success(
            synthetic_call_id(TOOL_SEARCH_NAME),
            json!({
                "query": query,
                "total": deferred.len(),
                "matches": matches
                    .into_iter()
                    .map(|(name, description)| json!({ "name": name, "description": description }))
                    .collect::<Vec<_>>()
            }),
        )
    }
}

/// `tool_describe` — return the full JSON schema for one deferred tool.
pub struct ToolDescribeTool {
    registry: ToolRegistry,
}

impl ToolDescribeTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl OperantTool for ToolDescribeTool {
    fn name(&self) -> &str {
        TOOL_DESCRIBE_NAME
    }

    fn description(&self) -> &str {
        "Return the full JSON schema for one deferred (MCP) tool."
    }

    fn schema(&self) -> ToolSchema {
        build_describe_schema()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = match args.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                return ToolResult::error(
                    synthetic_call_id(TOOL_DESCRIBE_NAME),
                    "tool_describe requires a non-empty 'name' argument".to_string(),
                );
            }
        };

        let schema = self
            .registry
            .get_schemas()
            .await
            .into_iter()
            .find(|s| s.name == name);

        match schema {
            Some(s) if is_deferrable(&s.name) => ToolResult::success(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                json!({
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.parameters,
                }),
            ),
            Some(_) => ToolResult::error(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                format!("'{name}' is not a deferred (MCP) tool — call it directly."),
            ),
            None => ToolResult::error(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                format!("No tool named '{name}' is registered."),
            ),
        }
    }
}

/// `tool_call` — execute a deferred tool through the normal registry
/// executor, so timeouts/guardrails/truncation behave identically to a
/// direct call.
pub struct ToolCallTool {
    registry: ToolRegistry,
}

impl ToolCallTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl OperantTool for ToolCallTool {
    fn name(&self) -> &str {
        TOOL_CALL_NAME
    }

    fn description(&self) -> &str {
        "Invoke a deferred (MCP) tool by name with JSON arguments."
    }

    fn schema(&self) -> ToolSchema {
        build_call_schema()
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let id = synthetic_call_id(TOOL_CALL_NAME);
        let name = match args.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                return ToolResult::error(
                    id,
                    "tool_call requires a non-empty 'name' argument".to_string(),
                );
            }
        };
        let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));

        if BRIDGE_TOOL_NAMES.contains(&name.as_str()) {
            return ToolResult::error(id, format!("'{name}' is a bridge tool — call it directly."));
        }
        if !is_deferrable(&name) {
            return ToolResult::error(
                id,
                format!("'{name}' is not a deferred tool — call it directly."),
            );
        }
        // Availability check mirrors a direct call: a deferred tool that is
        // disabled (by name or toolset) must not be invocable through the
        // bridge around the ban. `is_available` covers registration too.
        if !self.registry.is_available(&name).await {
            return ToolResult::error(
                id,
                format!("Tool '{name}' is not registered or is currently disabled."),
            );
        }

        // Execute through the registry: the executor stamps the inner
        // result's id/name and enforces the per-tool timeout. The inner
        // result is returned verbatim so the loop's normal tool-result
        // pipeline (truncation, persistence) applies to the real tool.
        match self.registry.execute(&name, &id, arguments, context).await {
            Ok(result) => result,
            Err(e) => ToolResult::error(id, e.to_string()),
        }
    }
}

#[cfg(test)]
struct DeferredDummy {
    name: &'static str,
    description: &'static str,
}

#[cfg(test)]
impl DeferredDummy {
    fn new(name: &'static str, description: &'static str) -> Self {
        Self { name, description }
    }
}

#[cfg(test)]
#[async_trait]
impl OperantTool for DeferredDummy {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name, self.description, json!({ "type": "object" }))
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(
            "dummy",
            json!({ "echoed_x": args.get("x").cloned().unwrap_or(Value::Null) }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ts(enabled: &str, listing: &str, listing_max_tokens: usize) -> ToolSearchSettings {
        ToolSearchSettings {
            enabled: enabled.to_string(),
            threshold_pct: 5.0,
            listing: listing.to_string(),
            listing_max_tokens,
            search_default_limit: 10,
            max_search_limit: 25,
        }
    }

    fn schema(name: &str, description: &str) -> ToolSchema {
        ToolSchema::new(name, description, json!({ "type": "object" }))
    }

    #[test]
    fn tier0_passthrough_when_no_deferrable() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("file_read", "Read a file"),
        ];
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(out.visible.len(), 2);
        assert!(out.deferred.is_empty());
        assert!(
            out.visible
                .iter()
                .all(|s| !BRIDGE_TOOL_NAMES.contains(&s.name.as_str()))
        );
    }

    #[test]
    fn mcp_tools_defer_behind_bridge() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("mcp_server_alpha", "Do alpha things"),
            schema("mcp_server_beta", "Do beta things"),
        ];
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(out.deferred.len(), 2);
        let names: Vec<&str> = out.visible.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"terminal"));
        assert!(names.contains(&TOOL_SEARCH_NAME));
        assert!(names.contains(&TOOL_DESCRIBE_NAME));
        assert!(names.contains(&TOOL_CALL_NAME));
        assert!(!names.contains(&"mcp_server_alpha"));
        assert!(!names.contains(&"mcp_server_beta"));
    }

    #[test]
    fn off_disables_bridge_even_with_mcp_tools() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("mcp_server_alpha", "Do alpha things"),
        ];
        let out = assemble_tools(all, &ts("off", "auto", 4000), 128_000);
        assert!(out.deferred.is_empty());
        assert_eq!(out.visible.len(), 2); // both eager, no bridge
    }

    #[test]
    fn listing_degrades_to_names_when_full_over_budget() {
        let deferred = vec![
            schema("mcp_s_alpha", "Alpha tool with a long description "),
            schema("mcp_s_beta", "Beta tool with a long description "),
        ];
        // Tiny budget: full listing won't fit, names-only will.
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 15), 128_000);
        assert_eq!(mode, ListingMode::Names);
    }

    #[test]
    fn listing_degrades_to_bare_when_names_over_budget() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha"), schema("mcp_s_beta", "Beta")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 5), 128_000);
        assert_eq!(mode, ListingMode::Bare);
    }

    #[test]
    fn listing_off_always_bare() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "off", 4000), 128_000);
        assert_eq!(mode, ListingMode::Bare);
    }

    #[test]
    fn server_summary_groups_by_server_prefix() {
        let deferred = vec![
            schema("mcp_alpha_tool_one", "1"),
            schema("mcp_alpha_tool_two", "2"),
            schema("mcp_beta_tool_three", "3"),
        ];
        let summary = server_summary(&deferred);
        assert!(summary.contains("alpha: 2 tool(s)"), "{summary}");
        assert!(summary.contains("beta: 1 tool(s)"), "{summary}");
    }

    #[tokio::test]
    async fn tool_search_queries_registry_statelessly() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(ToolSearchTool::new(registry.clone()))
            .await
            .unwrap();
        // Register a deferred tool directly into the shared map.
        registry
            .register(DeferredDummy::new(
                "mcp_srv_alpha",
                "Handles alpha processing",
            ))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new(
                "mcp_srv_beta",
                "Handles beta processing",
            ))
            .await
            .unwrap();

        let tool = ToolSearchTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "query": "alpha", "limit": 10 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let matches = parsed["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "mcp_srv_alpha");
    }

    #[tokio::test]
    async fn tool_describe_returns_full_schema() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let tool = ToolDescribeTool::new(registry.clone());
        let result = tool
            .execute(json!({ "name": "mcp_srv_alpha" }), ToolContext::default())
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["name"], "mcp_srv_alpha");
        assert!(parsed["parameters"].is_object());
    }

    #[tokio::test]
    async fn tool_call_executes_underlying_deferred_tool() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "mcp_srv_alpha", "arguments": { "x": 1 } }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["echoed_x"], 1);
        // The inner result name is stamped by the executor to the inner tool.
        assert_eq!(result.name, "mcp_srv_alpha");
    }

    #[tokio::test]
    async fn tool_call_rejects_non_deferred_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("terminal", "Native"))
            .await
            .unwrap();
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "terminal", "arguments": {} }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let message = result.error.as_deref().unwrap_or(&result.content);
        assert!(message.contains("not a deferred tool"), "{message}");
    }

    #[tokio::test]
    async fn tool_call_cannot_bypass_disabled_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        registry.disable_tool("mcp_srv_alpha").await;
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "mcp_srv_alpha", "arguments": {} }),
                ToolContext::default(),
            )
            .await;
        assert!(
            !result.success,
            "disabled tool must not be invocable via bridge"
        );
        let message = result.error.as_deref().unwrap_or(&result.content);
        assert!(message.contains("disabled"), "{message}");
    }

    #[tokio::test]
    async fn get_schemas_for_request_hides_bridge_in_tier0_and_shows_when_active() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("terminal", "Native"))
            .await
            .unwrap();
        registry
            .register(ToolSearchTool::new(registry.clone()))
            .await
            .unwrap();
        registry
            .register(ToolDescribeTool::new(registry.clone()))
            .await
            .unwrap();
        registry
            .register(ToolCallTool::new(registry.clone()))
            .await
            .unwrap();

        // Tier 0: no MCP tools → pure passthrough, bridge hidden.
        let visible = registry
            .get_schemas_for_request(&ts("auto", "auto", 4000), 128_000)
            .await;
        assert!(visible.iter().all(|s| s.name != "tool_search"));
        assert!(visible.iter().any(|s| s.name == "terminal"));

        // Active: MCP tools present → bridge shown, mcp_* hidden.
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let visible = registry
            .get_schemas_for_request(&ts("auto", "auto", 4000), 128_000)
            .await;
        let names: Vec<&str> = visible.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"tool_search"));
        assert!(names.contains(&"tool_describe"));
        assert!(names.contains(&"tool_call"));
        assert!(names.contains(&"terminal"));
        assert!(!names.contains(&"mcp_srv_alpha"));
    }

    #[test]
    fn listing_budget_with_zero_context_falls_back_to_cap() {
        let settings = ts("auto", "auto", 100);
        assert_eq!(listing_budget(&settings, 0), 100);
    }

    #[test]
    fn full_listing_fits_within_budget() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(mode, ListingMode::Full);
        let text = build_listing(&deferred, &ts("auto", "auto", 4000), 128_000);
        assert!(text.contains("mcp_s_alpha"), "{text}");
        assert!(text.contains("Alpha"), "{text}");
    }
}
